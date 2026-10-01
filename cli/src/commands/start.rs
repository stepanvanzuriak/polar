use crate::{CliError, Ctx, node, project};
use std::{
  path::{Path, PathBuf},
  process::Command,
};

pub(crate) fn start(
  ctx: &mut Ctx<'_, '_>,
  path: Option<&Path>,
  host: Option<&str>,
  args: &[String],
) -> Result<u8, CliError> {
  let dir = path.unwrap_or(Path::new(""));
  let Some(project) = project::load(dir, &ctx.io.cwd)? else {
    return Err(CliError::Message(format!(
      "`polar start` runs a built project: run it inside a project or name its \
       directory (one with a `{}`)",
      project::CONFIG
    )));
  };

  if project.library {
    return Err(project::library(&project.name));
  }

  let script = entry(&ctx.io.cwd.join(&project.out), &project, host)?;
  let node = node::locate(ctx.io.env).ok_or_else(|| {
    CliError::Message(
      "`polar start` needs Node.js (set POLAR_NODE or add `node` to PATH)"
        .to_string(),
    )
  })?;

  crate::watch::on_ctrl_c(|| {});

  let status = Command::new(&node)
    .arg(&script)
    .args(args)
    .current_dir(&ctx.io.cwd)
    .status()
    .map_err(|e| {
      CliError::Message(format!("cannot run {}: {e}", node.display()))
    })?;

  Ok(status.code().map_or(1, |code| u8::try_from(code).unwrap_or(1)))
}

fn entry(
  out: &Path,
  project: &project::Project,
  host: Option<&str>,
) -> Result<PathBuf, CliError> {
  const START: &str = "start.mjs";

  let shown = project.out.display();

  if let Some(host) = host {
    let script = out.join(host).join(START);

    return if script.is_file() {
      Ok(script)
    } else if out.join(START).is_file() {
      Ok(out.join(START))
    } else {
      Err(CliError::Message(format!(
        "`{}` has no build for `{host}` in `{shown}`: run `polar build` first",
        project.name
      )))
    };
  }

  if out.join(START).is_file() {
    return Ok(out.join(START));
  }

  let built: Vec<&String> = project
    .hosts
    .iter()
    .filter(|h| out.join(h.as_str()).join(START).is_file())
    .collect();

  match built.as_slice() {
    [one] => Ok(out.join(one.as_str()).join(START)),
    [] => Err(CliError::Message(format!(
      "`{}` has no build in `{shown}`: run `polar build` first",
      project.name
    ))),
    many => {
      let names: Vec<String> = many.iter().map(|h| format!("`{h}`")).collect();

      Err(CliError::Message(format!(
        "`{}` is built for hosts {}; choose one with `--host`",
        project.name,
        names.join(", ")
      )))
    }
  }
}
