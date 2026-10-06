mod build;
mod check;
mod fmt;
mod init;
mod run;
mod start;

pub(crate) use build::{
  build_into, write_externs, write_output, write_runtime, write_std,
};

use crate::{
  CliError, Command, Ctx,
  files::{
    Input, decode, module_input, module_specifier, normalize, read_bytes,
    read_source,
  },
  guard,
  project::{self, Target, targets},
  watch::{self, WatchMode},
};
use polar_compiler::{
  CompileOptions, CompileOutput, HostOption, choose_host,
  shared::diagnostic::{Diagnostic, Severity},
  shared::modules::{self, ModuleSource},
  shared::render::{RenderOptions, render_diagnostics},
  shared::source::SourceFile,
};
use std::{collections::HashMap, io::Write, path::PathBuf, sync::Arc};

pub(crate) fn dispatch(
  command: Command,
  ctx: &mut Ctx<'_, '_>,
) -> Result<u8, CliError> {
  let cwd = ctx.io.cwd.clone();

  match command {
    Command::Build { paths, emit: Some(stage), host, .. } => {
      build::emit(ctx, &paths, stage.into(), host.as_deref())
    }
    Command::Build { paths, out, watch: true, emit: None, .. } => {
      let target = single(targets(&paths, &cwd, out.as_deref())?)?;

      watch::foreground(
        ctx,
        &target.paths,
        WatchMode::Build { out: target.out },
      )
    }
    Command::Build { paths, out, watch: false, emit: None, host } => {
      build::build(
        ctx,
        &targets(&paths, &cwd, out.as_deref())?,
        host.as_deref(),
      )
    }
    Command::Check { paths, out, watch: true } => {
      let target = single(targets(&paths, &cwd, out.as_deref())?)?;

      watch::foreground(
        ctx,
        &target.paths,
        WatchMode::Check { out: Some(target.out) },
      )
    }
    Command::Check { paths, out, watch: false } => {
      check::check(ctx, &targets(&paths, &cwd, out.as_deref())?)
    }
    Command::Run { file, host, args } => {
      match (project::run_target(file.as_deref(), &cwd)?, host) {
        (project::RunTarget::Launch(project), None) => {
          run::launch(ctx, &project, &args)
        }
        (project::RunTarget::Launch(project), Some(host)) => {
          run::run(ctx, &project.main, Some(&host), Some(&project.hosts), &args)
        }
        (project::RunTarget::File { path, hosts }, host) => {
          run::run(ctx, &path, host.as_deref(), hosts.as_deref(), &args)
        }
      }
    }
    Command::Fmt { paths, check } => {
      fmt::fmt(ctx, &targets(&paths, &cwd, None)?, check)
    }
    Command::Start { path, host, args } => {
      start::start(ctx, path.as_deref(), host.as_deref(), &args)
    }
    Command::Init { dir, name, zones } => {
      init::init(ctx, dir.as_deref(), name.as_deref(), &zones)
    }
  }
}

fn single(mut targets: Vec<Target>) -> Result<Target, CliError> {
  match targets.len() {
    1 => Ok(targets.remove(0)),
    _ => Err(CliError::Message(
      "`--watch` watches one project, or one set of paths, at a time"
        .to_string(),
    )),
  }
}

pub(crate) struct Compiled {
  pub input: Input,
  pub file: SourceFile,
  pub output: Option<CompileOutput>,
  pub diagnostics: Vec<Diagnostic>,
  pub imports: Vec<Input>,
}

impl Compiled {
  pub fn has_errors(&self) -> bool {
    has_errors(&self.diagnostics)
  }
}

pub(crate) fn has_errors(diagnostics: &[Diagnostic]) -> bool {
  diagnostics.iter().any(|d| d.severity == Severity::Error)
}

#[derive(Debug, Clone, Default)]
pub(crate) struct Plan {
  pub host: HostOption,
  pub out_dir: Option<PathBuf>,
}

impl Plan {
  pub(crate) fn checking() -> Self {
    Self { host: HostOption::Fixed(None), out_dir: None }
  }

