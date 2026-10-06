use super::{Compiled, Plan, compile_imports, compile_one, render};
use crate::{
  CliError, Ctx,
  files::{
    discover, normalize, relative_path, rewrite_map, runtime_specifier,
    write_atomic,
  },
  guard, plural,
  project::{Start, Target},
  runtime::{LAUNCHER_JS, RUNTIME_JS, START_JS},
};
use polar_compiler::{
  CompileOptions, Stage, shared::diagnostic::Severity, stdlib,
};
use std::{
  path::{Path, PathBuf},
  time::Instant,
};

pub(crate) struct Written {
  pub host: Option<String>,
  pub dir: PathBuf,
}

pub(crate) fn build(
  ctx: &mut Ctx<'_, '_>,
  targets: &[Target],
  host: Option<&str>,
) -> Result<u8, CliError> {
  Ok(u8::from(build_into(ctx, targets, host, true)?.is_none()))
}

pub(crate) fn build_into(
  ctx: &mut Ctx<'_, '_>,
  targets: &[Target],
  host: Option<&str>,
  announce: bool,
) -> Result<Option<Vec<Written>>, CliError> {
  let start = Instant::now();
  let mut built = Vec::new();
  let mut written = Vec::new();

  for target in targets {
    if target.library {
      return Err(crate::project::library(
        target.project.as_deref().unwrap_or_default(),
      ));
    }

    let inputs = discover(&target.paths, &ctx.io.cwd, Some(&target.out))?;
    let project = target.project.as_ref().map(|_| target.hosts.as_slice());
    let hosts = Plan::hosts(host, project, &inputs)?;
    let several = hosts.len() > 1;

    for variant in hosts {
      let out = match (&variant, several) {
        (Some(name), true) => target.out.join(name),
        _ => target.out.clone(),
      };
      let out_dir = normalize(&ctx.io.cwd.join(&out));
      let plan = Plan {
        host: polar_compiler::HostOption::Fixed(variant.clone()),
        out_dir: Some(out_dir),
      };
      let mut compiled = inputs
        .iter()
        .map(|input| {
          compile_one(ctx, input, runtime_specifier(&input.output), &plan)
        })
        .collect::<Result<Vec<_>, _>>()?;

      compile_imports(
        ctx.compiler,
        &mut compiled,
        |input| runtime_specifier(&input.output),
        &plan,
      )?;

      written.push(Written {
        host: variant.clone(),
        dir: normalize(&ctx.io.cwd.join(&out)),
      });
      built.push((target, variant.filter(|_| several), out, compiled));
    }
  }

  let all: Vec<&Compiled> = built.iter().flat_map(|(.., c)| c).collect();
  let files: Vec<_> =
    all.iter().map(|c| (&c.file, c.diagnostics.as_slice())).collect();

  render(ctx.io.err, ctx.color, &files);

  if all.iter().any(|c| c.has_errors()) {
    return Ok(None);
  }

  for (target, variant, out, compiled) in &built {
    let out_dir = normalize(&ctx.io.cwd.join(out));

    write_runtime(&out_dir)?;
    write_std(ctx.compiler, &out_dir, compiled.iter().flat_map(std_imports))?;

    for c in compiled {
      write_output(&out_dir, c)?;
      write_externs(&out_dir, c)?;
    }

    write_bridges(&out_dir, compiled)?;

    if !announce {
      continue;
    }

    let files = plural(compiled.len(), "file");
    let what = match (&target.project, variant) {
      (Some(name), Some(host)) => {
        format!("{name} ({files}) for {host} into {}", out.display())
      }
      (Some(name), None) => format!("{name} ({files}) into {}", out.display()),
      (None, Some(host)) => {
        format!("{files} for {host} into {}", out.display())
      }
      (None, None) => files,
    };

    let _ =
      writeln!(ctx.io.err, "built {what} in {}ms", start.elapsed().as_millis());
  }

  for target in targets {
    let Some(start) = &target.start else { continue };
    let dirs: Vec<(Option<String>, PathBuf)> = written
      .iter()
      .zip(&built)
      .filter(|(_, (t, ..))| std::ptr::eq(*t, target))
      .map(|(w, _)| (w.host.clone(), w.dir.clone()))
      .collect();

    write_start(ctx, target, start, &dirs)?;
  }

  Ok(Some(written))
}

