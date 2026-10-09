use crate::{
  CliError,
  project::{Contexts, Package},
};
use polar_compiler::{
  shared::codes::DiagnosticCode,
  shared::diagnostic::{Diagnostic, Label},
  shared::source::Span,
};
use std::{
  collections::HashMap,
  fs,
  io::{self, Write},
  path::{Component, Path, PathBuf},
  sync::Arc,
};
use tempfile::NamedTempFile;
use walkdir::{DirEntry, WalkDir};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Input {
  pub display: String,
  pub path: PathBuf,
  pub output: PathBuf,
  pub package: Option<Arc<Package>>,
}

impl Input {
  #[must_use]
  pub fn plugins(&self) -> Vec<String> {
    self.package.as_ref().map(|p| p.plugins.clone()).unwrap_or_default()
  }

  #[must_use]
  pub fn out_root(&self) -> PathBuf {
    self.package.as_ref().map(|p| p.out_root()).unwrap_or_default()
  }

  #[must_use]
  pub fn source_root(&self) -> Option<PathBuf> {
    match self.package.as_deref() {
      Some(Package { module: Some(_), source, .. })
        if self.output.starts_with("_deps") =>
      {
        Some(source.path.clone())
      }
      _ => self
        .path
        .ancestors()
        .nth(self.output.components().count())
        .map(Path::to_path_buf),
    }
  }
}

fn is_pruned(entry: &DirEntry, out: Option<&Path>) -> bool {
  if entry.depth() == 0 || !entry.file_type().is_dir() {
    return false;
  }

  let name = entry.file_name().to_string_lossy();

  matches!(name.as_ref(), "node_modules" | "snapshots" | "target")
    || name.starts_with('.')
    || out.is_some_and(|out| normalize(entry.path()) == out)
}

#[must_use]
pub fn without_tests(
  inputs: Vec<Input>,
  args: &[PathBuf],
  cwd: &Path,
) -> Vec<Input> {
  let named: Vec<PathBuf> =
    args.iter().map(|arg| normalize(&cwd.join(arg))).collect();

  inputs
    .into_iter()
    .filter(|input| {
      !input.path.to_string_lossy().ends_with("_test.px")
        || named.contains(&normalize(&cwd.join(&input.path)))
    })
    .collect()
}

/// Expands file and directory arguments into the `.px` inputs to compile.
///
/// # Errors
///
/// Fails if an argument cannot be read, a file argument lacks the `.px`
/// extension, or two inputs would be written to the same output.
pub fn discover(
  args: &[PathBuf],
  cwd: &Path,
  out: Option<&Path>,
) -> Result<Vec<Input>, CliError> {
  let out = out.map(|out| normalize(&cwd.join(out)));
  let mut inputs = Vec::new();

  for arg in args {
    let path = normalize(&cwd.join(arg));
    let display = arg.display().to_string();
    let metadata =
      fs::metadata(&path).map_err(|e| CliError::read(&display, &e))?;

    if metadata.is_file() {
      if path.extension().is_none_or(|ext| ext != "px") {
        return Err(CliError::Message(format!(
          "`{display}` is not a Polar file: expected a `.px` extension"
        )));
      }

      let output = PathBuf::from(path.file_stem().unwrap_or_default());

      inputs.push(Input { display, path, output, package: None });
      continue;
    }

    let walk = WalkDir::new(&path)
      .sort_by_file_name()
      .into_iter()
      .filter_entry(|entry| !is_pruned(entry, out.as_deref()));

    for entry in walk {
      let entry = entry.map_err(|e| {
        let reason =
          e.io_error().map_or_else(|| e.to_string(), crate::io_reason);
        let at = e.path().map_or(display.clone(), |p| p.display().to_string());

        CliError::Read { path: at, reason }
      })?;

      let found = entry.path();

      if !entry.file_type().is_file()
        || found.extension().is_none_or(|e| e != "px")
      {
        continue;
      }

      let relative = found.strip_prefix(&path).unwrap_or(found);

      inputs.push(Input {
        display: arg.join(relative).display().to_string(),
        path: found.to_path_buf(),
        output: relative.with_extension(""),
        package: None,
      });
    }
  }

  check_collisions(&inputs)?;

  let mut contexts = Contexts::default();

  for input in &mut inputs {
    input.package = contexts.of(&input.path, cwd)?;
  }

  Ok(inputs)
}

