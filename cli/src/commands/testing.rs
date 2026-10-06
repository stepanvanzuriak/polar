use super::build_into;
use crate::{
  CliError, Ctx,
  project::{self, Target},
  runtime::TEST_RUNNER_JS,
};
use std::{fs, path::Path, process::Command};

pub(crate) fn test(
  ctx: &mut Ctx<'_, '_>,
  path: Option<&Path>,
  args: &[String],
) -> Result<u8, CliError> {
  let cwd = ctx.io.cwd.clone();
  let Some(project) = project::load(path.unwrap_or(Path::new("")), &cwd)?
  else {
    return Err(CliError::Message(
      "`polar test` runs inside a project (a folder with polar.toml)"
        .to_string(),
    ));
  };

  if project.library {
    return Err(project::library(&project.name));
  }

  let node = super::run::needs_node(ctx)?;
  let dir = tempfile::Builder::new()
    .prefix("polar-test-")
    .tempdir()
    .map_err(|e| CliError::write("a temporary directory", &e))?;
  let target = Target {
    paths: vec![project.src.clone()],
    out: dir.path().join("dist"),
    project: Some(project.name.clone()),
    hosts: project.hosts.clone(),
    library: false,
    start: None,
  };
  let host = (!project.hosts.is_empty()).then_some("Node");
  let Some(written) = build_into(ctx, &[target], host, false)? else {
    return Ok(1);
  };
  let Some(built) = written.first() else {
    return Ok(1);
  };
  let runner = dir.path().join("test_runner.mjs");

  fs::write(&runner, TEST_RUNNER_JS)
    .map_err(|e| CliError::write(runner.display().to_string(), &e))?;

  crate::watch::on_ctrl_c(|| {});

  let status = Command::new(&node)
    .arg("--enable-source-maps")
    .arg("--test-reporter=spec")
    .args(args)
    .arg(&runner)
    .arg(&built.dir)
    .current_dir(&ctx.io.cwd)
    .status()
    .map_err(|e| {
      CliError::Message(format!("cannot run {}: {e}", node.display()))
    })?;

  Ok(status.code().map_or(1, |code| u8::try_from(code).unwrap_or(1)))
}
