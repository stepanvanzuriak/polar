use std::{fmt::Write as _, fs, path::Path, process::Command};

const POLAR: &str = env!("CARGO_BIN_EXE_polar");

struct Ran {
  code: Option<i32>,
  out: String,
  err: String,
}

fn polar(cwd: &Path, args: &[&str]) -> Ran {
  let tmp = tempfile::tempdir().unwrap();
  let out = Command::new(POLAR)
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

fn write(root: &Path, files: &[(&str, &str)]) {
  for (path, text) in files {
    let full = root.join(path);

    fs::create_dir_all(full.parent().unwrap()).unwrap();
    fs::write(full, text).unwrap();
  }
}

fn project(name: &str) -> String {
  format!("[project]\nname = \"{name}\"\n")
}

fn tests(names: &[(&str, &str)]) -> String {
  let mut functions = String::new();
  let mut exports = String::new();

  for (name, body) in names {
    let _ = write!(
      functions,
      "  {name}() -> {{}} / {{Throws<Failed>}} {{\n    {body}\n  }}\n\n"
    );
    let _ = writeln!(exports, "  {name}");
  }

  format!(
    "module MathTest\n\nuses\n  Std.Assert\n\nfunctions\n{functions}exports\n{exports}"
  )
}

fn passing() -> String {
  tests(&[("test_add", "Assert.equal(1 + 2, 3)")])
}

fn failing() -> String {
  tests(&[("test_sub", "Assert.equal(1, 2)")])
}

fn main_px() -> &'static str {
  "module Main\n\nfunctions\n  main() {\n    1\n  }\n\nexports\n  main\n"
}

fn workspace(config: &str, members: &[(&str, &str)]) -> tempfile::TempDir {
  let dir = tempfile::tempdir().unwrap();

  write(dir.path(), &[("polar.toml", config)]);

  for (name, test) in members {
    let toml = project(name);

    write(
      dir.path(),
      &[
        (&format!("tests/{name}/polar.toml"), toml.as_str()),
        (&format!("tests/{name}/src/main.px"), main_px()),
        (&format!("tests/{name}/src/math_test.px"), test),
      ],
    );
  }

  dir
}

const ALL: &str = "[workspace]\nmembers = [\"tests/*\"]\n";

#[test]
fn runs_every_member_and_sums_the_results() {
  let (a, b) = (passing(), passing());
  let dir = workspace(ALL, &[("alpha", &a), ("beta", &b)]);
  let ran = polar(dir.path(), &["test"]);

  assert_eq!(ran.code, Some(0), "{}\n{}", ran.out, ran.err);
  assert!(ran.out.contains("project tests/alpha"), "{}", ran.out);
  assert!(ran.out.contains("project tests/beta"), "{}", ran.out);
  assert!(
    ran.out.contains("workspace result: ok. 2 projects; 2 passed; 0 failed"),
    "{}",
    ran.out
  );
  assert!(
    ran.out.find("tests/alpha").unwrap() < ran.out.find("tests/beta").unwrap()
  );
}

#[test]
fn a_failing_member_fails_the_run_and_others_still_run() {
  let (a, b) = (failing(), passing());
  let dir = workspace(ALL, &[("alpha", &a), ("beta", &b)]);
  let ran = polar(dir.path(), &["test"]);

  assert_eq!(ran.code, Some(1), "{}\n{}", ran.out, ran.err);
  assert!(ran.out.contains("test math_test::test_sub ... FAILED"));
  assert!(ran.out.contains("test math_test::test_add ... ok"));
  assert!(
    ran
      .out
      .contains("workspace result: FAILED. 2 projects; 1 passed; 1 failed"),
    "{}",
    ran.out
  );
}

#[test]
fn a_build_error_in_one_member_fails_only_that_member() {
  let (a, b) = (passing(), passing());
  let dir = workspace(ALL, &[("alpha", &a), ("beta", &b)]);

  write(dir.path(), &[("tests/alpha/src/main.px", "module Main\n\nbroken(")]);

  let ran = polar(dir.path(), &["test"]);

  assert_eq!(ran.code, Some(1), "{}\n{}", ran.out, ran.err);
  assert!(ran.out.contains("test math_test::test_add ... ok"), "{}", ran.out);
  assert!(ran.out.contains("workspace result: FAILED"), "{}", ran.out);
}

