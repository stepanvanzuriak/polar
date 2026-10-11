use std::{fs, path::Path, process::Command};

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
    .env_remove("POLAR_UPDATE")
    .output()
    .expect("run polar");

  Ran {
    code: out.status.code(),
    out: String::from_utf8_lossy(&out.stdout).into_owned(),
    err: String::from_utf8_lossy(&out.stderr).into_owned(),
  }
}

fn project(files: &[(&str, &str)]) -> tempfile::TempDir {
  let dir = tempfile::tempdir().unwrap();

  for (path, text) in files {
    let full = dir.path().join(path);

    fs::create_dir_all(full.parent().unwrap()).unwrap();
    fs::write(full, text).unwrap();
  }

  dir
}

const TOML: (&str, &str) = ("polar.toml", "[project]\nname = \"t\"\n");
const MAIN: (&str, &str) = (
  "src/main.px",
  "module Main\n\nfunctions\n  main() {\n    1\n  }\n\nexports\n  main\n",
);

const FAILS: &str = "module Bad\n\nfunctions\n  f() -> Int {\n    // expect-error: POLAR0501 \"expected `Int`\"\n    \"x\"\n  }\n\nexports\n  f\n";

#[test]
fn expect_error_file_passes_as_a_test_and_stays_out_of_the_build() {
  let dir = project(&[TOML, MAIN, ("src/bad.px", FAILS)]);
  let ran = polar(dir.path(), &["test"]);

  assert_eq!(ran.code, Some(0), "{}\n{}", ran.out, ran.err);
  assert!(ran.out.contains("test bad ... ok"), "{}", ran.out);
  assert_eq!(polar(dir.path(), &["build"]).code, Some(0));
  assert_eq!(polar(dir.path(), &["check"]).code, Some(0));
}

#[test]
fn expect_error_fails_when_the_file_compiles() {
  let text = FAILS.replace("\"x\"", "1");
  let dir = project(&[TOML, MAIN, ("src/bad.px", &text)]);
  let ran = polar(dir.path(), &["test"]);

  assert_eq!(ran.code, Some(1), "{}\n{}", ran.out, ran.err);
  assert!(ran.out.contains("test bad ... FAILED"), "{}", ran.out);
  assert!(ran.out.contains("it was not reported"), "{}", ran.out);
}

#[test]
fn expect_error_fails_on_the_wrong_line_or_an_extra_error() {
  let moved = FAILS.replace(
    "    // expect-error: POLAR0501 \"expected `Int`\"\n    \"x\"",
    "    \"x\"",
  );
  let moved = format!("// expect-error: POLAR0501\n{moved}");
  let dir = project(&[TOML, MAIN, ("src/bad.px", &moved)]);
  let ran = polar(dir.path(), &["test"]);

  assert_eq!(ran.code, Some(1), "{}\n{}", ran.out, ran.err);
  assert!(ran.out.contains("unexpected POLAR0501"), "{}", ran.out);
}

#[test]
fn bless_writes_the_expectations() {
  let raw = FAILS
    .replace("    // expect-error: POLAR0501 \"expected `Int`\"\n", "")
    .replacen("module Bad", "// expect-error: POLAR9999\nmodule Bad", 1);
  let dir = project(&[TOML, MAIN, ("src/bad.px", &raw)]);

  assert_eq!(polar(dir.path(), &["test"]).code, Some(1));

  let blessed = polar(dir.path(), &["test", "--bless"]);

  assert_eq!(blessed.code, Some(0), "{}\n{}", blessed.out, blessed.err);

  let text = fs::read_to_string(dir.path().join("src/bad.px")).unwrap();

  assert!(text.contains("// expect-error: POLAR0501 \"mismatched types\""));
  assert!(!text.contains("POLAR9999"));
  assert_eq!(polar(dir.path(), &["test"]).code, Some(0));
}

fn test_file(body: &str) -> String {
  format!(
    "module MathTest\n\nuses\n  Std.Assert\n  Std.Test\n\nfunctions\n{body}"
  )
}

#[test]
fn unexported_tests_still_run() {
  let text = test_file(
    "  test_a() -> {} / {Throws<Failed>} {\n    Assert.equal(1, 1)\n  }\n",
  );
  let dir = project(&[TOML, MAIN, ("src/math_test.px", &text)]);
  let ran = polar(dir.path(), &["test"]);

  assert_eq!(ran.code, Some(0), "{}\n{}", ran.out, ran.err);
  assert!(ran.out.contains("test math_test::test_a ... ok"), "{}", ran.out);
}

#[test]
fn capture_returns_what_was_logged() {
  let text = test_file(
    "  test_c() -> {} / {Async, Throws<Failed>} {\n    let got = Test.capture(function() {\n      Log.info(\"hi\")\n      7\n    })\n\n    Assert.equal(got.output, \"hi\\n\")\n    Assert.equal(got.value, 7)\n  }\n",
  );
  let dir = project(&[TOML, MAIN, ("src/math_test.px", &text)]);
  let ran = polar(dir.path(), &["test"]);

  assert_eq!(ran.code, Some(0), "{}\n{}", ran.out, ran.err);
  assert!(!ran.out.contains("hi\n"), "{}", ran.out);
}

#[test]
fn stderr_of_passing_tests_is_hidden() {
  let text = test_file(
    "  test_e() -> {} / {Throws<Failed>} {\n    Log.error(\"noisy\")\n  }\n",
  );
  let dir = project(&[TOML, MAIN, ("src/math_test.px", &text)]);
  let ran = polar(dir.path(), &["test"]);

  assert_eq!(ran.code, Some(0), "{}\n{}", ran.out, ran.err);
  assert!(!ran.out.contains("noisy") && !ran.err.contains("noisy"));
}

