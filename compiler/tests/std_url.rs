mod common;

use std::path::PathBuf;

use polar_compiler::{
  CompileOptions, HostOption, Stage, compile, dump_stage_with,
  shared::diagnostic::Severity, stdlib,
};
use proptest::{prelude::*, test_runner::Config};

fn fixture() -> PathBuf {
  PathBuf::from(env!("CARGO_MANIFEST_DIR"))
    .join("tests/fixtures/programs/std_url.px")
}

fn run(src: &str) -> String {
  common::node::run_program(src, "test.px", &[])
    .unwrap_or_else(|err| panic!("{err}"))
}

#[test]
fn std_url_program_runs_as_expected() {
  let path = fixture();
  let src = std::fs::read_to_string(&path).expect("read std_url.px");
  let expected = std::fs::read_to_string(path.with_extension("expected.txt"))
    .expect("read std_url.expected.txt");

  assert_eq!(run(&src), expected);
}

#[test]
fn std_url_ships_its_binding() {
  let url = stdlib::module("Url").expect("Std.Url exists");
  let out = compile(
    url.source,
    &stdlib::filename("Url"),
    &CompileOptions {
      runtime: "../runtime.js".to_string(),
      ..CompileOptions::default()
    },
  );

  assert!(out.diagnostics.is_empty(), "{:#?}", out.diagnostics);
  assert!(out.js.contains("from \"./bindings/Url.js\""), "{}", out.js);
  assert!(url.files.iter().any(|(path, _)| *path == "bindings/Url.js"));
}

#[test]
fn runs_in_browser_host() {
  const BROWSER_APP: &str = "module App

uses
  Std.Url

hosts
  Browser

functions
  title(text: String) -> String {
    Url.decode(text)
  }

exports
  title
";

  let options = CompileOptions {
    host: HostOption::Fixed(Some("Browser".to_string())),
    ..CompileOptions::default()
  };
  let out = compile(BROWSER_APP, "app.px", &options);

  assert!(
    out.diagnostics.iter().all(|d| d.severity != Severity::Error),
    "{:#?}",
    out.diagnostics
  );
  assert!(out.std_imports.contains(&"Url".to_string()));

  let hosts = dump_stage_with(
    BROWSER_APP,
    "app.px",
    Stage::Hosts,
    &CompileOptions::default(),
  );
  let output =
    hosts.output.unwrap_or_else(|| panic!("{:#?}", hosts.diagnostics));

  assert_eq!(output, "title : every host\n");
}

fn text() -> impl Strategy<Value = String> {
  prop::collection::vec(
    prop::sample::select(vec![
      "a", "Z", "0", "&", "=", "+", " ", "%", "%2", "?", "#", "/", "é", "日",
      "🎉", "-", "~",
    ]),
    0..6,
  )
  .prop_map(|parts| parts.concat())
}

fn literal(text: &str) -> String {
  format!("\"{}\"", text.replace('#', "\\#"))
}

fn round_trip_program(pairs: &[(String, String)]) -> String {
  let list = pairs
    .iter()
    .map(|(n, v)| format!("{{ name: {}, value: {} }}", literal(n), literal(v)))
    .collect::<Vec<_>>()
    .join(", ");

  format!(
    "module M\n\nuses\n  Std.List\n  Std.Url\n\nfunctions\n  main() {{\n    let pairs = [{list}]\n\n    Log.info(Bool.to_string(Url.parse_query(Url.build_query(pairs)) == pairs))\n  }}\n\nexports\n  main\n"
  )
}

proptest! {
  #![proptest_config(Config { cases: 16, ..Config::default() })]

  #[test]
  fn query_round_trip(pairs in prop::collection::vec((text(), text()), 0..6)) {
    prop_assert_eq!(run(&round_trip_program(&pairs)), "true\n");
  }
}
