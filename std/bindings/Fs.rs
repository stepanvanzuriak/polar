use crate::{
  builtins::{arg, bytes, text},
  interp::{Control, Interp, R},
  value::Value,
};
use std::{
  fs,
  io::{self, Write},
};
use tempfile::TempDir;
use walkdir::WalkDir;

fn code_of(e: &io::Error) -> (&'static str, &'static str) {
  match e.kind() {
    io::ErrorKind::NotFound => ("ENOENT", "no such file or directory"),
    io::ErrorKind::PermissionDenied => ("EACCES", "permission denied"),
    io::ErrorKind::AlreadyExists => ("EEXIST", "file already exists"),
    io::ErrorKind::NotADirectory => ("ENOTDIR", "not a directory"),
    io::ErrorKind::IsADirectory => {
      ("EISDIR", "illegal operation on a directory")
    }
    io::ErrorKind::InvalidInput => ("EINVAL", "invalid argument"),
    io::ErrorKind::StorageFull => ("ENOSPC", "no space left on device"),
    io::ErrorKind::DirectoryNotEmpty => ("ENOTEMPTY", "directory not empty"),
    _ => ("EUNKNOWN", "unknown error"),
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

fn strings(mut names: Vec<String>) -> Value {
  names.sort_by_key(|n| n.encode_utf16().collect::<Vec<u16>>());

  Value::list(names.into_iter().map(Value::str).collect())
}

pub fn read(_: &mut Interp, a: &[Value]) -> R {
  let path = text(a, 0)?;

  attempt(fs::read(path), "open", path, |bytes| {
    Value::str(String::from_utf8_lossy(&bytes))
  })
}

pub fn read_bytes(_: &mut Interp, a: &[Value]) -> R {
  let path = text(a, 0)?;

  attempt(fs::read(path), "open", path, Value::bytes)
}

pub fn write_bytes(_: &mut Interp, a: &[Value]) -> R {
  let (path, data) = (text(a, 0)?, bytes(a, 1)?);

  attempt(fs::write(path, data), "open", path, |()| Value::unit())
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

fn files(root: &str) -> io::Result<Vec<String>> {
  if !fs::metadata(root)?.is_dir() {
    return Err(io::ErrorKind::NotADirectory.into());
  }

  WalkDir::new(root)
    .min_depth(1)
    .into_iter()
    .filter(|entry| !entry.as_ref().is_ok_and(|e| e.file_type().is_dir()))
    .map(|entry| {
      let entry = entry?;
      let relative = entry.path().strip_prefix(root).unwrap_or(entry.path());

      Ok(relative.to_string_lossy().replace('\\', "/"))
    })
    .collect()
}

pub fn walk(_: &mut Interp, a: &[Value]) -> R {
  let path = text(a, 0)?;

  attempt(files(path), "scandir", path, strings)
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

fn temp(prefix: &str) -> io::Result<TempDir> {
  tempfile::Builder::new().prefix(prefix).tempdir()
}

pub fn mkdtemp(_: &mut Interp, a: &[Value]) -> R {
  let prefix = text(a, 0)?;

  attempt(temp(prefix), "mkdtemp", prefix, |dir| {
    Value::str(dir.keep().to_string_lossy())
  })
}

pub fn with_temp_dir(it: &mut Interp, a: &[Value]) -> R {
  let dir = temp(text(a, 0)?).map_err(|e| {
    Control::fail(format!("cannot create a temporary directory: {e}"))
  })?;

  it.apply(arg(a, 1)?, vec![Value::str(dir.path().to_string_lossy())])
}
