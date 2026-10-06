use std::{
  fmt::Write as _,
  fs,
  path::Path,
  process::{Command, Output},
};

const POLAR: &str = env!("CARGO_BIN_EXE_polar");

struct Ran {
  code: Option<i32>,
  out: String,
  err: String,
}

fn polar(cwd: &Path, args: &[&str]) -> Ran {
  let tmp = tempfile::tempdir().unwrap();
  let out: Output = Command::new(POLAR)
    .args(args)
    .current_dir(cwd)
    .env("TMPDIR", tmp.path())
    .env_remove("POLAR_DEBUG")
    .output()
    .expect("run polar");

  Ran {
    code: out.status.code(),
    out: String::from_utf8_lossy(&out.stdout).into_owned(),
    err: String::from_utf8_lossy(&out.stderr).into_owned(),
  }
}

fn test_module(
  module: &str,
  tests: &[(&str, &str)],
  exported: &[&str],
) -> String {
  let mut functions = String::new();
  let mut exports = String::new();

  for (name, body) in tests {
    let _ = write!(
      functions,
      "  {name}() -> {{}} / {{Throws<Failed>}} {{\n    {body}\n  }}\n\n"
    );
  }

  for name in exported {
    let _ = writeln!(exports, "  {name}");
  }

  format!(
    "module {module}\n\nuses\n  Std.Assert\n\nfunctions\n{functions}exports\n{exports}"
  )
}

fn project(config: &str, files: &[(&str, &str)]) -> tempfile::TempDir {
  let dir = tempfile::tempdir().unwrap();

  fs::write(dir.path().join("polar.toml"), config).unwrap();

  for (path, text) in files {
    let full = dir.path().join(path);

    fs::create_dir_all(full.parent().unwrap()).unwrap();
    fs::write(full, text).unwrap();
  }

  dir
}

const CONFIG: &str = "[project]\nname = \"t\"\n";

fn passing() -> String {
  test_module(
    "MathTest",
    &[("test_add", "Assert.equal(1 + 2, 3)")],
    &["test_add"],
  )
}

fn failing() -> String {
  test_module("MathTest", &[("test_sub", "Assert.equal(1, 2)")], &["test_sub"])
}

fn two_tests() -> String {
  test_module(
    "MathTest",
    &[("test_a", "Assert.equal(1, 1)"), ("test_b", "Assert.equal(2, 2)")],
    &["test_a", "test_b"],
  )
}

#[test]
fn passing_tests_exit_0() {
  let dir = project(CONFIG, &[("src/math_test.px", &passing())]);
  let ran = polar(dir.path(), &["test"]);

  assert_eq!(ran.code, Some(0), "{}\n{}", ran.out, ran.err);
  assert!(ran.out.contains("✔ test_add"), "{}", ran.out);
}

#[test]
fn failing_test_exits_1() {
  let dir = project(CONFIG, &[("src/math_test.px", &failing())]);
  let ran = polar(dir.path(), &["test"]);

  assert_eq!(ran.code, Some(1), "{}\n{}", ran.out, ran.err);
  assert!(ran.out.contains("✖ test_sub"), "{}", ran.out);
  assert!(ran.out.contains("expected 2, got 1"), "{}", ran.out);
}

#[test]
fn failure_points_at_px() {
  let dir = project(CONFIG, &[("src/math_test.px", &failing())]);
  let ran = polar(dir.path(), &["test"]);

  assert!(
    ran.out.lines().any(|l| l.contains("math_test.px:") && l.contains("at ")),
    "{}",
    ran.out
  );
}

#[test]
fn only_test_prefixed_exports_run() {
  let text = test_module(
    "MathTest",
    &[("test_a", "Assert.equal(1, 1)"), ("helper", "Assert.equal(1, 1)")],
    &["test_a", "helper"],
  );
  let dir = project(CONFIG, &[("src/math_test.px", &text)]);
  let ran = polar(dir.path(), &["test"]);

  assert_eq!(ran.code, Some(0), "{}\n{}", ran.out, ran.err);
  assert!(ran.out.contains("test_a"), "{}", ran.out);
  assert!(!ran.out.contains("helper"), "{}", ran.out);
}

