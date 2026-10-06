use polar_cli::{Compiler, Io, run_with};
use std::fs;
use std::path::Path;
use std::{collections::HashMap, ffi::OsString};

struct Ran {
  code: u8,
  out: String,
  err: String,
}

fn polar_in(
  cwd: &Path,
  args: &[&str],
  env: &[(&str, &str)],
  compiler: Compiler,
) -> Ran {
  let (mut out, mut err) = (Vec::new(), Vec::new());
  let env: HashMap<String, String> =
    env.iter().map(|(k, v)| ((*k).to_string(), (*v).to_string())).collect();
  let lookup = |name: &str| env.get(name).cloned();
  let args: Vec<OsString> = args.iter().map(OsString::from).collect();
  let code = {
    let mut io = Io {
      out: &mut out,
      err: &mut err,
      stdout_is_tty: false,
      stderr_is_tty: false,
      cwd: cwd.to_path_buf(),
      env: &lookup,
    };

    run_with(&args, &mut io, compiler)
  };

  Ran {
    code,
    out: String::from_utf8(out).expect("stdout is UTF-8"),
    err: String::from_utf8(err).expect("stderr is UTF-8"),
  }
}

fn polar(cwd: &Path, args: &[&str]) -> Ran {
  polar_in(cwd, args, &[], Compiler::default())
}

fn tree(dir: &Path) -> Vec<String> {
  let mut out: Vec<String> = walkdir::WalkDir::new(dir)
    .into_iter()
    .filter_map(Result::ok)
    .filter(|e| e.file_type().is_file())
    .map(|e| e.path().strip_prefix(dir).unwrap().display().to_string())
    .collect();

  out.sort();
  out
}

fn read(path: &Path) -> String {
  fs::read_to_string(path).unwrap()
}

#[test]
fn creates_a_project_that_runs() {
  let dir = tempfile::tempdir().unwrap();

  let ran = polar(dir.path(), &["init", "hello"]);

  assert_eq!(ran.code, 0, "{}", ran.err);
  assert_eq!(
    read(&dir.path().join("hello/polar.toml")),
    "[project]\nname = \"hello\"\n"
  );
  assert!(dir.path().join("hello/src/main.px").is_file());
  assert!(ran.out.contains("next: cd hello && polar run"), "{}", ran.out);

  let ran = polar(&dir.path().join("hello"), &["check"]);

  assert_eq!(ran.code, 0, "{}", ran.err);
}

#[test]
fn init_here() {
  let dir = tempfile::tempdir().unwrap();
  let root = dir.path().join("my-here");

  fs::create_dir(&root).unwrap();

  let ran = polar(&root, &["init"]);

  assert_eq!(ran.code, 0, "{}", ran.err);
  assert!(root.join("polar.toml").is_file());
  assert!(ran.out.contains("next: polar run"), "{}", ran.out);
  assert!(!ran.out.contains("cd "), "{}", ran.out);
}

#[test]
fn creates_nested_dir() {
  let dir = tempfile::tempdir().unwrap();

  let ran = polar(dir.path(), &["init", "a/b/c"]);

  assert_eq!(ran.code, 0, "{}", ran.err);
  assert!(dir.path().join("a/b/c/polar.toml").is_file());
}

#[test]
fn refuses_existing_project() {
  let dir = tempfile::tempdir().unwrap();

  fs::write(dir.path().join("polar.toml"), "junk").unwrap();

  let ran = polar(dir.path(), &["init"]);

  assert_eq!(ran.code, 1);
  assert!(ran.err.contains("already exists"), "{}", ran.err);
  assert_eq!(read(&dir.path().join("polar.toml")), "junk");
  assert_eq!(tree(dir.path()), ["polar.toml"]);
}

#[test]
fn keeps_existing_files() {
  let dir = tempfile::tempdir().unwrap();

  fs::create_dir(dir.path().join("src")).unwrap();
  fs::write(dir.path().join("src/main.px"), "old").unwrap();

  let ran = polar(dir.path(), &["init", "--name", "app"]);

  assert_eq!(ran.code, 0, "{}", ran.err);
  assert_eq!(read(&dir.path().join("src/main.px")), "old");
  assert!(ran.out.contains("kept src/main.px"), "{}", ran.out);
}

