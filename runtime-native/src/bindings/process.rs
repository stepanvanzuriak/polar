use crate::{
  builtins::{arg, number, text},
  interp::{Control, Interp, R},
  value::Value,
};
use std::{
  io::Write,
  process::{Command, Stdio},
};

pub fn args(it: &mut Interp, _: &[Value]) -> R {
  Ok(Value::list(it.args.iter().map(Value::str).collect()))
}

pub fn env(it: &mut Interp, a: &[Value]) -> R {
  Ok(
    it.env_var(text(a, 0)?)
      .map_or_else(Value::none, |v| Value::some(Value::str(v))),
  )
}

pub fn cwd(_: &mut Interp, _: &[Value]) -> R {
  Ok(Value::str(
    std::env::current_dir()
      .map_or_else(|_| String::new(), |p| p.to_string_lossy().into_owned()),
  ))
}

pub fn set_exit_code(it: &mut Interp, a: &[Value]) -> R {
  #[allow(clippy::cast_possible_truncation)]
  {
    it.exit_code = Some(number(a, 0)? as i32);
  }

  Ok(Value::unit())
}

fn reason(e: &std::io::Error) -> String {
  match e.kind() {
    std::io::ErrorKind::NotFound => "command not found".to_string(),
    std::io::ErrorKind::PermissionDenied => "permission denied".to_string(),
    _ => e.to_string(),
  }
}

struct Spawned {
  code: i32,
  stdout: String,
  stderr: String,
}

fn start(
  it: &mut Interp,
  a: &[Value],
  capture: bool,
) -> Result<Spawned, Control> {
  let command = text(a, 0)?;
  let args: Vec<String> = arg(a, 1)?
    .list_items()
    .unwrap_or_default()
    .iter()
    .filter_map(|v| v.as_str().map(str::to_string))
    .collect();
  let options = arg(a, 2)?;
  let mut cmd = Command::new(command);

  cmd.args(&args);

  if let Some(Value::Variant(v)) = options.field("cwd")
    && let Some(Value::Str(dir)) = v.fields.first()
  {
    cmd.current_dir(&**dir);
  }

  for (k, v) in &it.env_overlay {
    match v {
      Some(v) => cmd.env(k, v),
      None => cmd.env_remove(k),
    };
  }

  for var in
    options.field("env").and_then(Value::list_items).unwrap_or_default()
  {
    if let (Some(name), Some(value)) = (
      var.field("name").and_then(Value::as_str),
      var.field("value").and_then(Value::as_str),
    ) {
      cmd.env(name, value);
    }
  }

  if capture {
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
  }

  let _ = it.out.flush();

  match cmd.output_or_status(capture) {
    Ok(done) => Ok(done),
    Err(e) => {
      let _ = writeln!(it.err, "error: cannot run `{command}`: {}", reason(&e));

      Ok(Spawned { code: 127, stdout: String::new(), stderr: String::new() })
    }
  }
}

trait Launch {
  fn output_or_status(&mut self, capture: bool) -> std::io::Result<Spawned>;
}

impl Launch for Command {
  fn output_or_status(&mut self, capture: bool) -> std::io::Result<Spawned> {
    let (status, stdout, stderr) = if capture {
      let out = self.output()?;

      (
        out.status,
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
      )
    } else {
      (self.status()?, String::new(), String::new())
    };

    #[cfg(unix)]
    let signal = {
      use std::os::unix::process::ExitStatusExt;

      status.signal().map(|s| 128 + s)
    };
    #[cfg(not(unix))]
    let signal: Option<i32> = None;

    Ok(Spawned { code: status.code().or(signal).unwrap_or(1), stdout, stderr })
  }
}

pub fn run(it: &mut Interp, a: &[Value]) -> R {
  Ok(Value::Num(f64::from(start(it, a, false)?.code)))
}

pub fn output(it: &mut Interp, a: &[Value]) -> R {
  let done = start(it, a, true)?;

  Ok(Value::record(vec![
    ("code", Value::Num(f64::from(done.code))),
    ("stdout", Value::str(done.stdout)),
    ("stderr", Value::str(done.stderr)),
  ]))
}

pub fn scoped_env(it: &mut Interp, a: &[Value]) -> R {
  let mark = it.env_overlay.len();

  for var in arg(a, 0)?.list_items().unwrap_or_default() {
    if let (Some(name), Some(value)) = (
      var.field("name").and_then(Value::as_str),
      var.field("value").and_then(Value::as_str),
    ) {
      it.env_overlay.push((name.to_string(), Some(value.to_string())));
    }
  }

  let result = it.apply(arg(a, 1)?, Vec::new());

  it.env_overlay.truncate(mark);

  result
}
