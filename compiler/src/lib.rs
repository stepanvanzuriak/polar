//! The compiler is one pipeline; each step lives in `pipeline.rs`.
//!
//! ```text
//! syntax/   source ── lex ──▶ tokens ── parse ──▶ AST ── expand ──▶ AST
//!                  lexer/             parser/     ast/   derive.rs
//!
//! core/     AST ── lower ──▶ core IR ── check ──▶ types
//! check/           core/lower.rs       check/, types/
//!
//!           core IR + types ── elaborate ──▶ core IR
//!                              core/annotate.rs
//!                              core/dictionaries.rs
//!
//! backend/  core IR ── host ──▶ core IR for one host
//!                      codegen/hosts.rs
//!
//!           core IR ── emit ──▶ JS AST ── print ──▶ JS + .d.ts + source map
//!                      core/matching.rs  js/print.rs
//!                      codegen/emit.rs   js/sourcemap.rs, codegen/dts.rs
//! ```
//!
//! `shared/` holds what every step uses: sources, spans, diagnostics.
//! `fmt/` is the formatter; it reuses `syntax/` and stops there.
//! `stdlib.rs` runs the front of the pipeline over the standard library.

pub mod backend;
#[doc(hidden)]
pub mod check;
#[doc(hidden)]
pub mod core;
pub mod fmt;
mod pipeline;
pub mod shared;
pub mod stdlib;
pub mod syntax;
mod testing;
#[doc(hidden)]
pub mod types;

pub use fmt::{FormatResult, format, format_plugins};

use crate::{
  backend::{codegen, js},
  shared::{
    diagnostic::{Diagnostic, DiagnosticBag},
    source::SourceFile,
  },
  syntax::{ast, lexer, lexer::dump::dump_tokens, parser},
};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum HostOption {
  #[default]
  Auto,
  Flag(String),
  Fixed(Option<String>),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Target {
  #[default]
  Js,
  Native,
}

impl Target {
  #[must_use]
  pub fn lang(self) -> &'static str {
    match self {
      Self::Js => "js",
      Self::Native => "rs",
    }
  }

  #[must_use]
  pub fn label(self) -> &'static str {
    match self {
      Self::Js => "JavaScript",
      Self::Native => "native",
    }
  }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompileOptions {
  pub target: Target,
  pub runtime: String,
  pub modules: Vec<shared::modules::ModuleSource>,
  pub host: HostOption,
  pub source_dir: Option<String>,
  pub output_dir: Option<String>,
  pub source_root: Option<String>,
  pub plugins: Vec<String>,
}