fn write_start(
  ctx: &Ctx<'_, '_>,
  target: &Target,
  start: &Start,
  dirs: &[(Option<String>, PathBuf)],
) -> Result<(), CliError> {
  let top = normalize(&ctx.io.cwd.join(&target.out));
  let write = |path: PathBuf, text: &[u8]| {
    write_atomic(&path, text)
      .map_err(|e| CliError::write(path.display().to_string(), &e))
  };

  if let Some(launch) = &start.launch {
    let name = launch.script.file_name().map_or_else(
      || "launcher.mjs".to_string(),
      |n| n.to_string_lossy().into_owned(),
    );
    let script = std::fs::read(&launch.script)
      .map_err(|e| CliError::read(launch.script.display().to_string(), &e))?;
    let hosts: serde_json::Map<String, serde_json::Value> = dirs
      .iter()
      .filter_map(|(host, dir)| {
        Some((host.clone()?, relative_path(&top, dir).into()))
      })
      .collect();
    let config = serde_json::json!({
      "launcher": format!("_polar/launcher/{name}"),
      "manifest": {
        "version": 1,
        "project": target.project,
        "root": relative_path(&top, &ctx.io.cwd.join(&start.dir)),
        "main": start.main,
        "hosts": hosts,
        "options": launch.options,
      },
    });

    write(top.join("_polar").join("launcher").join(&name), &script)?;
    write(
      top.join("_polar").join("start.json"),
      config.to_string().as_bytes(),
    )?;
    return write(top.join("start.mjs"), START_JS.as_bytes());
  }

  if !ctx.io.cwd.join(&start.dir).join(&start.source).is_file() {
    return Ok(());
  }

  let config = serde_json::json!({
    "launcher": "_polar/launcher.mjs",
    "main": start.main,
    "source": start.source,
  });

  for (_, dir) in dirs {
    write(dir.join("_polar").join("launcher.mjs"), LAUNCHER_JS.as_bytes())?;
    write(
      dir.join("_polar").join("start.json"),
      config.to_string().as_bytes(),
    )?;
    write(dir.join("start.mjs"), START_JS.as_bytes())?;
  }

  Ok(())
}

pub(crate) fn write_externs(
  out_dir: &Path,
  c: &Compiled,
) -> Result<Vec<PathBuf>, CliError> {
  let mut written = Vec::new();

  for (source, relative) in c.output.iter().flat_map(|o| &o.extern_files) {
    let bytes =
      std::fs::read(source).map_err(|e| CliError::read(source, &e))?;
    let target = out_dir.join(c.input.out_root()).join(relative);

    write_atomic(&target, &bytes)
      .map_err(|e| CliError::write(target.display().to_string(), &e))?;
    written.push(target);
  }

  Ok(written)
}

pub(crate) fn write_bridges(
  out_dir: &Path,
  compiled: &[Compiled],
) -> Result<Option<PathBuf>, CliError> {
  let polar = out_dir.join("_polar");
  let mut imports = String::new();
  let mut entries: Vec<String> = Vec::new();

  for c in compiled {
    let Some(output) = &c.output else { continue };
    let js = out_dir.join(c.input.output.with_extension("js"));
    let specifier = relative_path(&polar, &js);

    for effect in &output.bridges {
      if entries.iter().any(|e| e.starts_with(&format!("{effect}:"))) {
        continue;
      }

      let local = format!("${}", entries.len());

      let _ = std::fmt::Write::write_fmt(
        &mut imports,
        format_args!(
          "import {{ $bridge${effect} as {local} }} from \"{specifier}\";\n"
        ),
      );
      entries.push(format!("{effect}: {local}"));
    }
  }

  if entries.is_empty() {
    return Ok(None);
  }

  let js = format!(
    "{imports}\nexport const bridges = {{ {} }};\n",
    entries.join(", ")
  );
  let dts = "export declare const bridges: Record<string, unknown>;\n";
  let path = polar.join("bridges.js");
  let dts_path = polar.join("bridges.d.ts");

  write_atomic(&path, js.as_bytes())
    .map_err(|e| CliError::write(path.display().to_string(), &e))?;
  write_atomic(&dts_path, dts.as_bytes())
    .map_err(|e| CliError::write(dts_path.display().to_string(), &e))?;

  Ok(Some(path))
}

pub(crate) fn write_runtime(out_dir: &Path) -> Result<(), CliError> {
  let path = out_dir.join("_polar").join("runtime.js");

  write_atomic(&path, RUNTIME_JS.as_bytes())
    .map_err(|e| CliError::write(path.display().to_string(), &e))
}

pub(crate) fn std_imports(c: &Compiled) -> Vec<String> {
  c.output.as_ref().map(|o| o.std_imports.clone()).unwrap_or_default()
}

