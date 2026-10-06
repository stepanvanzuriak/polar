mod common;

use std::path::PathBuf;

fn fixture() -> PathBuf {
  PathBuf::from(env!("CARGO_MANIFEST_DIR"))
    .join("tests/fixtures/programs/std_time.px")
}

#[test]
fn std_time_program_runs_as_expected() {
  let path = fixture();
  let src = std::fs::read_to_string(&path).expect("read std_time.px");
  let expected = std::fs::read_to_string(path.with_extension("expected.txt"))
    .expect("read std_time.expected.txt");
  let out =
    common::node::run_program_with(&src, "test.px", &[], Some("Node"), &[])
      .unwrap_or_else(|err| panic!("{err}"));

  assert_eq!(out, expected);
}

#[test]
fn program_can_bind_the_clock() {
  let src = "module App

uses
  Std.Time

hosts
  Node

binds
  Clock in Node {
    now() {
      Time.from_millis(0)
    }
  }

functions
  main() -> {} / {Clock} {
    Log.info(Time.to_iso(Clock.now()))
  }

exports
  main
";
  let out =
    common::node::run_program_with(src, "app.px", &[], Some("Node"), &[])
      .unwrap_or_else(|err| panic!("{err}"));

  assert_eq!(out, "1970-01-01T00:00:00.000Z\n");
}

#[test]
fn std_time_runs_in_the_browser_host() {
  use polar_compiler::{
    CompileOptions, HostOption, compile, shared::diagnostic::Severity,
  };

  let src = "module App

uses
  Std.Time

hosts
  Browser

functions
  stamp() -> String / {Clock} {
    Time.compact(Clock.now())
  }

exports
  stamp
";
  let out = compile(
    src,
    "app.px",
    &CompileOptions {
      host: HostOption::Fixed(Some("Browser".to_string())),
      ..CompileOptions::default()
    },
  );

  assert!(
    out.diagnostics.iter().all(|d| d.severity != Severity::Error),
    "{:#?}",
    out.diagnostics
  );
  assert!(out.std_imports.contains(&"Time".to_string()));
}