#[test]
fn finds_nested_test_files() {
  let text = test_module(
    "MoneyTest",
    &[("test_nested", "Assert.equal(1, 1)")],
    &["test_nested"],
  );
  let dir = project(CONFIG, &[("src/money/money_test.px", &text)]);
  let ran = polar(dir.path(), &["test"]);

  assert_eq!(ran.code, Some(0), "{}\n{}", ran.out, ran.err);
  assert!(ran.out.contains("✔ test_nested"), "{}", ran.out);
}

#[test]
fn test_uses_project_module() {
  let money = "module Money\n\nfunctions\n  double(n: Int) -> Int {\n    n * 2\n  }\n\nexports\n  double\n";
  let test = "module MoneyTest\n\nuses\n  Money\n  Std.Assert\n\nfunctions\n  test_double() -> {} / {Throws<Failed>} {\n    Assert.equal(Money.double(4), 8)\n  }\n\nexports\n  test_double\n";
  let dir =
    project(CONFIG, &[("src/money.px", money), ("src/money_test.px", test)]);
  let ran = polar(dir.path(), &["test"]);

  assert_eq!(ran.code, Some(0), "{}\n{}", ran.out, ran.err);
  assert!(ran.out.contains("✔ test_double"), "{}", ran.out);
}

#[test]
fn no_tests_is_ok() {
  let main = "module Main\n\nfunctions\n  main() {\n    Log.info(\"hi\")\n  }\n\nexports\n  main\n";
  let dir = project(CONFIG, &[("src/main.px", main)]);
  let ran = polar(dir.path(), &["test"]);

  assert_eq!(ran.code, Some(0), "{}\n{}", ran.out, ran.err);
  assert!(ran.out.contains("no tests found"), "{}", ran.out);
}

#[test]
fn compile_error_fails_before_node() {
  let text = test_module(
    "MathTest",
    &[("test_bad", "Assert.equal(1, \"a\")")],
    &["test_bad"],
  );
  let dir = project(CONFIG, &[("src/math_test.px", &text)]);
  let ran = polar(dir.path(), &["test"]);

  assert_eq!(ran.code, Some(1), "{}\n{}", ran.out, ran.err);
  assert!(ran.err.contains("error[POLAR"), "{}", ran.err);
  assert!(!ran.out.contains("ℹ tests"), "{}", ran.out);
}

#[test]
fn args_go_to_node() {
  let dir = project(CONFIG, &[("src/math_test.px", &two_tests())]);
  let ran = polar(dir.path(), &["test", "--", "--test-name-pattern=test_a"]);

  assert_eq!(ran.code, Some(0), "{}\n{}", ran.out, ran.err);
  assert!(ran.out.contains("✔ test_a"), "{}", ran.out);
  assert!(!ran.out.contains("✔ test_b"), "{}", ran.out);
}

#[test]
fn node_project_with_hosts() {
  let text = "module EnvTest\n\nuses\n  Std.Assert\n\nhosts\n  Node\n  Browser\n\nfunctions\n  test_args() -> {} / {Throws<Failed>} {\n    Assert.assert(true)\n  }\n\nexports\n  test_args\n";
  let dir = project(
    "[project]\nname = \"t\"\nhosts = [\"Node\", \"Browser\"]\n",
    &[("src/env_test.px", text)],
  );
  let ran = polar(dir.path(), &["test"]);

  assert_eq!(ran.code, Some(0), "{}\n{}", ran.out, ran.err);
  assert!(ran.out.contains("✔ test_args"), "{}", ran.out);
}

#[test]
fn outside_a_project() {
  let dir = tempfile::tempdir().unwrap();
  let ran = polar(dir.path(), &["test"]);

  assert_eq!(ran.code, Some(1), "{}\n{}", ran.out, ran.err);
  assert!(ran.err.contains("runs inside a project"), "{}", ran.err);
}

#[test]
fn help_lists_test() {
  let dir = tempfile::tempdir().unwrap();
  let ran = polar(dir.path(), &["--help"]);

  assert!(ran.out.contains("test"), "{}", ran.out);
}
