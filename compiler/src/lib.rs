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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompileOptions {
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

  let tests = testing::check(&analysed.module, &file, &mut bag);
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
    ..CompileOutput::default()
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
