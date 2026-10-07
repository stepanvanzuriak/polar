use crate::{CliError, files::write_atomic};
use std::fmt::Write as _;
use std::{
  collections::{BTreeMap, HashMap},
  fs,
  path::{Path, PathBuf},
  process::Command,
  sync::{Mutex, OnceLock},
};

const API_LIB: &str = include_str!("../../plugin/src/lib.rs");

const API_MANIFEST: &str = r#"[package]
name = "polar-plugin"
version = "0.0.0"
edition = "2024"
publish = false

[dependencies]
serde = { version = "1", features = ["derive"] }
serde_json = "1"
"#;

pub const DIR: &str = ".polar";

pub const SOURCE: &str = "plugin";

fn built() -> &'static Mutex<HashMap<PathBuf, String>> {
  static BUILT: OnceLock<Mutex<HashMap<PathBuf, String>>> = OnceLock::new();

  BUILT.get_or_init(|| Mutex::new(HashMap::new()))
}

fn notices() -> &'static Mutex<Vec<String>> {
  static NOTICES: OnceLock<Mutex<Vec<String>>> = OnceLock::new();

  NOTICES.get_or_init(|| Mutex::new(Vec::new()))
}

pub(crate) fn take_notices() -> Vec<String> {
  notices().lock().map(|mut n| std::mem::take(&mut *n)).unwrap_or_default()
}

pub(crate) fn notice(message: String) {
  if let Ok(mut n) = notices().lock()
    && !n.contains(&message)
  {
    n.push(message);
  }
}

/// Makes sure the plugin of the package in `dir` exists, is built and
/// defines exactly `zones`, and returns the path of its library.
///
/// # Errors
///
/// Fails if the plugin can't be written, doesn't build, can't be loaded, or
/// defines other zones than the manifest lists.
pub fn prepare(
  dir: &Path,
  display: &Path,
  package: &str,
  zones: &[String],
  dependencies: &BTreeMap<String, toml::Value>,
  shown: &str,
) -> Result<String, CliError> {
  if let Some(path) = built().lock().ok().and_then(|b| b.get(dir).cloned()) {
    return Ok(path);
  }

  let manifest = dir.join(DIR).join("plugin").join("Cargo.toml");

  if write_files(dir, package, zones, dependencies)? {
    notice(format!(
      "note: created {} with a stub for the zones {}; implement them there",
      display.join(SOURCE).join("lib.rs").display(),
      zones.iter().map(|z| format!("`{z}`")).collect::<Vec<_>>().join(", "),
    ));
  }

  let library = build(&manifest, &display.join(SOURCE))?;
  let defined = polar_compiler::syntax::plugins::load(&library)
    .map_err(CliError::Message)?;
  let names: Vec<&str> = defined.iter().map(|z| z.keyword.as_str()).collect();

  for zone in zones {
    if !names.contains(&zone.as_str()) {
      return Err(CliError::Message(format!(
        "invalid {shown}: `[plugin]` lists the zone `{zone}`, but the plugin \
         doesn't define it"
      )));
    }
  }

  for name in &names {
    if !zones.iter().any(|z| z == name) {
      return Err(CliError::Message(format!(
        "invalid {shown}: the plugin defines the zone `{name}`; add it to \
         `zones` under `[plugin]`"
      )));
    }
  }

  if let Ok(mut b) = built().lock() {
    b.insert(dir.to_path_buf(), library.clone());
  }

  Ok(library)
}

pub(crate) fn write_files(
  dir: &Path,
  package: &str,
  zones: &[String],
  dependencies: &BTreeMap<String, toml::Value>,
) -> Result<bool, CliError> {
  let root = dir.join(DIR);
  let lib = dir.join(SOURCE).join("lib.rs");
  let created = !lib.is_file();

  sync_api(&root.join("api"))?;

  if created {
    scaffold(&lib, zones)?;
  }

  write_if_changed(
    &root.join("plugin").join("Cargo.toml"),
    &cargo_manifest(package, dependencies),
  )?;

  Ok(created)
}