#[test]
fn name_from_folder() {
  let dir = tempfile::tempdir().unwrap();

  let ran = polar(dir.path(), &["init", "My-App"]);

  assert_eq!(ran.code, 0, "{}", ran.err);
  assert!(
    read(&dir.path().join("My-App/polar.toml")).contains("name = \"my_app\"")
  );
}

#[test]
fn name_flag_wins() {
  let dir = tempfile::tempdir().unwrap();

  let ran = polar(dir.path(), &["init", "x", "--name", "shop"]);

  assert_eq!(ran.code, 0, "{}", ran.err);
  assert!(read(&dir.path().join("x/polar.toml")).contains("name = \"shop\""));
}

#[test]
fn invalid_name_is_rejected() {
  let dir = tempfile::tempdir().unwrap();

  let ran = polar(dir.path(), &["init", "x", "--name", "1 bad!"]);

  assert_eq!(ran.code, 1);
  assert!(ran.err.contains("--name"), "{}", ran.err);
  assert!(tree(dir.path()).is_empty());
}

#[test]
fn gitignore_ignores_build_output() {
  let dir = tempfile::tempdir().unwrap();

  polar(dir.path(), &["init", "hello"]);

  let text = read(&dir.path().join("hello/.gitignore"));

  assert!(text.contains("dist/") && text.contains(".polar/"), "{text}");
}

#[test]
fn zone_writes_plugin_files() {
  let dir = tempfile::tempdir().unwrap();

  let ran = polar(dir.path(), &["init", "app", "--zone", "notes"]);

  assert_eq!(ran.code, 0, "{}", ran.err);

  let app = dir.path().join("app");

  assert!(read(&app.join("polar.toml")).contains("zones = [\"notes\"]"));
  assert!(read(&app.join("plugin/lib.rs")).contains("keyword: \"notes\""));
  assert!(app.join(".polar/api/src/lib.rs").is_file());
  assert!(app.join(".polar/plugin/Cargo.toml").is_file());
  assert!(ran.out.contains("plugin/lib.rs"), "{}", ran.out);
}

#[test]
fn two_zones() {
  let dir = tempfile::tempdir().unwrap();

  let ran =
    polar(dir.path(), &["init", "app", "--zone", "notes", "--zone", "tags"]);

  assert_eq!(ran.code, 0, "{}", ran.err);

  let app = dir.path().join("app");
  let lib = read(&app.join("plugin/lib.rs"));

  assert!(
    read(&app.join("polar.toml")).contains("zones = [\"notes\", \"tags\"]")
  );
  assert!(lib.contains("struct Notes") && lib.contains("struct Tags"), "{lib}");
}

#[test]
fn zone_must_be_lowercase() {
  let dir = tempfile::tempdir().unwrap();

  let ran = polar(dir.path(), &["init", "app", "--zone", "Notes"]);

  assert_eq!(ran.code, 1);
  assert!(tree(dir.path()).is_empty());
}

#[test]
fn zone_not_builtin() {
  let dir = tempfile::tempdir().unwrap();

  let ran = polar(dir.path(), &["init", "app", "--zone", "functions"]);

  assert_eq!(ran.code, 1);
  assert!(ran.err.contains("already a Polar keyword"), "{}", ran.err);
  assert!(tree(dir.path()).is_empty());
}

#[test]
fn zone_not_twice() {
  let dir = tempfile::tempdir().unwrap();

  let ran = polar(dir.path(), &["init", "app", "--zone", "a", "--zone", "a"]);

  assert_eq!(ran.code, 1);
  assert!(ran.err.contains("given twice"), "{}", ran.err);
  assert!(tree(dir.path()).is_empty());
}

#[test]
fn zone_project_checks() {
  let dir = tempfile::tempdir().unwrap();

  let ran = polar(dir.path(), &["init", "app", "--zone", "notes"]);

  assert_eq!(ran.code, 0, "{}", ran.err);

  let ran = polar(&dir.path().join("app"), &["check"]);

  assert_eq!(ran.code, 0, "{}", ran.err);
}

#[test]
fn help_lists_init() {
  let ran = polar(Path::new("."), &["--help"]);

  assert_eq!(ran.code, 0);
  assert!(ran.out.contains("init"), "{}", ran.out);
}