#[test]
fn project_flag_picks_members() {
  let (a, b) = (failing(), passing());
  let dir = workspace(ALL, &[("alpha", &a), ("beta", &b)]);
  let ran = polar(dir.path(), &["test", "-p", "beta"]);

  assert_eq!(ran.code, Some(0), "{}\n{}", ran.out, ran.err);
  assert!(!ran.out.contains("tests/alpha"), "{}", ran.out);
  assert!(ran.out.contains("1 project;"), "{}", ran.out);
}

#[test]
fn unknown_project_is_an_error() {
  let a = passing();
  let dir = workspace(ALL, &[("alpha", &a)]);
  let ran = polar(dir.path(), &["test", "-p", "nope"]);

  assert_eq!(ran.code, Some(1));
  assert!(ran.err.contains("no project `nope`"), "{}", ran.err);
}

#[test]
fn project_flag_outside_a_workspace_is_an_error() {
  let dir = tempfile::tempdir().unwrap();

  write(dir.path(), &[("polar.toml", &project("solo"))]);

  let ran = polar(dir.path(), &["test", "-p", "solo"]);

  assert_eq!(ran.code, Some(1));
  assert!(ran.err.contains("not one"), "{}", ran.err);
}

#[test]
fn jobs_print_in_member_order() {
  let (a, b, c) = (passing(), passing(), passing());
  let dir = workspace(ALL, &[("alpha", &a), ("beta", &b), ("gamma", &c)]);
  let ran = polar(dir.path(), &["test", "--jobs", "3"]);

  assert_eq!(ran.code, Some(0), "{}\n{}", ran.out, ran.err);

  let at = |name: &str| ran.out.find(name).unwrap();

  assert!(at("tests/alpha") < at("tests/beta"));
  assert!(at("tests/beta") < at("tests/gamma"));
  assert!(ran.out.contains("3 projects; 3 passed"), "{}", ran.out);
}

#[test]
fn exclude_and_members_without_tests_are_skipped() {
  let (a, b) = (passing(), passing());
  let config = "[workspace]\nmembers = [\"tests/*\", \"tools\"]\nexclude = [\"tests/beta\"]\n";
  let dir = workspace(config, &[("alpha", &a), ("beta", &b)]);

  write(
    dir.path(),
    &[
      ("tools/polar.toml", &project("tools")),
      ("tools/src/main.px", main_px()),
    ],
  );

  let ran = polar(dir.path(), &["test"]);

  assert_eq!(ran.code, Some(0), "{}\n{}", ran.out, ran.err);
  assert!(ran.out.contains("tests/alpha"), "{}", ran.out);
  assert!(!ran.out.contains("tests/beta"), "{}", ran.out);
  assert!(!ran.out.contains("project tools"), "{}", ran.out);
  assert!(ran.out.contains("1 project;"), "{}", ran.out);
}

#[test]
fn check_at_the_root_checks_every_member() {
  let (a, b) = (passing(), passing());
  let dir = workspace(ALL, &[("alpha", &a), ("beta", &b)]);

  assert_eq!(polar(dir.path(), &["check"]).code, Some(0));

  write(dir.path(), &[("tests/beta/src/main.px", "module Main\n\nbroken(")]);

  let ran = polar(dir.path(), &["check"]);

  assert_eq!(ran.code, Some(1), "{}", ran.err);
  assert!(ran.err.contains("tests/beta/src/main.px"), "{}", ran.err);
}

#[test]
fn a_workspace_cannot_also_be_a_project() {
  let dir = tempfile::tempdir().unwrap();

  write(
    dir.path(),
    &[("polar.toml", "[project]\nname = \"p\"\n\n[workspace]\nmembers = []\n")],
  );

  let ran = polar(dir.path(), &["check", "."]);

  assert_eq!(ran.code, Some(1));
  assert!(ran.err.contains("only lists its members"), "{}", ran.err);
}

#[test]
fn a_member_needs_a_manifest() {
  let dir = tempfile::tempdir().unwrap();

  write(
    dir.path(),
    &[("polar.toml", "[workspace]\nmembers = [\"missing\"]\n")],
  );

  let ran = polar(dir.path(), &["test"]);

  assert_eq!(ran.code, Some(1));
  assert!(ran.err.contains("`missing` has no polar.toml"), "{}", ran.err);
}

#[test]
fn only_a_trailing_star_is_supported() {
  let dir = tempfile::tempdir().unwrap();

  write(dir.path(), &[("polar.toml", "[workspace]\nmembers = [\"a*/b\"]\n")]);

  let ran = polar(dir.path(), &["test"]);

  assert_eq!(ran.code, Some(1));
  assert!(ran.err.contains("last component"), "{}", ran.err);
}