fn write_if_changed(path: &Path, text: &str) -> Result<(), CliError> {
  if fs::read_to_string(path).is_ok_and(|old| old == text) {
    return Ok(());
  }

  write_atomic(path, text.as_bytes())
    .map_err(|e| CliError::write(path.display().to_string(), &e))
}

fn sync_api(dir: &Path) -> Result<(), CliError> {
  write_if_changed(&dir.join("Cargo.toml"), API_MANIFEST)?;
  write_if_changed(&dir.join("src").join("lib.rs"), API_LIB)
}

fn pascal(keyword: &str) -> String {
  keyword
    .split('_')
    .map(|part| {
      let mut chars = part.chars();

      chars
        .next()
        .map(|c| c.to_uppercase().chain(chars).collect::<String>())
        .unwrap_or_default()
    })
    .collect()
}

fn cargo_manifest(
  package: &str,
  dependencies: &BTreeMap<String, toml::Value>,
) -> String {
  let mut deps = String::from("polar-plugin = { path = \"../api\" }\n");

  for (name, spec) in dependencies {
    let _ = writeln!(deps, "{name} = {spec}");
  }

  format!(
    "[package]\nname = \"{package}_plugin\"\nversion = \"0.1.0\"\nedition = \
     \"2024\"\npublish = false\n\n[lib]\npath = \"../../{SOURCE}/lib.rs\"\n\
     crate-type = [\"cdylib\"]\n\n[dependencies]\n{deps}\n[workspace]\n"
  )
}

fn scaffold(lib: &Path, zones: &[String]) -> Result<(), CliError> {
  let mut source = String::from(
    "use polar_plugin::{Diagnostic, Entry, Expansion, Module, Span, Zone, \
     ZonePlugin, export};\n",
  );

  for zone in zones {
    let name = pascal(zone);

    let _ = write!(
      source,
      "\nstruct {name};\n\nimpl ZonePlugin for {name} {{\n  fn zone(&self) -> \
       Zone {{\n    Zone {{\n      keyword: \"{zone}\".to_string(),\n      \
       after: \"types\".to_string(),\n      blank_between_entries: false,\n    \
       }}\n  }}\n\n  fn expand(&self, zone: Span, _entries: &[Entry], _module: \
       &Module) -> Expansion {{\n    let mut out = Expansion::default();\n\n    \
       out.error(\n      Diagnostic::error(\"the `{zone}` zone isn't implemented \
       yet\", zone)\n        .with_help(\"implement it in {SOURCE}/lib.rs\"),\n    \
       );\n    out\n  }}\n}}\n"
    );
  }

  let names: Vec<String> = zones.iter().map(|z| pascal(z)).collect();

  let _ = write!(source, "\nexport!({});\n", names.join(", "));

  write_if_changed(lib, &source)
}

fn build(manifest: &Path, display: &Path) -> Result<String, CliError> {
  let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
  let out = Command::new(&cargo)
    .args(["build", "--quiet", "--message-format=json-render-diagnostics"])
    .arg("--manifest-path")
    .arg(manifest)
    .output()
    .map_err(|e| {
      CliError::Message(format!(
        "building the plugin in {} needs cargo: cannot run `{cargo}`: {e}",
        display.display()
      ))
    })?;

  if !out.status.success() {
    return Err(CliError::Message(format!(
      "the plugin in {} doesn't build:\n{}",
      display.display(),
      String::from_utf8_lossy(&out.stderr).trim_end()
    )));
  }

  String::from_utf8_lossy(&out.stdout)
    .lines()
    .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
    .filter(|m| m["reason"] == "compiler-artifact")
    .filter(|m| {
      m["target"]["kind"]
        .as_array()
        .is_some_and(|kinds| kinds.iter().any(|k| k == "cdylib"))
    })
    .find_map(|m| m["filenames"][0].as_str().map(ToString::to_string))
    .ok_or_else(|| {
      CliError::Message(format!(
        "the plugin in {} builds no library: its Cargo.toml needs \
         `crate-type = [\"cdylib\"]`",
        display.display()
      ))
    })
}
