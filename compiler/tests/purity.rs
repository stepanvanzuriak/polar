use std::{fs, path::Path};

const FORBIDDEN: &[&str] = &[
  "std::fs",
  "std::io",
  "std::net",
  "std::process",
  "std::env",
  "std::thread",
  "println!",
  "eprintln!",
  "print!",
  "eprint!",
  "dbg!",
];

fn violations(source: &str) -> Vec<(usize, &'static str)> {
  let mut out = Vec::new();

  for (i, line) in source.lines().enumerate() {
    let code = line.split("//").next().unwrap_or_default();

    for needle in FORBIDDEN {
      let hits = code.match_indices(needle).any(|(at, _)| {
        let before = code[..at].chars().next_back();

        !before.is_some_and(|c| c.is_alphanumeric() || c == '_')
      });

      if hits {
        out.push((i + 1, *needle));
      }
    }
  }

  out
}

fn rust_files(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
  for entry in fs::read_dir(dir).expect("read compiler/src") {
    let path = entry.unwrap().path();

    if path.is_dir() {
      rust_files(&path, out);
    } else if path.extension().is_some_and(|e| e == "rs") {
      out.push(path);
    }
  }
}

#[test]
fn no_io_in_the_compiler() {
  let mut files = Vec::new();

  rust_files(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut files);
  assert!(files.len() > 10, "found too few sources");

  let found: Vec<String> = files
    .iter()
    .flat_map(|path| {
      let source = fs::read_to_string(path).unwrap();

      violations(&source)
        .into_iter()
        .map(|(line, needle)| format!("{}:{line}: {needle}", path.display()))
        .collect::<Vec<_>>()
    })
    .collect();

  assert!(
    found.is_empty(),
    "the compiler must not do I/O:\n{}",
    found.join("\n")
  );
}

#[test]
fn adding_io_fails() {
  let source = "use crate::syntax::ast;\nuse std::fs;\nfn f() {}\n";

  assert_eq!(violations(source), [(2, "std::fs")]);
  assert_eq!(violations("  eprintln!(\"x\");"), [(1, "eprintln!")]);
  assert!(violations("// std::fs in a comment\nlet sprint = 1;").is_empty());
}

fn dependency_tables(manifest: &str) -> Vec<String> {
  manifest
    .lines()
    .map(str::trim)
    .filter(|l| l.starts_with('[') && l.ends_with(']'))
    .map(|l| l.trim_matches(['[', ']']).to_string())
    .filter(|t| {
      t == "dependencies"
        || t == "build-dependencies"
        || (t.starts_with("target.") && !t.ends_with("dev-dependencies"))
        || t.starts_with("dependencies.")
    })
    .collect()
}

const PLUGIN_LOADING: &[&str] =
  &["libloading", "polar-plugin", "serde", "serde_json"];

fn dependency_names(manifest: &str) -> Vec<String> {
  let mut names = Vec::new();
  let mut inside = false;

  for line in manifest.lines().map(str::trim) {
    if line.starts_with('[') {
      inside = line == "[dependencies]";
      continue;
    }

    if inside && let Some((name, _)) = line.split_once('=') {
      names.push(name.trim().to_string());
    }
  }

  names.sort();
  names
}

#[test]
fn only_plugin_loading_dependencies() {
  let manifest =
    fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"))
      .unwrap();

  assert_eq!(dependency_tables(&manifest), ["dependencies"]);
  assert_eq!(dependency_names(&manifest), PLUGIN_LOADING);
}

#[test]
fn dev_dependencies_are_allowed() {
  let manifest = "[package]\nname = \"x\"\n\n[dev-dependencies]\nproptest = \"1\"\ninsta = \"1\"\n";

  assert!(dependency_tables(manifest).is_empty());
  assert_eq!(
    dependency_tables("[dependencies]\nregex = \"1\"\n"),
    ["dependencies"]
  );
  assert_eq!(
    dependency_tables("[target.'cfg(unix)'.dependencies]\nlibc = \"1\"\n"),
    ["target.'cfg(unix)'.dependencies"]
  );
}
