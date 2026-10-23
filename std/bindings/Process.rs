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
  Ok(Value::maybe_str(it.env_var(text(a, 0)?)))
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

fn env_pairs(vars: &Value) -> Vec<(String, String)> {
  vars
    .list_items()
    .unwrap_or_default()
    .iter()
    .filter_map(|var| {
      Some((
        var.field("name")?.as_str()?.to_string(),
        var.field("value")?.as_str()?.to_string(),
      ))
    })
    .collect()
}

fn feed(child: &mut std::process::Child, input: Option<String>) {
  if let Some(input) = input
    && let Some(mut stdin) = child.stdin.take()
  {
    std::thread::spawn(move || {
      let _ = stdin.write_all(input.as_bytes());
    });
  }
}

fn spawn(
  cmd: &mut Command,
  capture: bool,
  input: Option<String>,
) -> std::io::Result<Spawned> {
  if input.is_some() {
    cmd.stdin(Stdio::piped());
  }

  let (status, stdout, stderr) = if capture {
    let mut child =
      cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;

    feed(&mut child, input);

    let out = child.wait_with_output()?;

    (
      out.status,
      String::from_utf8_lossy(&out.stdout).into_owned(),
      String::from_utf8_lossy(&out.stderr).into_owned(),
    )
  } else {
    let mut child = cmd.spawn()?;

    feed(&mut child, input);

    (child.wait()?, String::new(), String::new())
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

  cmd.envs(it.env_overlay.iter().map(|(k, v)| (k, v)));

  if let Some(vars) = options.field("env") {
    cmd.envs(env_pairs(vars));
  }

  let input = match options.field("input") {
    Some(Value::Variant(v)) => v.fields.first().and_then(Value::as_str),
    _ => None,
  }
  .map(str::to_string);

  let _ = it.out.flush();

  match spawn(&mut cmd, capture, input) {
    Ok(done) => Ok(done),
    Err(e) => {
      let _ = writeln!(it.err, "error: cannot run `{command}`: {}", reason(&e));

      Ok(Spawned { code: 127, stdout: String::new(), stderr: String::new() })
    }
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

  it.env_overlay.extend(env_pairs(arg(a, 0)?));

  let result = it.apply(arg(a, 1)?, Vec::new());

  it.env_overlay.truncate(mark);

  result
}
