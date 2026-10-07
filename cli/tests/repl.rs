use std::{
  fs,
  io::Write,
  path::Path,
  process::{Command, Stdio},
};

const POLAR: &str = env!("CARGO_BIN_EXE_polar");

struct Ran {
  code: Option<i32>,
  out: String,
  err: String,
}

fn repl(cwd: &Path, args: &[&str], input: &str) -> Ran {
  let tmp = tempfile::tempdir().unwrap();
  let mut child = Command::new(POLAR)
    .arg("repl")
    .args(args)
    .current_dir(cwd)
    .env("TMPDIR", tmp.path())
    .env_remove("POLAR_DEBUG")
    .stdin(Stdio::piped())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .spawn()
    .expect("run polar");

  child.stdin.take().unwrap().write_all(input.as_bytes()).unwrap();

  let out = child.wait_with_output().unwrap();

  Ran {
    code: out.status.code(),
    out: String::from_utf8_lossy(&out.stdout).into_owned(),
    err: String::from_utf8_lossy(&out.stderr).into_owned(),
  }
}

fn project(files: &[(&str, &str)]) -> tempfile::TempDir {
  let dir = tempfile::tempdir().unwrap();

  fs::write(
    dir.path().join("polar.toml"),
    "[project]\nname = \"t\"\nhosts = [\"Node\"]\n",
  )
  .unwrap();

  for (path, text) in files {
    let full = dir.path().join(path);

    fs::create_dir_all(full.parent().unwrap()).unwrap();
    fs::write(full, text).unwrap();
  }

  dir
}

const GREET: &str = "module Greet

uses
  Std.Ref

constants
  counter: Ref<Int> = Ref.new(0)

functions
  hello(name: String) -> String {
    \"hello #{name}\"
  }

  current() -> Int / {Mut} {
    Ref.get(counter)
  }

exports
  hello
  current
  counter
";

const CLOCK: &str = "module Clock

hosts
  Node

effects
  Clock in Node {
    now() -> Int
  }
  Ghost in Node {
    boo() -> Int
  }

binds
  Clock in Node {
    now() {
      1700000000
    }
  }

exports
  Node
  Clock
  Ghost
";

const MAIN: &str = "module Main

uses
  Greet
  Std.Ref

hosts
  Node

types
  Oops = Oops(String)