fn check_collisions(inputs: &[Input]) -> Result<(), CliError> {
  let mut seen: HashMap<&Path, &str> = HashMap::new();

  for input in inputs {
    let first =
      input.output.components().next().map(std::path::Component::as_os_str);
    let nested = input.output.components().count() > 1;

    if first.is_some_and(|c| c == "_polar") && nested {
      return Err(CliError::Message(format!(
        "output collision: `{}` would be written into `_polar/`, where the runtime goes",
        input.display
      )));
    }

    if first.is_some_and(|c| c == "_deps") && nested {
      return Err(CliError::Message(format!(
        "output collision: `{}` would be written into `_deps/`, where dependencies go",
        input.display
      )));
    }

    if let Some(other) = seen.insert(&input.output, &input.display) {
      return Err(CliError::Message(format!(
        "output collision: `{other}` and `{}` would both be written to `{}.js`",
        input.display,
        input.output.display()
      )));
    }
  }

  Ok(())
}

#[derive(Debug, Clone)]
pub struct Source {
  pub text: String,
  pub invalid: Option<Diagnostic>,
}

const BOM: &[u8] = &[0xEF, 0xBB, 0xBF];

/// Reads an input from disk, decoding it as UTF-8 (see [`decode`]).
///
/// # Errors
///
/// Fails if the file cannot be read.
pub fn read_source(input: &Input) -> Result<Source, CliError> {
  let bytes =
    read_bytes(input).map_err(|e| CliError::read(&input.display, &e))?;

  Ok(decode(&input.display, &bytes))
}

/// Reads an input's bytes from disk. A module no file holds may be a sibling
/// that a plugin zone in a file next to it generates (see [`siblings`]).
///
/// # Errors
///
/// Fails if the file cannot be read and no zone generates it.
pub fn read_bytes(input: &Input) -> io::Result<Vec<u8>> {
  match fs::read(&input.path) {
    Err(e) if e.kind() == io::ErrorKind::NotFound => {
      generated_source(input).map(String::into_bytes).ok_or(e)
    }
    read => read,
  }
}

/// The source of the sibling module `input` names, from whichever `.px` file
/// in its folder generates it.
fn generated_source(input: &Input) -> Option<String> {
  let plugins = input.plugins();

  if plugins.is_empty() {
    return None;
  }

  let dir = input.path.parent()?;
  let wanted = input.path.file_name()?.to_str()?;
  let mut files: Vec<PathBuf> = fs::read_dir(dir)
    .ok()?
    .filter_map(|entry| entry.ok().map(|entry| entry.path()))
    .filter(|path| {
      path.is_file() && path.extension().is_some_and(|e| e == "px")
    })
    .collect();

  files.sort();

  files.into_iter().find_map(|path| {
    let text = fs::read_to_string(&path).ok()?;
    let display = Path::new(&input.display)
      .with_file_name(path.file_name()?)
      .display()
      .to_string();
    let generated = crate::guard(&display, || {
      polar_compiler::shared::modules::siblings(&text, &display, &plugins)
    })
    .ok()?;

    generated
      .into_iter()
      .find(|(name, _)| {
        polar_compiler::shared::modules::sibling_file(name) == wanted
      })
      .map(|(_, source)| source)
  })
}

/// The modules the plugin zones in `input` generate next to it, as inputs to
/// compile with it.
///
/// # Errors
///
/// Fails if a plugin panics, or a generated module's file already exists.
pub fn siblings(input: &Input, text: &str) -> Result<Vec<Input>, CliError> {
  let plugins = input.plugins();

  if plugins.is_empty() {
    return Ok(Vec::new());
  }

  let generated = crate::guard(&input.display, || {
    polar_compiler::shared::modules::siblings(text, &input.display, &plugins)
  })
  .map_err(CliError::Ice)?;
  let mut inputs = Vec::new();

  for (name, _) in generated {
    let file = polar_compiler::shared::modules::sibling_file(&name);
    let path = input.path.with_file_name(&file);
    let display =
      Path::new(&input.display).with_file_name(&file).display().to_string();

    if path.exists() {
      return Err(CliError::Message(format!(
        "`{}` generates the module `{name}` next to it, but `{display}` \
         already exists; rename or remove that file",
        input.display
      )));
    }

    inputs.push(Input {
      display,
      path,
      output: input.output.with_file_name(Path::new(&file).with_extension("")),
      package: input.package.clone(),
    });
  }

  Ok(inputs)
}

#[must_use]
pub fn decode(display: &str, bytes: &[u8]) -> Source {
  let bytes = bytes.strip_prefix(BOM).unwrap_or(bytes);

  match std::str::from_utf8(bytes) {
    Ok(text) => Source { text: text.to_string(), invalid: None },
    Err(e) => {
      let at = e.valid_up_to();
      let span = Span::empty(Arc::from(display), at);
      let diagnostic = Diagnostic::error(
        DiagnosticCode::InvalidUtf8,
        "source file is not valid UTF-8",
        Label::new(span).with_message("this byte is not valid UTF-8"),
      )
      .with_help("save the file as UTF-8");

      Source {
        text: String::from_utf8_lossy(bytes).into_owned(),
        invalid: Some(diagnostic),
      }
    }
  }
}

