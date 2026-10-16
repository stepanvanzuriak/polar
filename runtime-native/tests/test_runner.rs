use std::{fs, path::Path};

struct Ran {
  code: u8,
  out: String,
  err: String,
}

fn polar_test(dir: &Path, env: &[(&str, &str)]) -> Ran {
  let args: Vec<std::ffi::OsString> =
    ["test", "--timeout", "500"].into_iter().map(Into::into).collect();
  let rt = env!("CARGO_BIN_EXE_polar-rt").to_string();
  let extra: Vec<(String, String)> =
    env.iter().map(|(k, v)| ((*k).to_string(), (*v).to_string())).collect();
  let lookup = move |name: &str| match name {
    "POLAR_RT" => Some(rt.clone()),
    other => extra
      .iter()
      .find(|(k, _)| k == other)
      .map(|(_, v)| v.clone())
      .or_else(|| std::env::var(other).ok()),
  };
  let (mut out, mut err) = (Vec::new(), Vec::new());
  let mut io = polar_cli::Io {
    out: &mut out,
    err: &mut err,
    stdout_is_tty: false,
    stderr_is_tty: false,
    cwd: dir.to_path_buf(),
    env: &lookup,
  };
  let code = polar_cli::run(&args, &mut io);

  Ran {
    code,
    out: String::from_utf8_lossy(&out).into_owned(),
    err: String::from_utf8_lossy(&err).into_owned(),
  }
}

fn project(files: &[(&str, &str)]) -> tempfile::TempDir {
  project_with("[project]\nname = \"t\"\nhosts = [\"Native\"]\n", files)
}

fn project_with(config: &str, files: &[(&str, &str)]) -> tempfile::TempDir {
  let dir = tempfile::tempdir().unwrap();

  fs::write(dir.path().join("polar.toml"), config).unwrap();

  for (path, text) in files {
    let full = dir.path().join(path);

    fs::create_dir_all(full.parent().unwrap()).unwrap();
    fs::write(full, text).unwrap();
  }

  dir
}

const GREET: &str = r#"module Greet

hosts
  Native

effects
  Word in Native {
    word() -> String
  }

binds
  Word in Native {
    word() {
      "real"
    }
  }

functions
  hello() -> {} / {Word} {
    Log.info("hello #{Word.word()}")
  }

  spin(n: Int) -> Int {
    if n < 0 { 0 } else { spin(n + 1) }
  }

exports
  hello
  spin
  Native
  Word
"#;

const TESTS: &str = r##"module GreetTest

uses
  Greet
  Std.Assert
  Std.Test

hosts
  Native

functions
  test_passes() -> {} / {Throws<Failed>} {
    Assert.equal(1, 1)
  }

  test_fails() -> {} / {Throws<Failed>} {
    Assert.equal(1, 2)
  }

  test_capture_and_mock() -> {} / {..} {
    let real = Test.capture(function() { Greet.hello() })
    let fake = Test.mock(
      Word,
      { word: function() { "fake" } },
      function() { Test.capture(function() { Greet.hello() }) },
    )

    Assert.equal(real.output, "hello real\n")
    Assert.equal(fake.output, "hello fake\n")
  }

  test_snapshot() -> {} / {..} {
    Test.assert_snapshot("greet", "hi\n")
  }

  test_spins() {
    Log.info("#{Greet.spin(0)}")
  }
"##;

#[test]
fn native_tests_run_and_report() {
  let dir = project(&[
    ("src/greet.px", GREET),
    ("src/greet_test.px", TESTS),
    ("snapshots/greet.txt", "hi\n"),
  ]);
  let ran = polar_test(dir.path(), &[]);

  assert_eq!(ran.code, 1, "{}{}", ran.out, ran.err);

  for line in [
    "test greet_test::test_passes ... ok",
    "test greet_test::test_fails ... FAILED",
    "test greet_test::test_capture_and_mock ... ok",
    "test greet_test::test_snapshot ... ok",
    "test greet_test::test_spins ... TIMEOUT",
    "expected 2, got 1",
    "3 passed; 2 failed",
  ] {
    assert!(ran.out.contains(line), "{line}\n{}{}", ran.out, ran.err);
  }

  assert!(dir.path().join(".polar/test/host/Native/tests").is_file());
}

#[test]
fn native_snapshot_prefers_the_host_file() {
  let test = "module SnapTest\n\nuses\n  Std.Test\n\nfunctions\n  test_snap() -> {} / {..} {\n    Test.assert_snapshot(\"out\", \"native\\n\")\n  }\n";
  let dir = project(&[
    ("src/snap_test.px", test),
    ("snapshots/out.txt", "shared\n"),
    ("snapshots/out.Native.txt", "native\n"),
  ]);
  let ran = polar_test(dir.path(), &[]);

  assert_eq!(ran.code, 0, "{}{}", ran.out, ran.err);
}

#[test]
fn native_snapshot_update_writes_the_file() {
  let test = "module SnapTest\n\nuses\n  Std.Test\n\nfunctions\n  test_snap() -> {} / {..} {\n    Test.assert_snapshot(\"out\", \"native\\n\")\n  }\n";
  let dir = project(&[("src/snap_test.px", test)]);
  let ran = polar_test(dir.path(), &[]);

  assert_eq!(ran.code, 1, "{}", ran.out);
  assert!(ran.out.contains("no snapshot at snapshots/out.txt"), "{}", ran.out);

  let ran = polar_test(dir.path(), &[("POLAR_UPDATE", "1")]);

  assert_eq!(ran.code, 0, "{}{}", ran.out, ran.err);
  assert_eq!(
    fs::read_to_string(dir.path().join("snapshots/out.txt")).unwrap(),
    "native\n"
  );
}

#[test]
fn node_only_helpers_are_rejected_on_native() {
  let test = "module AppTest\n\nuses\n  Std.Test\n\nfunctions\n  test_app() -> {} / {..} {\n    Test.with_app(function(app) { {} })\n  }\n";
  let dir = project(&[("src/app_test.px", test)]);
  let ran = polar_test(dir.path(), &[]);

  assert_eq!(ran.code, 1, "{}", ran.out);
  assert!(ran.err.contains("Net"), "{}", ran.err);
}

#[test]
fn one_file_runs_on_node_and_native() {
  let test = "module SumTest\n\nuses\n  Std.Assert\n\nhosts\n  Native\n  Node\n\nfunctions\n  test_sum() -> {} / {Throws<Failed>} {\n    Assert.equal(2 + 2, 4)\n  }\n";
  let dir = project_with(
    "[project]\nname = \"t\"\nhosts = [\"Native\", \"Node\"]\n",
    &[("src/sum_test.px", test)],
  );
  let ran = polar_test(dir.path(), &[]);

  assert_eq!(ran.code, 0, "{}{}", ran.out, ran.err);
  assert!(ran.out.contains("sum_test::test_sum (Node) ... ok"), "{}", ran.out);
  assert!(
    ran.out.contains("sum_test::test_sum (Native) ... ok"),
    "{}",
    ran.out
  );
}