pub(crate) fn write_std(
  compiler: crate::Compiler,
  out_dir: &Path,
  names: impl IntoIterator<Item = String>,
) -> Result<Vec<PathBuf>, CliError> {
  let mut todo: Vec<String> = names.into_iter().collect();
  let mut done: Vec<String> = Vec::new();
  let mut written = Vec::new();

  while let Some(name) = todo.pop() {
    if done.contains(&name) {
      continue;
    }

    let filename = stdlib::filename(&name);
    let ice = |message: String| {
      CliError::Ice(crate::Ice {
        message,
        file: filename.clone(),
        backtrace: None,
      })
    };
    let std = stdlib::module(&name)
      .ok_or_else(|| ice(format!("no std module `{name}`")))?;
    let options = stdlib::compile_options(&name, "../runtime.js");
    let output =
      guard(&filename, || (compiler.compile)(std.source, &filename, &options))
        .map_err(CliError::Ice)?;

    if let Some(error) =
      output.diagnostics.iter().find(|d| d.severity == Severity::Error)
    {
      return Err(ice(format!(
        "the std module `Std.{name}` does not compile: {}",
        error.message
      )));
    }

    let dir = out_dir.join("_polar").join("std");

    let own = std.files.iter().map(|&(file, text)| (dir.join(file), text));

    for (path, text) in [
      (dir.join(format!("{name}.js")), output.js.as_str()),
      (dir.join(format!("{name}.d.ts")), output.dts.as_str()),
    ]
    .into_iter()
    .chain(own)
    {
      write_atomic(&path, text.as_bytes())
        .map_err(|e| CliError::write(path.display().to_string(), &e))?;
      written.push(path);
    }

    todo.extend(output.std_imports);
    done.push(name);
  }

  Ok(written)
}

pub(crate) fn write_output(
  out_dir: &Path,
  c: &Compiled,
) -> Result<[PathBuf; 3], CliError> {
  let Some(output) = &c.output else {
    return Err(CliError::Message(format!(
      "`{}` was not compiled",
      c.input.display
    )));
  };

  let js_path = out_dir.join(c.input.output.with_extension("js"));
  let map_path = out_dir.join(c.input.output.with_extension("js.map"));
  let dts_path = out_dir.join(c.input.output.with_extension("d.ts"));
  let js_name = file_name(&js_path);
  let map_name = file_name(&map_path);
  let map_dir = map_path.parent().unwrap_or(out_dir);

  let js = format!(
    "{}\n//# sourceMappingURL={map_name}\n",
    output.js.trim_end_matches('\n')
  );
  let map = rewrite_map(
    &output.sourcemap,
    &js_name,
    &relative_path(map_dir, &c.input.path),
  );

  for (path, text) in
    [(&js_path, js), (&map_path, map), (&dts_path, output.dts.clone())]
  {
    write_atomic(path, text.as_bytes())
      .map_err(|e| CliError::write(path.display().to_string(), &e))?;
  }

  Ok([js_path, map_path, dts_path])
}

fn file_name(path: &Path) -> String {
  path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

pub(crate) fn emit(
  ctx: &mut Ctx<'_, '_>,
  paths: &[PathBuf],
  stage: Stage,
  host: Option<&str>,
) -> Result<u8, CliError> {
  let inputs = discover(paths, &ctx.io.cwd, None)?;

  let [input] = inputs.as_slice() else {
    return Err(CliError::Message(format!(
      "`--emit` needs exactly one input file, but {} were given",
      inputs.len()
    )));
  };

  let source = crate::files::read_source(input)?;
  let file = polar_compiler::shared::source::SourceFile::new(
    input.display.clone(),
    source.text.clone(),
  );

  if let Some(invalid) = source.invalid {
    render(ctx.io.err, ctx.color, &[(&file, &[invalid])]);
    return Ok(1);
  }

  let (modules, _) = super::read_imports(input, &source.text)?;
  let plan = match stage {
    Stage::Js => Plan::for_program(host, inputs.as_slice(), None)?,
    _ => Plan::checking(),
  };
  let options = CompileOptions {
    modules,
    host: plan.host,
    plugins: input.plugins(),
    ..CompileOptions::default()
  };
  let dump = ctx.compiler.dump_stage;
  let result = guard(&input.display, || {
    dump(&source.text, &input.display, stage, &options)
  })
  .map_err(CliError::Ice)?;

  render(ctx.io.err, ctx.color, &[(&file, &result.diagnostics)]);

  match result.output {
    Some(output) => {
      let _ = write!(ctx.io.out, "{output}");

      if !output.ends_with('\n') {
        let _ = writeln!(ctx.io.out);
      }

      Ok(u8::from(super::has_errors(&result.diagnostics)))
    }
    None => Ok(1),
  }
}
