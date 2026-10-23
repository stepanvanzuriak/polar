use crate::{
  builtins::{arg, bytes, number, text},
  interp::{Control, Interp, R},
  value::Value,
};
use std::{
  fs,
  io::{self, Write},
  path::Path,
  time::{Duration, UNIX_EPOCH},
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
    io::ErrorKind::Unsupported => ("ENOTSUP", "operation not supported"),
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

fn copy_tree(from: &Path, to: &Path) -> io::Result<()> {
  let meta = fs::symlink_metadata(from)?;

  if meta.is_dir() {
    fs::create_dir_all(to)?;

    for entry in fs::read_dir(from)? {
      let entry = entry?;

      copy_tree(&entry.path(), &to.join(entry.file_name()))?;
    }

    fs::set_permissions(to, meta.permissions())
  } else if meta.is_symlink() {
    let _ = fs::remove_file(to);

    link(&fs::read_link(from)?, to)
  } else {
    fs::copy(from, to).map(|_| ())
  }
}

#[cfg(unix)]
fn link(target: &Path, path: &Path) -> io::Result<()> {
  std::os::unix::fs::symlink(target, path)
}

#[cfg(not(unix))]
fn link(_: &Path, _: &Path) -> io::Result<()> {
  Err(io::ErrorKind::Unsupported.into())
}

pub fn copy(_: &mut Interp, a: &[Value]) -> R {
  let (from, to) = (text(a, 0)?, text(a, 1)?);

  attempt(copy_tree(Path::new(from), Path::new(to)), "cp", from, |()| {
    Value::unit()
  })
}

pub fn symlink(_: &mut Interp, a: &[Value]) -> R {
  let (target, path) = (text(a, 0)?, text(a, 1)?);

  attempt(link(Path::new(target), Path::new(path)), "symlink", path, |()| {
    Value::unit()
  })
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn touch(path: &str, millis: f64) -> io::Result<()> {
  let when = if millis >= 0.0 {
    UNIX_EPOCH + Duration::from_millis(millis as u64)
  } else {
    UNIX_EPOCH - Duration::from_millis(-millis as u64)
  };

  fs::File::open(path)?.set_times(fs::FileTimes::new().set_modified(when))
}

pub fn set_mtime(_: &mut Interp, a: &[Value]) -> R {
  let path = text(a, 0)?;

  attempt(touch(path, number(a, 1)?), "utime", path, |()| Value::unit())
}

#[cfg(unix)]
fn set_mode(path: &str, mode: u32) -> io::Result<()> {
  use std::os::unix::fs::PermissionsExt;

  fs::set_permissions(path, fs::Permissions::from_mode(mode))
}

#[cfg(not(unix))]
fn set_mode(_: &str, _: u32) -> io::Result<()> {
  Err(io::ErrorKind::Unsupported.into())
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub fn chmod(_: &mut Interp, a: &[Value]) -> R {
  let path = text(a, 0)?;

  attempt(set_mode(path, number(a, 1)? as u32), "chmod", path, |()| {
    Value::unit()
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