/// Writes `bytes` to `target` through a temporary file in the same directory.
///
/// # Errors
///
/// Fails if the directory cannot be created or the file cannot be written or
/// moved into place.
pub fn write_atomic(target: &Path, bytes: &[u8]) -> io::Result<()> {
  write_atomic_with(target, bytes, |_| Ok(()))
}

/// Like [`write_atomic`], calling `before_persist` on the temporary file just
/// before it replaces `target`.
///
/// # Errors
///
/// As [`write_atomic`], and whatever `before_persist` returns.
pub fn write_atomic_with(
  target: &Path,
  bytes: &[u8],
  before_persist: impl FnOnce(&Path) -> io::Result<()>,
) -> io::Result<()> {
  let dir = target
    .parent()
    .filter(|p| !p.as_os_str().is_empty())
    .unwrap_or(Path::new("."));

  fs::create_dir_all(dir)?;

  let mut temp = NamedTempFile::new_in(dir)?;

  temp.write_all(bytes)?;
  temp.flush()?;
  temp.as_file().sync_all()?;
  before_persist(temp.path())?;
  temp.persist(target).map_err(|e| e.error)?;

  Ok(())
}

#[must_use]
pub fn normalize(path: &Path) -> PathBuf {
  let mut out = PathBuf::new();

  for component in path.components() {
    match component {
      Component::CurDir => {}
      Component::ParentDir => {
        if !out.pop() {
          out.push("..");
        }
      }
      other => out.push(other),
    }
  }

  out
}

#[must_use]
pub fn relative_path(from: &Path, to: &Path) -> String {
  let (from, to) = (normalize(from), normalize(to));
  let from: Vec<_> = from.components().collect();
  let to: Vec<_> = to.components().collect();
  let common = from.iter().zip(&to).take_while(|(a, b)| a == b).count();
  let ups = std::iter::repeat_n("..".to_string(), from.len() - common);
  let downs =
    to[common..].iter().map(|c| c.as_os_str().to_string_lossy().into_owned());
  let parts: Vec<String> = ups.chain(downs).collect();

  if parts.is_empty() { ".".to_string() } else { parts.join("/") }
}

#[must_use]
pub fn runtime_specifier(output: &Path) -> String {
  format!("{}_polar/runtime.js", to_out_dir(output))
}

#[must_use]
pub fn module_specifier(from: &Path, to: &Path) -> String {
  let to: Vec<String> = to
    .components()
    .map(|c| c.as_os_str().to_string_lossy().into_owned())
    .collect();

  format!("{}{}.js", to_out_dir(from), to.join("/"))
}

fn to_out_dir(output: &Path) -> String {
  match output.components().count().saturating_sub(1) {
    0 => "./".to_string(),
    depth => "../".repeat(depth),
  }
}

#[must_use]
pub fn module_input(from: &Input, path: &str) -> Input {
  let first = path.split('.').next().unwrap_or(path);

  if let Some(package) = &from.package {
    let owner =
      package.deps.iter().find(|d| d.module.as_deref() == Some(first)).or_else(
        || (package.module.as_deref() == Some(first)).then_some(package),
      );

    if let Some(owner) = owner {
      return package_input(owner, path);
    }
  }

  let file = PathBuf::from(polar_compiler::shared::modules::file(path));
  let depth = from.output.components().count();
  let root = |path: &Path| {
    let mut root = path.to_path_buf();

    for _ in 0..depth {
      root.pop();
    }

    root
  };

  Input {
    display: root(Path::new(&from.display)).join(&file).display().to_string(),
    path: root(&from.path).join(&file),
    output: file.with_extension(""),
    package: from.package.clone(),
  }
}

fn package_input(package: &Arc<Package>, path: &str) -> Input {
  let file = match path.split_once('.') {
    Some((_, rest)) => {
      PathBuf::from(polar_compiler::shared::modules::file(rest))
    }
    None => PathBuf::from(".px"),
  };
  let output = package.out_root().join(file.with_extension(""));
  let display = package.source.display.join(&file).display().to_string();
  let path = package.source.path.join(&file);

  Input { display, path, output, package: Some(package.clone()) }
}

#[must_use]
pub fn rewrite_map(map: &str, file: &str, source: &str) -> String {
  let Ok(mut json) = serde_json::from_str::<serde_json::Value>(map) else {
    return map.to_string();
  };

  if let Some(object) = json.as_object_mut() {
    object.insert("file".into(), file.into());
    object.insert("sources".into(), serde_json::json!([source]));
  }

  json.to_string()
}