  pub(crate) fn for_program(
    flag: Option<&str>,
    inputs: &[Input],
    out_dir: Option<PathBuf>,
  ) -> Result<Self, CliError> {
    let host = Self::one_host(flag, None, inputs)?;

    Ok(Self { host: HostOption::Fixed(host), out_dir })
  }

  pub(crate) fn one_host(
    flag: Option<&str>,
    configured: Option<&[String]>,
    inputs: &[Input],
  ) -> Result<Option<String>, CliError> {
    match Self::hosts(flag, configured, inputs)?.as_slice() {
      [] => Ok(None),
      [one] => Ok(one.clone()),
      many => {
        let names: Vec<String> =
          many.iter().flatten().map(|h| format!("`{h}`")).collect();

        Err(CliError::Message(format!(
          "this project builds for hosts {}; choose one with `--host`",
          names.join(", ")
        )))
      }
    }
  }

  pub(crate) fn hosts(
    flag: Option<&str>,
    project: Option<&[String]>,
    inputs: &[Input],
  ) -> Result<Vec<Option<String>>, CliError> {
    let declared = program_hosts(inputs)?;
    let configured = project.unwrap_or_default();

    if flag.is_none() && !configured.is_empty() {
      let mut hosts = Vec::new();

      for host in configured {
        if !declared.contains(host) {
          let names: Vec<String> =
            declared.iter().map(|h| format!("`{h}`")).collect();
          let known = if names.is_empty() {
            "no hosts".to_string()
          } else {
            names.join(", ")
          };

          return Err(CliError::Message(format!(
            "`polar.toml` lists host `{host}`, but this program declares {known}"
          )));
        }

        if !hosts.contains(&Some(host.clone())) {
          hosts.push(Some(host.clone()));
        }
      }

      return Ok(hosts);
    }

    match choose_host(flag, &declared) {
      Ok(host) => Ok(vec![host]),
      Err(message) if flag.is_none() && project.is_some() => {
        let listed: Vec<String> =
          declared.iter().map(|h| format!("\"{h}\"")).collect();

        Err(CliError::Message(format!(
          "{message}, or build them all by adding `hosts = [{}]` to \
           `[project]` in `polar.toml`",
          listed.join(", ")
        )))
      }
      Err(message) => Err(CliError::Message(message)),
    }
  }
}

pub(crate) fn program_hosts(inputs: &[Input]) -> Result<Vec<String>, CliError> {
  let mut hosts: Vec<String> = Vec::new();

  for input in inputs {
    let Ok(source) = read_source(input) else { continue };
    let (modules, _) = read_imports(input, &source.text)?;
    let texts =
      std::iter::once((input.display.clone(), source.text, input.plugins()))
        .chain(modules.into_iter().map(|m| (m.path, m.source, m.plugins)));

    for (name, text, plugins) in texts {
      let found = guard(&name, || modules::hosts(&text, &name, &plugins))
        .map_err(CliError::Ice)?;

      for host in found {
        if !hosts.contains(&host) {
          hosts.push(host);
        }
      }
    }
  }

  hosts.sort();
  Ok(hosts)
}

pub(crate) fn compile_one(
  ctx: &Ctx<'_, '_>,
  input: &Input,
  runtime: String,
  plan: &Plan,
) -> Result<Compiled, CliError> {
  compile_input(ctx.compiler, input, runtime, plan)
}