functions
  boot() -> {} / {Mut} {
    Ref.set(Greet.counter, 41)
  }

  fail() -> {} / {Throws<Oops>} {
    throw Oops(\"no\")
  }

exports
  Node
  boot
  fail
";

fn fixture() -> tempfile::TempDir {
  project(&[
    ("src/greet.px", GREET),
    ("src/clock.px", CLOCK),
    ("src/main.px", MAIN),
  ])
}

#[test]
fn value_and_type() {
  let dir = fixture();
  let ran = repl(dir.path(), &[], "1 + 2\n");

  assert_eq!(ran.code, Some(0), "{}", ran.err);
  assert_eq!(ran.out, "Int = 3\n");
}

#[test]
fn binding_persists() {
  let dir = fixture();
  let ran = repl(dir.path(), &[], "let x = 2\nx * 21\n");

  assert_eq!(ran.code, Some(0), "{}", ran.err);
  assert_eq!(ran.out, "x: Int = 2\nInt = 42\n");
}

#[test]
fn shadowing() {
  let dir = fixture();
  let ran = repl(dir.path(), &[], "let x = 1\nlet x = \"a\"\nx\n");

  assert_eq!(ran.code, Some(0), "{}", ran.err);
  assert_eq!(ran.out, "x: Int = 1\nx: String = \"a\"\nString = \"a\"\n");
}

#[test]
fn type_error_recovers() {
  let dir = fixture();
  let ran = repl(dir.path(), &[], "let y = 5\n1 + \"a\"\ny\n1\n");

  assert_eq!(ran.code, Some(1));
  assert!(ran.err.contains("mismatched types"), "{}", ran.err);
  assert_eq!(ran.out, "y: Int = 5\nInt = 5\nInt = 1\n");
}

#[test]
fn project_function() {
  let dir = fixture();
  let ran = repl(dir.path(), &[], "Greet.hello(\"Ada\")\n");

  assert_eq!(ran.code, Some(0), "{}", ran.err);
  assert_eq!(ran.out, "String = \"hello Ada\"\n");
}

#[test]
fn effect_with_bind() {
  let dir = fixture();
  let ran = repl(dir.path(), &[], "Clock.now()\n");

  assert_eq!(ran.code, Some(0), "{}", ran.err);
  assert_eq!(ran.out, "Int = 1700000000\n");
}

#[test]
fn effect_without_bind() {
  let dir = fixture();
  let ran = repl(dir.path(), &[], "Ghost.boo()\n1\n");

  assert_eq!(ran.code, Some(1));
  assert!(ran.err.contains("`Ghost` has no binding"), "{}", ran.err);
  assert_eq!(ran.out, "Int = 1\n");
}

#[test]
fn setup_runs() {
  let dir = fixture();
  let ran = repl(dir.path(), &["--setup", "Main.boot"], "Greet.current()\n");

  assert_eq!(ran.code, Some(0), "{}", ran.err);
  assert_eq!(ran.out, "Int = 41\n");
}

#[test]
fn setup_fails() {
  let dir = fixture();
  let ran = repl(dir.path(), &["--setup", "Main.fail"], "1\n");

  assert_eq!(ran.code, Some(1));
  assert_eq!(ran.out, "");
  assert!(ran.err.contains("--setup Main.fail"), "{}", ran.err);
}

#[test]
fn meta_commands() {
  let dir = fixture();
  let ran = repl(dir.path(), &[], ":type Greet.hello\n:quit\n1\n");

  assert_eq!(ran.code, Some(0), "{}", ran.err);
  assert_eq!(ran.out, "function(String) -> String\n");
}

#[test]
fn piped() {
  let dir = fixture();
  let piped = repl(dir.path(), &[], "1+1\n");
  let flag = repl(dir.path(), &["-e", "1+1"], "");

  assert_eq!(piped.code, Some(0), "{}", piped.err);
  assert_eq!(piped.out, "Int = 2\n");
  assert_eq!(flag.out, piped.out);
  assert_eq!(flag.code, Some(0));
}

fn module_source(name: &str) -> String {
  format!(
    "module {name}\n\nfunctions\n  f() -> Int {{\n    7\n  }}\n\nexports\n  f\n"
  )
}

#[test]
fn folder_module() {
  let dir = project(&[(
    "src/controllers/pages_controller.px",
    &module_source("PagesController"),
  )]);
  let ran = repl(dir.path(), &["-e", "PagesController.f()"], "");

  assert_eq!(ran.code, Some(0), "{}", ran.err);
  assert!(ran.out.contains("Int = 7"), "{}", ran.out);
}

#[test]
fn nested_folders() {
  let dir = project(&[("src/a/b/c.px", &module_source("C"))]);
  let ran = repl(dir.path(), &["-e", "C.f()"], "");

  assert_eq!(ran.code, Some(0), "{}", ran.err);
  assert!(ran.out.contains("Int = 7"), "{}", ran.out);
}

#[test]
fn same_name_twice() {
  let dir = project(&[
    ("src/a/x.px", &module_source("X")),
    ("src/b/x.px", &module_source("X")),
  ]);
  let ran = repl(dir.path(), &["-e", "1"], "");

  assert_ne!(ran.code, Some(0));
  assert!(
    ran.err.contains("a/x.px") && ran.err.contains("b/x.px"),
    "{}",
    ran.err
  );
  assert_eq!(ran.err.matches("two modules").count(), 1, "{}", ran.err);
}
