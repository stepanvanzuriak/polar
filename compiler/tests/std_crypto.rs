mod common;

use std::path::PathBuf;

fn fixture() -> PathBuf {
  PathBuf::from(env!("CARGO_MANIFEST_DIR"))
    .join("tests/fixtures/programs/std_crypto.px")
}

#[test]
fn std_crypto_program_runs_as_expected() {
  let path = fixture();
  let src = std::fs::read_to_string(&path).expect("read std_crypto.px");
  let expected = std::fs::read_to_string(path.with_extension("expected.txt"))
    .expect("read std_crypto.expected.txt");
  let out =
    common::node::run_program_with(&src, "test.px", &[], Some("Node"), &[])
      .unwrap_or_else(|err| panic!("{err}"));

  assert_eq!(out, expected);
}

#[test]
fn program_can_bind_random() {
  let src = "module App

uses
  Std.Crypto

hosts
  Node

binds
  Random in Node {
    bytes(count) {
      \"00ff\"
    }
  }

functions
  main() -> {} / {Random} {
    Log.info(Random.bytes(2))
  }

exports
  main
";
  let out =
    common::node::run_program_with(src, "app.px", &[], Some("Node"), &[])
      .unwrap_or_else(|err| panic!("{err}"));

  assert_eq!(out, "00ff\n");
}

#[test]
fn std_crypto_is_rejected_in_the_browser_host() {
  use polar_compiler::{
    CompileOptions, HostOption, compile, shared::diagnostic::Severity,
  };

  let src = "module App

uses
  Std.Crypto

hosts
  Browser

functions
  main() -> {} / {Random} {
    Log.info(Crypto.base64url(Random.bytes(32)))
  }

exports
  main
";
  let out = compile(
    src,
    "app.px",
    &CompileOptions {
      host: HostOption::Fixed(Some("Browser".to_string())),
      ..CompileOptions::default()
    },
  );
  let errors: Vec<_> =
    out.diagnostics.iter().filter(|d| d.severity == Severity::Error).collect();

  assert_eq!(errors.len(), 1, "{errors:#?}");
  assert!(errors[0].message.contains("Browser"), "{errors:#?}");
}

fn run_password_fixture() -> (String, String) {
  let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
    .join("tests/fixtures/programs/std_crypto_password.px");
  let src = std::fs::read_to_string(&path).expect("read fixture");
  let expected = std::fs::read_to_string(path.with_extension("expected.txt"))
    .expect("read expected");
  let out =
    common::node::run_program_with(&src, "test.px", &[], Some("Node"), &[])
      .unwrap_or_else(|err| panic!("{err}"));

  (out, expected)
}

#[test]
fn std_crypto_password_program_runs_as_expected() {
  let (out, expected) = run_password_fixture();

  assert_eq!(out, expected);
}

#[test]
fn std_crypto_password_cost_cap_is_fast() {
  let start = std::time::Instant::now();
  let (out, _) = run_password_fixture();

  assert!(out.contains("cost_cap: false"), "{out}");
  assert!(start.elapsed() < std::time::Duration::from_secs(10));
}
