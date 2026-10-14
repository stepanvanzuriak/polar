use crate::{
  builtins::{arg, text},
  interp::{Control, Interp, R},
  value::Value,
};
use std::{
  fs,
  io::{self, Write},
  path::Path,
};

fn code_of(e: &io::Error) -> (&'static str, &'static str) {
  match e.raw_os_error() {
    Some(1) => ("EPERM", "operation not permitted"),
    Some(2) => ("ENOENT", "no such file or directory"),
    Some(13) => ("EACCES", "permission denied"),
    Some(17) => ("EEXIST", "file already exists"),
    Some(20) => ("ENOTDIR", "not a directory"),
    Some(21) => ("EISDIR", "illegal operation on a directory"),
    Some(22) => ("EINVAL", "invalid argument"),
    Some(28) => ("ENOSPC", "no space left on device"),
    Some(39) => ("ENOTEMPTY", "directory not empty"),
    _ => match e.kind() {
      io::ErrorKind::NotFound => ("ENOENT", "no such file or directory"),
      io::ErrorKind::PermissionDenied => ("EACCES", "permission denied"),
      io::ErrorKind::AlreadyExists => ("EEXIST", "file already exists"),
      _ => ("EUNKNOWN", "unknown error"),
    },
  }
}

fn failure(e: &io::Error, syscall: &str, path: &str) -> Value {
  let (code, description) = code_of(e);

  Value::err(Value::record(vec![
    ("code", Value::str(code)),
    ("path", Value::str(path)),
    (
      "message",
      Value::str(format!("{code}: {description}, {syscall} '{path}'")),
    ),
  ]))
}

fn attempt<T>(
  result: io::Result<T>,
  syscall: &str,
  path: &str,
  ok: impl FnOnce(T) -> Value,
) -> R {
  Ok(match result {
    Ok(value) => Value::ok(ok(value)),
    Err(e) => failure(&e, syscall, path),
  })
}

fn sorted(mut names: Vec<String>) -> Vec<String> {
  names.sort_by_key(|n| n.encode_utf16().collect::<Vec<u16>>());
  names
}

fn strings(names: Vec<String>) -> Value {
  Value::list(sorted(names).into_iter().map(Value::str).collect())
}

pub fn read(_: &mut Interp, a: &[Value]) -> R {
  let path = text(a, 0)?;

  attempt(fs::read(path), "open", path, |bytes| {
    Value::str(String::from_utf8_lossy(&bytes))
  })
}

pub fn write(_: &mut Interp, a: &[Value]) -> R {
  let (path, body) = (text(a, 0)?, text(a, 1)?);

  attempt(fs::write(path, body), "open", path, |()| Value::unit())
}

pub fn append(_: &mut Interp, a: &[Value]) -> R {
  let (path, body) = (text(a, 0)?, text(a, 1)?);
  let result = fs::OpenOptions::new()
    .append(true)
    .create(true)
    .open(path)
    .and_then(|mut f| f.write_all(body.as_bytes()));

  attempt(result, "open", path, |()| Value::unit())
}

pub fn exists(_: &mut Interp, a: &[Value]) -> R {
  Ok(Value::Bool(fs::metadata(text(a, 0)?).is_ok()))
}

pub fn is_dir(_: &mut Interp, a: &[Value]) -> R {
  Ok(Value::Bool(fs::metadata(text(a, 0)?).is_ok_and(|m| m.is_dir())))
}

pub fn mkdir_all(_: &mut Interp, a: &[Value]) -> R {
  let path = text(a, 0)?;

  attempt(fs::create_dir_all(path), "mkdir", path, |()| Value::unit())
}

fn names(path: &str) -> io::Result<Vec<String>> {
  fs::read_dir(path)?
    .map(|entry| entry.map(|e| e.file_name().to_string_lossy().into_owned()))
    .collect()
}

pub fn list(_: &mut Interp, a: &[Value]) -> R {
  let path = text(a, 0)?;

  attempt(names(path), "scandir", path, strings)
}

fn files(root: &Path, prefix: &str, out: &mut Vec<String>) -> io::Result<()> {
  let dir =
    if prefix.is_empty() { root.to_path_buf() } else { root.join(prefix) };

  for entry in fs::read_dir(dir)? {
    let entry = entry?;
    let name = entry.file_name().to_string_lossy().into_owned();
    let relative =
      if prefix.is_empty() { name } else { format!("{prefix}/{name}") };

    if entry.file_type()?.is_dir() {
      files(root, &relative, out)?;
    } else {
      out.push(relative);
    }
  }

  Ok(())
}

pub fn walk(_: &mut Interp, a: &[Value]) -> R {
  let path = text(a, 0)?;
  let mut out = Vec::new();

  attempt(files(Path::new(path), "", &mut out), "scandir", path, |()| {
    strings(out)
  })
}

pub fn remove(_: &mut Interp, a: &[Value]) -> R {
  let path = text(a, 0)?;
  let meta = match fs::symlink_metadata(path) {
    Ok(m) => m,
    Err(e) => return Ok(failure(&e, "lstat", path)),
  };

  if meta.is_dir() {
    attempt(fs::remove_dir(path), "rmdir", path, |()| Value::unit())
  } else {
    attempt(fs::remove_file(path), "unlink", path, |()| Value::unit())
  }
}

pub fn remove_all(_: &mut Interp, a: &[Value]) -> R {
  let path = text(a, 0)?;
  let result = match fs::symlink_metadata(path) {
    Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
    Err(e) => Err(e),
    Ok(m) if m.is_dir() => fs::remove_dir_all(path),
    Ok(_) => fs::remove_file(path),
  };

  attempt(result, "rm", path, |()| Value::unit())
}

fn temp(prefix: &str) -> io::Result<String> {
  let dir = tempfile::Builder::new()
    .prefix(prefix)
    .tempdir_in(std::env::temp_dir())?
    .keep();

  Ok(dir.to_string_lossy().into_owned())
}

pub fn mkdtemp(_: &mut Interp, a: &[Value]) -> R {
  let prefix = text(a, 0)?;

  attempt(temp(prefix), "mkdtemp", prefix, Value::str)
}

pub fn with_temp_dir(it: &mut Interp, a: &[Value]) -> R {
  let prefix = text(a, 0)?;
  let dir = temp(prefix).map_err(|e| {
    Control::fail(format!("cannot create a temporary directory: {e}"))
  })?;
  let result = it.apply(arg(a, 1)?, vec![Value::str(&dir)]);
  let _ = fs::remove_dir_all(&dir);

  result
}