#[test]
fn around_each_wraps_every_test() {
  let text = test_file(
    "  around_each(run: function() -> {} / {| e}) -> {} / {| e} {\n    Log.info(\"setup\")\n    run()\n  }\n\n  test_a() -> {} / {Throws<Failed>} {\n    Assert.equal(1, 2)\n  }\n",
  );
  let dir = project(&[TOML, MAIN, ("src/math_test.px", &text)]);
  let ran = polar(dir.path(), &["test"]);

  assert_eq!(ran.code, Some(1), "{}\n{}", ran.out, ran.err);
  assert!(ran.out.contains("setup"), "{}", ran.out);
}

#[test]
fn equal_text_shows_only_changed_lines() {
  let text = test_file(
    "  test_t() -> {} / {Throws<Failed>} {\n    Assert.equal_text(\"a\\nb\\nc\", \"a\\nx\\nc\")\n  }\n",
  );
  let dir = project(&[TOML, MAIN, ("src/math_test.px", &text)]);
  let ran = polar(dir.path(), &["test"]);

  assert_eq!(ran.code, Some(1), "{}\n{}", ran.out, ran.err);
  assert!(ran.out.contains("- 2: x\n+ 2: b"), "{}", ran.out);
  assert!(!ran.out.contains("- 1: a"), "{}", ran.out);
}

#[test]
fn json_format_has_one_object_per_test() {
  let text = test_file(
    "  test_a() -> {} / {Throws<Failed>} {\n    Assert.equal(1, 1)\n  }\n",
  );
  let dir = project(&[TOML, MAIN, ("src/math_test.px", &text)]);
  let ran = polar(dir.path(), &["test", "--format", "json"]);

  assert_eq!(ran.code, Some(0), "{}\n{}", ran.out, ran.err);

  let lines: Vec<serde_json::Value> =
    ran.out.lines().map(|l| serde_json::from_str(l).unwrap()).collect();

  assert_eq!(lines.len(), 2, "{}", ran.out);
  assert_eq!(lines[0]["event"], "test");
  assert_eq!(lines[0]["outcome"], "pass");
  assert_eq!(lines[0]["file"], "src/math_test.px");
  assert_eq!(lines[1]["event"], "result");
  assert_eq!(lines[1]["passed"], 1);
}

const MESSY: &str =
  "module  M\n\nfunctions\n  f()   ->  Int {\n 1\n  }\n\nexports\n  f\n";

#[test]
fn fmt_expect_compares_and_fmt_idempotent_passes() {
  let dir = project(&[("messy.px", MESSY)]);
  let formatted = {
    let copy = project(&[("m.px", MESSY)]);

    polar(copy.path(), &["fmt", "m.px"]);
    fs::read_to_string(copy.path().join("m.px")).unwrap()
  };

  fs::write(dir.path().join("messy.expected.px"), &formatted).unwrap();

  let ok =
    polar(dir.path(), &["fmt", "--expect", "messy.expected.px", "messy.px"]);

  assert_eq!(ok.code, Some(0), "{}", ok.err);
  assert_eq!(fs::read_to_string(dir.path().join("messy.px")).unwrap(), MESSY);

  fs::write(dir.path().join("messy.expected.px"), MESSY).unwrap();

  let bad =
    polar(dir.path(), &["fmt", "--expect", "messy.expected.px", "messy.px"]);

  assert_eq!(bad.code, Some(1));
  assert!(bad.err.contains("differs from messy.expected.px"), "{}", bad.err);

  let idem = polar(dir.path(), &["fmt", "--check", "--idempotent", "messy.px"]);

  assert_eq!(idem.code, Some(1), "{}", idem.err);
  assert!(!idem.err.contains("not idempotent"), "{}", idem.err);
}

#[test]
fn fmt_skips_fixtures_with_an_expected_sibling_and_excluded_paths() {
  let dir = project(&[
    (
      "polar.toml",
      "[project]\nname = \"t\"\n\n[fmt]\nexclude = [\"src/skip\"]\n",
    ),
    ("src/messy.px", MESSY),
    ("src/messy.expected.px", "module M\n"),
    ("src/skip/other.px", MESSY),
  ]);
  let ran = polar(dir.path(), &["fmt"]);

  assert_eq!(ran.code, Some(0), "{}", ran.err);

  let read = |p: &str| fs::read_to_string(dir.path().join(p)).unwrap();

  assert_eq!(read("src/messy.px"), MESSY);
  assert_eq!(read("src/skip/other.px"), MESSY);
  assert_eq!(read("src/messy.expected.px"), "module M\n");
}

#[test]
fn algo_diff_keeps_shared_elements_and_indexes_changes() {
  let text = "module MathTest\n\nuses\n  Std.Algo\n  Std.Assert\n  Std.List\n\nfunctions\n  test_diff() -> {} / {Throws<Failed>} {\n    let edits = Algo.diff([1, 2, 3, 4], [1, 3, 5, 4])\n    let kept = List.length(List.filter(edits, function(e) {\n      match e {\n        Keep(_) -> true,\n        _ -> false,\n      }\n    }))\n\n    Assert.equal(List.length(edits), 5)\n    Assert.equal(kept, 3)\n  }\n";
  let dir = project(&[TOML, MAIN, ("src/math_test.px", text)]);
  let ran = polar(dir.path(), &["test"]);

  assert_eq!(ran.code, Some(0), "{}\n{}", ran.out, ran.err);
}