pub(crate) fn compile_input(
  compiler: crate::Compiler,
  input: &Input,
  runtime: String,
  plan: &Plan,
) -> Result<Compiled, CliError> {
  let source = read_source(input)?;
  let file = SourceFile::new(input.display.clone(), source.text.clone());

  if let Some(invalid) = source.invalid {
    return Ok(Compiled {
      input: input.clone(),
      file,
      output: None,
      diagnostics: vec![invalid],
      imports: Vec::new(),
    });
  }

  let (modules, mut imports) = read_imports(input, &source.text)?;

  for sibling in crate::files::siblings(input, &source.text)? {
    if !imports.iter().any(|i| i.path == sibling.path) {
      imports.push(sibling);
    }
  }

  let dir_of = |path: &std::path::Path| {
    path.parent().map(|p| p.to_string_lossy().into_owned())
  };
  let output_dir = plan
    .out_dir
    .as_ref()
    .and_then(|out| dir_of(&normalize(&out.join(&input.output))));
  let source_root =
    input.source_root().map(|root| root.to_string_lossy().into_owned());
  let options = CompileOptions {
    runtime,
    modules,
    host: plan.host.clone(),
    source_dir: dir_of(&input.path),
    output_dir,
    source_root,
    plugins: input.plugins(),
  };
  let output = guard(&input.display, || {
    (compiler.compile)(&source.text, &input.display, &options)
  })
  .map_err(CliError::Ice)?;

  if let Some(message) = &output.host_error {
    return Err(CliError::Message(message.clone()));
  }

  let mut diagnostics = output.diagnostics.clone();

  if let Some(origin) = generated_by(input, &source.text) {
    for d in &mut diagnostics {
      d.notes.push(format!("`{}` has no file. {origin}", input.display));
    }
  }

  Ok(Compiled {
    input: input.clone(),
    file,
    output: Some(output),
    diagnostics,
    imports,
  })
}

/// For a module a plugin zone generates (see `files::siblings`), the line
/// that says which zone.
fn generated_by(input: &Input, text: &str) -> Option<String> {
  if input.path.exists() {
    return None;
  }

  text
    .lines()
    .next()
    .and_then(|line| line.strip_prefix("// "))
    .filter(|line| line.starts_with("Generated by the "))
    .map(str::to_string)
}

pub(crate) fn read_imports(
  input: &Input,
  source: &str,
) -> Result<(Vec<ModuleSource>, Vec<Input>), CliError> {
  let plugins = input.plugins();
  let first = guard(&input.display, || {
    modules::imports(source, &input.display, &plugins)
  })
  .map_err(CliError::Ice)?;
  let mut todo: std::collections::VecDeque<(String, Input)> =
    first.into_iter().map(|path| (path, input.clone())).collect();
  let mut seen: Vec<String> = Vec::new();
  let mut sources = Vec::new();
  let mut inputs = Vec::new();

  while let Some((path, from)) = todo.pop_front() {
    if seen.contains(&path) {
      continue;
    }

    seen.push(path.clone());

    let module = module_input(&from, &path);
    let Ok(bytes) = read_bytes(&module) else { continue };
    let text = decode(&module.display, &bytes).text;
    let plugins = module.plugins();
    let nested = guard(&module.display, || {
      modules::imports(&text, &module.display, &plugins)
    })
    .map_err(CliError::Ice)?;

    todo.extend(nested.into_iter().map(|p| (p, module.clone())));
    sources.push(ModuleSource {
      path,
      source: text,
      specifier: module_specifier(&input.output, &module.output),
      plugins,
    });
    inputs.push(module);
  }

  Ok((sources, inputs))
}

pub(crate) fn compile_imports(
  compiler: crate::Compiler,
  all: &mut Vec<Compiled>,
  runtime: impl Fn(&Input) -> String,
  plan: &Plan,
) -> Result<(), CliError> {
  let mut next = 0;

  while next < all.len() {
    let imports = all[next].imports.clone();

    for import in imports {
      if !all.iter().any(|c| c.input.path == import.path) {
        let runtime = runtime(&import);

        all.push(compile_input(compiler, &import, runtime, plan)?);
      }
    }

    next += 1;
  }

  Ok(())
}

pub(crate) fn render(
  err: &mut dyn Write,
  color: bool,
  files: &[(&SourceFile, &[Diagnostic])],
) {
  let map: HashMap<Arc<str>, SourceFile> = files
    .iter()
    .map(|(file, _)| (Arc::from(file.name()), (*file).clone()))
    .collect();
  let all: Vec<Diagnostic> =
    files.iter().flat_map(|(_, ds)| ds.iter().cloned()).collect();

  if !all.is_empty() {
    let _ = write!(
      err,
      "{}",
      render_diagnostics(&all, &map, RenderOptions { color })
    );
  }
}

pub(crate) fn render_compiled(ctx: &mut Ctx<'_, '_>, compiled: &[Compiled]) {
  let files: Vec<_> =
    compiled.iter().map(|c| (&c.file, c.diagnostics.as_slice())).collect();

  render(ctx.io.err, ctx.color, &files);
}
