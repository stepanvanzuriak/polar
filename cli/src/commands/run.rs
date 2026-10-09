use super::{Plan, build_into, compile_imports, compile_one, render_compiled};
use crate::{
  CliError, Ctx,
  files::{discover, rewrite_map, runtime_specifier},
  node,
  project::{Launch, Project, Target},
  runtime::{LAUNCHER_JS, RUNTIME_JS},
};
use std::{
  ffi::OsString,
  fs,
  path::{Path, PathBuf},
  process::Command,
};
use tempfile::TempDir;

pub(crate) struct Prepared {
  pub dir: TempDir,
  pub script_args: Vec<OsString>,
}

impl Prepared {
  pub fn command(&self, node: &Path, args: &[String], cwd: &Path) -> Command {
    let mut command = Command::new(node);

    command
      .arg("--enable-source-maps")
      .args(&self.script_args)
      .arg("--")
      .args(args)
      .current_dir(cwd);
    command
  }
}

pub(crate) fn run(
  ctx: &mut Ctx<'_, '_>,
  file: &Path,
  host: Option<&str>,
  configured: Option<&[String]>,
  args: &[String],
) -> Result<u8, CliError> {
  let node = needs_node(ctx)?;

  let Some(prepared) = prepare_file(ctx, file, host, configured)? else {
    return Ok(1);
  };

  crate::watch::on_ctrl_c(|| {});

  let status =
    prepared.command(&node, args, &ctx.io.cwd).status().map_err(|e| {
      CliError::Message(format!("cannot run {}: {e}", node.display()))
    })?;

  keep_if_debug(ctx, prepared);

  Ok(status.code().map_or(1, |code| u8::try_from(code).unwrap_or(1)))
}

pub(crate) fn keep_if_debug(ctx: &mut Ctx<'_, '_>, prepared: Prepared) {
  if ctx.debug {
    let kept = prepared.dir.keep();
    let _ = writeln!(ctx.io.err, "note: kept the build in {}", kept.display());
  }
}

fn prefer_node(
  configured: Option<&[String]>,
  inputs: &[crate::files::Input],
) -> Result<Option<&'static str>, CliError> {
  let hosts = match configured {
    Some(hosts) if !hosts.is_empty() => hosts.to_vec(),
    _ => super::program_hosts(inputs)?,
  };

  Ok((hosts.len() > 1 && hosts.iter().any(|h| h == "Node")).then_some("Node"))
}

pub(crate) fn prepare_file(
  ctx: &mut Ctx<'_, '_>,
  file: &Path,
  host: Option<&str>,
  configured: Option<&[String]>,
) -> Result<Option<Prepared>, CliError> {
  let inputs = discover(&[file.to_path_buf()], &ctx.io.cwd, None)?;
  let [input] = inputs.as_slice() else {
    return Err(CliError::Message(
      "`polar run` takes a single `.px` file".to_string(),
    ));
  };

  let host = match host {
    Some(host) => Some(host),
    None => prefer_node(configured, &inputs)?,
  };
  let plan = Plan {
    host: polar_compiler::HostOption::Fixed(Plan::one_host(
      host,
      configured,
      inputs.as_slice(),
    )?),
    out_dir: None,
  };
  let mut compiled =
    vec![compile_one(ctx, input, "./_polar/runtime.js".to_string(), &plan)?];

  compile_imports(
    ctx.compiler,
    &mut compiled,
    |input| runtime_specifier(&input.output),
    &plan,
  )?;
  render_compiled(ctx, &compiled);

  if compiled.iter().any(|c| c.has_errors() || c.output.is_none()) {
    return Ok(None);
  }

  let dir = tempfile::Builder::new()
    .prefix("polar-run-")
    .tempdir()
    .map_err(|e| CliError::write("a temporary directory", &e))?;

  let main_js = dir.path().join("main.js");
  let output_of = |c: &super::Compiled| {
    if c.input.path == input.path {
      PathBuf::from("main")
    } else {
      c.input.output.clone()
    }
  };

  if let Some(c) =
    compiled.iter().skip(1).find(|c| output_of(c) == Path::new("main"))
  {
    return Err(CliError::Message(format!(
      "`polar run` writes `{}` as `main.js`, so it cannot also run `{}`",
      input.display, c.input.display
    )));
  }

  let mut files: Vec<(PathBuf, String)> = vec![
    (dir.path().join("_polar").join("runtime.js"), RUNTIME_JS.to_string()),
    (dir.path().join("launcher.mjs"), LAUNCHER_JS.to_string()),
  ];
  let mut std_imports = Vec::new();

  for c in &compiled {
    let Some(output) = &c.output else { continue };
    let js = dir.path().join(output_of(c).with_extension("js"));
    let map = dir.path().join(output_of(c).with_extension("js.map"));
    let (js_name, map_name) = (file_name(&js), file_name(&map));

    files.push((
      js,
      format!("{}\n//# sourceMappingURL={map_name}\n", output.js.trim_end()),
    ));
    files.push((
      map,
      rewrite_map(&output.sourcemap, &js_name, &c.input.path.to_string_lossy()),
    ));
    std_imports.extend(output.std_imports.iter().cloned());
  }

  crate::commands::write_std(ctx.compiler, dir.path(), std_imports)?;

  for (path, text) in &files {
    let written = path
      .parent()
      .map_or(Ok(()), fs::create_dir_all)
      .and_then(|()| fs::write(path, text));

    written.map_err(|e| CliError::write(path.display().to_string(), &e))?;
  }

  let script_args = vec![
    dir.path().join("launcher.mjs").into_os_string(),
    main_js.into_os_string(),
    OsString::from(&input.display),
  ];

  Ok(Some(Prepared { dir, script_args }))
}

