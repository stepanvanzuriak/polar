use polar_compiler::{
  CompileOptions, Stage, dump_stage_with,
  shared::codes::DiagnosticCode,
  shared::modules::ModuleSource,
  shared::render::{RenderOptions, render_diagnostics},
  shared::source::SourceFile,
};
use std::{
  collections::HashMap,
  fs,
  path::{Path, PathBuf},
  sync::Arc,
};

fn dir() -> PathBuf {
  PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/types")
}

fn stem(path: &Path) -> String {
  path.file_stem().unwrap().to_string_lossy().into_owned()
}

fn fixtures() -> Vec<PathBuf> {
  let mut paths: Vec<PathBuf> = fs::read_dir(dir())
    .unwrap()
    .map(|entry| entry.unwrap().path())
    .filter(|path| path.extension().is_some_and(|e| e == "px"))
    .filter(|path| !stem(path).contains('.'))
    .collect();

  paths.sort();
  paths
}

fn header(src: &str) -> Option<String> {
  src
    .lines()
    .find_map(|l| l.strip_prefix("module "))
    .map(|n| n.trim().to_string())
}

fn modules_for(path: &Path, src: &str) -> Vec<ModuleSource> {
  let prefix = format!("{}.", stem(path));
  let mut modules: Vec<ModuleSource> = fs::read_dir(dir())
    .unwrap()
    .map(|entry| entry.unwrap().path())
    .filter(|other| stem(other).starts_with(&prefix))
    .map(|other| {
      let text = fs::read_to_string(&other).unwrap();

      ModuleSource {
        path: header(&text).unwrap_or_default(),
        source: text,
        specifier: String::new(),
        plugins: Vec::new(),
      }
    })
    .collect();

  if !modules.is_empty()
    && let Some(name) = header(src)
  {
    modules.push(ModuleSource {
      path: name,
      source: src.to_string(),
      specifier: String::new(),
      plugins: Vec::new(),
    });
  }

  modules
}

fn render(path: &Path) -> String {
  let src = fs::read_to_string(path).unwrap();
  let name = path.file_name().unwrap().to_string_lossy().into_owned();
  let options = CompileOptions {
    modules: modules_for(path, &src),
    ..CompileOptions::default()
  };
  let result = dump_stage_with(&src, &name, Stage::Types, &options);
  let mut files = HashMap::new();

  files.insert(Arc::from(name.as_str()), SourceFile::new(name.clone(), src));

  render_diagnostics(
    &result.diagnostics,
    &files,
    RenderOptions { color: false },
  )
}

#[test]
fn snapshots() {
  let bless = std::env::var("POLAR_BLESS").is_ok_and(|v| v == "1");
  let mut failures = Vec::new();

  for path in fixtures() {
    let rendered = render(&path);
    let expected_path = path.with_extension("stderr");

    if bless {
      fs::write(&expected_path, &rendered).unwrap();
      continue;
    }

    match fs::read_to_string(&expected_path) {
      Ok(expected) if expected == rendered => {}
      Ok(expected) => failures.push(format!(
        "{}\n--- expected\n{expected}\n--- got\n{rendered}",
        path.display()
      )),
      Err(_) => failures.push(format!(
        "{} has no .stderr; run with POLAR_BLESS=1\n{rendered}",
        path.display()
      )),
    }
  }

  assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

#[test]
fn every_type_code_has_a_fixture() {
  let all: String = fixtures().iter().map(|p| render(p)).collect();

  for code in DiagnosticCode::ALL {
    let n = *code as u16;

    if (501..=513).contains(&n)
      || (601..=603).contains(&n)
      || (701..=709).contains(&n)
      || n == 314
    {
      assert!(all.contains(&format!("[{code}]")), "no fixture shows {code}");
    }
  }
}

#[test]
fn no_internal_names() {
  for path in fixtures() {
    let rendered = render(&path);
    let internal = rendered.char_indices().any(|(i, c)| {
      c == '?' && rendered[i + 1..].starts_with(|d: char| d.is_ascii_digit())
    });

    assert!(!internal, "{}: {rendered}", path.display());
    assert!(!rendered.contains("Gen("), "{}: {rendered}", path.display());

    if !stem(&path).starts_with("qualified") {
      assert!(!rendered.contains("Std."), "{}: {rendered}", path.display());
    }
  }
}