impl Default for CompileOptions {
  fn default() -> Self {
    Self {
      target: Target::Js,
      runtime: "./_polar/runtime.js".to_string(),
      modules: Vec::new(),
      host: HostOption::Auto,
      source_dir: None,
      output_dir: None,
      source_root: None,
      plugins: Vec::new(),
    }
  }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CompileOutput {
  pub js: String,
  pub dts: String,
  pub sourcemap: String,
  pub diagnostics: Vec<Diagnostic>,
  pub std_imports: Vec<String>,
  pub host: Option<String>,
  pub host_error: Option<String>,
  pub extern_files: Vec<(String, String)>,
  pub bridges: Vec<String>,
  pub module: Option<String>,
  pub tests: Vec<TestFn>,
  pub each: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestFn {
  pub name: String,
  pub line: usize,
}

pub use codegen::hosts::choose_host;

#[must_use]
pub fn compile(
  source: &str,
  filename: &str,
  options: &CompileOptions,
) -> CompileOutput {
  syntax::plugins::within(&options.plugins, || {
    compile_within(source, filename, options)
  })
}

fn compile_within(
  source: &str,
  filename: &str,
  options: &CompileOptions,
) -> CompileOutput {
  let file = SourceFile::new(filename, source);
  let mut bag = DiagnosticBag::default();

  stdlib::reset_session();

  let Some(analysed) = pipeline::front_end(&file, &options.modules, &mut bag)
  else {
    return failed(bag);
  };

  let testing::Checked { tests, each } =
    testing::check(&analysed.module, &file, &mut bag);
  let module = analysed.module.name.as_ref().map(|n| n.text.clone());

  let host = match pipeline::host(options, &analysed.effects) {
    Ok(host) => host,
    Err(message) => {
      return CompileOutput {
        diagnostics: bag.into_sorted(),
        host_error: Some(message),
        ..CompileOutput::default()
      };
    }
  };

  let (core, extern_files) = codegen::hosts::for_host(
    analysed.core,
    &analysed.types,
    &analysed.effects,
    host.as_deref(),
    options,
    &mut bag,
  );
  let core = core::inline::inline(core, &options.modules);
  let dts = codegen::dts::emit(&core, &analysed.types, options);

  if bag.has_errors() {
    return failed(bag);
  }

  let std_imports = core.std_imports.clone();
  let mut bridges: Vec<String> = core
    .bridges
    .iter()
    .filter(|b| host.as_deref() == Some(b.via.as_str()))
    .map(|b| b.effect.clone())
    .collect();

  bridges.dedup();

  let printed = pipeline::emit(core, &file, options);
  let mut map = js::sourcemap::from_mappings(&printed.mappings, &file);

  if host.is_some() && analysed.effects.hosts.len() > 1 {
    map.sources_content.clear();
  }

  let sourcemap = map.to_json();

  CompileOutput {
    js: printed.code,
    dts,
    sourcemap,
    diagnostics: bag.into_sorted(),
    std_imports,
    host,
    extern_files,
    bridges,
    module,
    tests,
    each,
    ..CompileOutput::default()
  }
}

#[derive(Debug, Clone, Default)]
pub struct CoreOutput {
  pub module: Option<core::ir::CModule>,
  pub diagnostics: Vec<Diagnostic>,
  pub host: Option<String>,
  pub host_error: Option<String>,
}

#[must_use]
pub fn compile_core(
  source: &str,
  filename: &str,
  options: &CompileOptions,
) -> CoreOutput {
  syntax::plugins::within(&options.plugins, || {
    core_within(source, filename, options)
  })
}

fn core_within(
  source: &str,
  filename: &str,
  options: &CompileOptions,
) -> CoreOutput {
  let file = SourceFile::new(filename, source);
  let mut bag = DiagnosticBag::default();

  stdlib::reset_session();

  let Some(analysed) = pipeline::front_end(&file, &options.modules, &mut bag)
  else {
    return CoreOutput {
      diagnostics: bag.into_sorted(),
      ..CoreOutput::default()
    };
  };

  let host = match pipeline::host(options, &analysed.effects) {
    Ok(host) => host,
    Err(message) => {
      return CoreOutput {
        diagnostics: bag.into_sorted(),
        host_error: Some(message),
        ..CoreOutput::default()
      };
    }
  };

  let (core, _) = codegen::hosts::for_host(
    analysed.core,
    &analysed.types,
    &analysed.effects,
    host.as_deref(),
    options,
    &mut bag,
  );
  let core = core::inline::inline(core, &options.modules);
  let mut core = core::matching::compile_matches(core);

  core::serial::lower_typed_ops(&mut core);

  core.source = Some((filename.to_string(), source.to_string()));

  let module = (!bag.has_errors()).then_some(core);

  CoreOutput { module, diagnostics: bag.into_sorted(), host, host_error: None }
}

#[derive(Debug, Clone, Default)]
pub struct NativeOutput {
  pub bundle: Option<core::serial::Bundle>,
  pub diagnostics: Vec<Diagnostic>,
  pub host_error: Option<String>,
}

/// Compiles a program and everything it imports for the native runtime.
#[must_use]
pub fn compile_native(
  source: &str,
  filename: &str,
  options: &CompileOptions,
) -> NativeOutput {
  let options = CompileOptions { target: Target::Native, ..options.clone() };
  let main = compile_core(source, filename, &options);

  if main.host_error.is_some() {
    return NativeOutput {
      diagnostics: main.diagnostics,
      host_error: main.host_error,
      ..NativeOutput::default()
    };
  }

  let Some(main_module) = main.module else {
    return NativeOutput {
      diagnostics: main.diagnostics,
      ..NativeOutput::default()
    };
  };
  let mut diagnostics = main.diagnostics;
  let host = main.host.clone();
  let mut modules: Vec<(String, core::ir::CModule)> = Vec::new();
  let mut pending: Vec<String> = Vec::new();
  let want = |module: &core::ir::CModule, pending: &mut Vec<String>| {
    pending.extend(module.std_imports.iter().map(|n| format!("Std.{n}")));
    pending.extend(module.user_imports.iter().map(|(p, _)| p.clone()));
  };

  want(&main_module, &mut pending);

  while let Some(path) = pending.pop() {
    if modules.iter().any(|(p, _)| *p == path) {
      continue;
    }

    let out = if let Some(name) = path.strip_prefix("Std.") {
      let Some(std) = stdlib::module(name) else { continue };
      let std_options = CompileOptions {
        target: Target::Native,
        ..stdlib::compile_options(name, &options.runtime)
      };

      compile_core(std.source, &stdlib::filename(name), &std_options)
    } else {
      let Some(user) = options.modules.iter().find(|m| m.path == path) else {
        continue;
      };
      let user_options = CompileOptions {
        host: HostOption::Fixed(host.clone()),
        ..options.clone()
      };

      compile_core(&user.source, &user.filename(), &user_options)
    };

    diagnostics.extend(out.diagnostics);

    if let Some(message) = out.host_error {
      return NativeOutput {
        diagnostics,
        host_error: Some(format!("{path}: {message}")),
        ..NativeOutput::default()
      };
    }

    let Some(module) = out.module else {
      return NativeOutput { diagnostics, ..NativeOutput::default() };
    };

    want(&module, &mut pending);
    modules.push((path, module));
  }

  let missing = codegen::hosts::missing_native_imports(&main_module, &modules);

  if !missing.is_empty() {
    diagnostics.extend(missing);

    return NativeOutput { diagnostics, ..NativeOutput::default() };
  }

  NativeOutput {
    bundle: Some(core::serial::Bundle { main: main_module, modules }),
    diagnostics,
    host_error: None,
  }
}

fn failed(bag: DiagnosticBag) -> CompileOutput {
  CompileOutput { diagnostics: bag.into_sorted(), ..CompileOutput::default() }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
  Tokens,
  Ast,
  Core,
  Types,
  Hosts,
  Dictionaries,
  Js,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DumpResult {
  pub output: Option<String>,
  pub diagnostics: Vec<Diagnostic>,
}

#[must_use]
pub fn dump_stage(source: &str, filename: &str, stage: Stage) -> DumpResult {
  dump_stage_with(source, filename, stage, &CompileOptions::default())
}

#[must_use]
pub fn dump_stage_with(
  source: &str,
  filename: &str,
  stage: Stage,
  options: &CompileOptions,
) -> DumpResult {
  syntax::plugins::within(&options.plugins, || {
    dump_within(source, filename, stage, options)
  })
}

fn dump_within(
  source: &str,
  filename: &str,
  stage: Stage,
  options: &CompileOptions,
) -> DumpResult {
  let file = SourceFile::new(filename, source);
  let mut bag = DiagnosticBag::default();

  stdlib::reset_session();

  let output = match stage {
    Stage::Tokens => {
      let lexed = lexer::lex(&file, &mut bag);

      Some(dump_tokens(&file, &lexed.tokens))
    }
    Stage::Ast => {
      let lexed = lexer::lex(&file, &mut bag);
      let mut module = parser::parse(&file, &lexed, &mut bag);

      if !bag.has_errors() {
        let _ =
          pipeline::expand(&mut module, &file, &options.modules, &mut bag);
      }

      Some(ast::dump::dump_ast(&module))
    }
    _ => dump_analysed(&file, stage, options, &mut bag),
  };

  DumpResult { output, diagnostics: bag.into_sorted() }
}

fn dump_analysed(
  file: &SourceFile,
  stage: Stage,
  options: &CompileOptions,
  bag: &mut DiagnosticBag,
) -> Option<String> {
  let modules = &options.modules;
  let mut module = pipeline::parse(file, bag)?;
  let expansion = pipeline::expand(&mut module, file, modules, bag);

  if bag.has_errors() {
    expansion.remap(bag);
    return None;
  }

  let lowered = pipeline::lower(&module, modules, &expansion, bag);

  if stage == Stage::Core {
    expansion.remap(bag);

    return (!bag.has_errors()).then(|| core::dump::dump_module(&lowered.core));
  }

  let types = pipeline::check(&module, &lowered, bag);

  expansion.remap(bag);

  let types = types?;

  match stage {
    Stage::Types => return Some(check::dump::dump_types(&types)),
    Stage::Hosts => {
      return Some(check::dump::dump_hosts(&types, &lowered.core.binds));
    }
    _ => {}
  }

  let core = pipeline::elaborate(lowered.core, &types, modules, &expansion);

  if stage == Stage::Dictionaries {
    return Some(core::dump::dump_module(&core));
  }

  let host = pipeline::host(options, &lowered.effects).unwrap_or(None);
  let (core, _) = codegen::hosts::for_host(
    core,
    &types,
    &lowered.effects,
    host.as_deref(),
    options,
    bag,
  );
  let core = core::inline::inline(core, &options.modules);

  Some(pipeline::emit(core, file, options).code)
}
