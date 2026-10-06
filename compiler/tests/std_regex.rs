mod common;

use std::path::PathBuf;

fn fixture() -> PathBuf {
  PathBuf::from(env!("CARGO_MANIFEST_DIR"))
    .join("tests/fixtures/programs/std_regex.px")
}

#[test]
fn std_regex_program_runs_as_expected() {
  let path = fixture();
  let src = std::fs::read_to_string(&path).expect("read std_regex.px");
  let expected = std::fs::read_to_string(path.with_extension("expected.txt"))
    .expect("read std_regex.expected.txt");
  let out =
    common::node::run_program_with(&src, "test.px", &[], Some("Node"), &[])
      .unwrap_or_else(|err| panic!("{err}"));

  assert_eq!(out, expected);
}

#[test]
fn std_regex_builds_in_the_browser_host() {
  use polar_compiler::{
    CompileOptions, HostOption, compile, shared::diagnostic::Severity,
  };

  let src = "module App

uses
  Std.Regex

hosts
  Browser

functions
  is_slug(text: String) -> Bool {
    Regex.matches(\"^[a-z0-9-]+$\", text)
  }

exports
  is_slug
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
  assert!(out.std_imports.contains(&"Regex".to_string()));
}
