use polar_compiler::{
  CompileOptions, CompileOutput, Stage, compile, dump_stage,
  shared::codes::DiagnosticCode,
};

#[test]
fn empty_source_compiles_cleanly() {
  let out = compile("", "empty.px", &CompileOptions::default());

  assert_eq!(
    CompileOutput { sourcemap: String::new(), ..out.clone() },
    CompileOutput {
      js: "import * as $rt from \"./_polar/runtime.js\";\n".to_string(),
      ..CompileOutput::default()
    }
  );
  assert!(out.sourcemap.starts_with("{\"version\":3,"), "{}", out.sourcemap);
}

#[test]
fn default_runtime_specifier() {
  assert_eq!(CompileOptions::default().runtime, "./_polar/runtime.js");
}

#[test]
fn parse_error_stops_the_pipeline() {
  let out =
    compile("functions\n  f( { pots }\n", "bad.px", &CompileOptions::default());

  assert!(!out.diagnostics.is_empty());
  assert!(
    out.diagnostics.iter().all(|d| d.code != DiagnosticCode::UnknownName)
  );
  assert_eq!(out.js, "");
}

#[test]
fn resolution_error_is_reported() {
  let out = compile(
    "functions\n  f(post) { pots }\n",
    "bad.px",
    &CompileOptions::default(),
  );
  let codes: Vec<_> = out.diagnostics.iter().map(|d| d.code).collect();

  assert_eq!(codes, [DiagnosticCode::UnknownName]);
  assert_eq!(out.js, "");
}

#[test]
fn dump_stage_core() {
  let dump = dump_stage(
    "functions\n  main() {\n    Log.info(\"hi\")\n  }\nexports\n  main\n",
    "hello.px",
    Stage::Core,
  );

  assert!(dump.diagnostics.is_empty(), "{:?}", dump.diagnostics);
  assert_eq!(
    dump.output.as_deref(),
    Some(
      "(module\n  (fn main/0 export () (app (builtin Log info) (lit \"hi\"))))\n"
    )
  );
}

#[test]
fn dump_stage_stops_at_the_failing_stage() {
  let dump = dump_stage("functions\n  f( {\n", "bad.px", Stage::Core);

  assert_eq!(dump.output, None);
  assert!(!dump.diagnostics.is_empty());
}