pub(crate) fn launch(
  ctx: &mut Ctx<'_, '_>,
  project: &Project,
  args: &[String],
) -> Result<u8, CliError> {
  let Some(launch) = &project.launch else {
    return run(ctx, &project.main, None, Some(&project.hosts), args);
  };
  let node = needs_node(ctx)?;
  let Some(prepared) = prepare_launch(ctx, project, launch)? else {
    return Ok(1);
  };

  crate::watch::on_ctrl_c(|| {});

  let status =
    prepared.command(&node, args, &ctx.io.cwd).status().map_err(|e| {
      CliError::Message(format!("cannot run {}: {e}", node.display()))
    })?;

  keep_if_debug(ctx, prepared);

  Ok(status.code().map_or(1, |code| u8::try_from(code).unwrap_or(1)))
}

pub(crate) fn prepare_launch(
  ctx: &mut Ctx<'_, '_>,
  project: &Project,
  launch: &Launch,
) -> Result<Option<Prepared>, CliError> {
  let dir = tempfile::Builder::new()
    .prefix("polar-run-")
    .tempdir()
    .map_err(|e| CliError::write("a temporary directory", &e))?;
  let out = dir.path().join("dist");
  let target = Target {
    paths: vec![project.src.clone()],
    out: out.clone(),
    project: Some(project.name.clone()),
    hosts: project.hosts.clone(),
    library: false,
    start: None,
  };
  let Some(written) = build_into(ctx, &[target], None, false, None)? else {
    return Ok(None);
  };
  let hosts: serde_json::Map<String, serde_json::Value> = written
    .iter()
    .filter_map(|w| {
      let host = w.host.clone()?;

      Some((host, w.dir.to_string_lossy().into_owned().into()))
    })
    .collect();
  let main = project
    .start()
    .map_or_else(|| PathBuf::from("main.js"), |start| start.main);
  let manifest = serde_json::json!({
    "version": 1,
    "project": project.name,
    "root": crate::files::normalize(&ctx.io.cwd.join(&project.dir)),
    "out": out,
    "main": main,
    "hosts": hosts,
    "options": launch.options,
  });
  let manifest_path = dir.path().join("manifest.json");

  fs::write(&manifest_path, manifest.to_string())
    .map_err(|e| CliError::write(manifest_path.display().to_string(), &e))?;

  let script_args = vec![
    launch.script.clone().into_os_string(),
    manifest_path.into_os_string(),
  ];

  Ok(Some(Prepared { dir, script_args }))
}

pub(crate) fn needs_node(ctx: &Ctx<'_, '_>) -> Result<PathBuf, CliError> {
  node::locate(ctx.io.env).ok_or_else(|| {
    CliError::Message(
      "Polar needs Node.js to run programs (set POLAR_NODE or add `node` to PATH)"
        .to_string(),
    )
  })
}

fn file_name(path: &Path) -> String {
  path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}
