mod common;

use common::{node::run_program_plugins, notes_plugin};
use polar_compiler::{
  CompileOptions, Stage, compile, dump_stage_with, format_plugins,
  shared::codes::DiagnosticCode::{
    ExpectedDeclaration, PluginError, PluginKeywordAsName, TypeMismatch,
    ZoneDuplicate, ZoneOutOfOrder,
  },
  shared::diagnostic::Diagnostic,
  syntax::ast::{Builtin, ZoneKind},
  syntax::plugins,
};

fn paths(sets: &[&str]) -> Vec<String> {
  sets.iter().map(|_| notes_plugin()).collect()
}

fn options(sets: &[&str]) -> CompileOptions {
  CompileOptions { plugins: paths(sets), ..CompileOptions::default() }
}

fn diagnostics(src: &str, sets: &[&str]) -> Vec<Diagnostic> {
  compile(src, "test.px", &options(sets)).diagnostics
}

fn ast(src: &str, sets: &[&str]) -> String {
  let out = dump_stage_with(src, "test.px", Stage::Ast, &options(sets));

  assert!(out.diagnostics.is_empty(), "{:#?}", out.diagnostics);
  out.output.expect("an AST dump")
}

const NOTES: &str = "types
  Name = String

notes
  greeting = \"hello\"
  farewell = \"bye\"

functions
  main() {
    Log.info(greeting)
    Log.info(farewell)
  }

exports
  main
";

#[test]
fn builtin_ranks_follow_the_roadmap_order() {
  let order: Vec<&str> = Builtin::ALL.iter().map(|b| b.as_str()).collect();

  assert_eq!(
    order,
    [
      "uses",
      "hosts",
      "traits",
      "types",
      "constants",
      "effects",
      "externs",
      "binds",
      "functions",
      "impls",
      "exports"
    ]
  );

  for pair in Builtin::ALL.windows(2) {
    assert!(ZoneKind::Builtin(pair[0]) < ZoneKind::Builtin(pair[1]));
  }
}

#[test]
fn a_plugin_zone_ranks_right_after_its_builtin() {
  let dump = ast(NOTES, &["test"]);
  let notes = dump.find("(Zone notes").expect("a notes zone");
  let types = dump.find("(Zone types").expect("a types zone");
  let constants = dump.find("(Zone constants").expect("a constants zone");

  assert!(types < notes && notes < constants, "{dump}");
}

#[test]
fn notes_parse_when_enabled() {
  let dump = ast(NOTES, &["test"]);

  assert!(
    dump.contains(r#"(PluginEntry text="greeting = \"hello\"""#),
    "{dump}"
  );
  assert!(dump.contains("(PluginEntry text=\"farewell"), "{dump}");
}

#[test]
fn emit_ast_shows_the_generated_zone() {
  let dump = ast(NOTES, &["test"]);

  assert!(dump.contains("(Zone constants"), "{dump}");
  assert!(dump.contains("(ConstDecl greeting"), "{dump}");
}

#[test]
fn notes_run() {
  let plugin = notes_plugin();
  let out =
    run_program_plugins(NOTES, "test.px", &[], None, &[], &[plugin.as_str()]);

  assert_eq!(out.unwrap(), "hello\nbye\n");
}

#[test]
fn notes_is_a_plain_name_when_not_enabled() {
  let src = "functions\n  main() {\n    let notes = 1\n\n    notes + 1\n  }\n";

  assert!(diagnostics(src, &[]).is_empty());
}

#[test]
fn a_disabled_plugin_zone_suggests_enabling_it() {
  let out = diagnostics(NOTES, &[]);
  let first = out.first().expect("a diagnostic");

  assert_eq!(first.code, ExpectedDeclaration, "{out:#?}");
  assert!(
    first.help.as_deref().is_some_and(|h| h.contains("[dependencies]")),
    "{first:#?}"
  );
}

#[test]
fn notes_after_functions_is_out_of_order() {
  let src = "functions\n  main() { 1 }\n\nnotes\n  a = \"x\"\n";
  let out = diagnostics(src, &["test"]);

  assert_eq!(out[0].code, ZoneOutOfOrder, "{out:#?}");
  assert!(out[0].message.contains("`notes` zone must come before"));
  assert!(
    out[0]
      .help
      .as_deref()
      .is_some_and(|h| h.contains("`types` -> `notes` -> `constants`")),
    "{:#?}",
    out[0]
  );
}

#[test]
fn notes_twice_is_a_duplicate() {
  let src = "notes\n  a = \"x\"\n\nnotes\n  b = \"y\"\n";

  assert_eq!(diagnostics(src, &["test"])[0].code, ZoneDuplicate);
}

#[test]
fn an_enabled_keyword_is_not_a_function_name() {
  let src = "functions\n  notes() { 1 }\n";
  let out = diagnostics(src, &["test"]);

  assert_eq!(out[0].code, PluginKeywordAsName, "{out:#?}");
  assert!(
    out[0].message.contains("`notes` is a zone keyword in this project"),
    "{:#?}",
    out[0]
  );
}

#[test]
fn a_type_error_in_generated_code_points_at_the_entry() {
  let src = "notes\n  greeting = 42\n\nfunctions\n  main() { greeting }\n";
  let out = diagnostics(src, &["test"]);
  let error = out.iter().find(|d| d.code == TypeMismatch).expect("a mismatch");
  let entry = src.find("greeting = 42").unwrap();
  let span = &error.primary.span;

  assert!(
    span.start >= entry && span.end <= entry + "greeting = 42".len(),
    "{error:#?}"
  );
}

#[test]
fn a_broken_entry_does_not_hide_the_next() {
  let src = "notes\n  a = \n  b = \"x\"\n\nfunctions\n  main() { b }\n";
  let out = diagnostics(src, &["test"]);

  assert_eq!(out.len(), 1, "{out:#?}");
}

#[test]
fn formatting_notes_is_idempotent() {
  let src = "notes\n  // first\n  a   =  \"x\"  // trailing\n\n  b = \"y\"\n  // dangling\n";
  let sets = paths(&["test"]);
  let once = format_plugins(src, "test.px", &sets).output.expect("formats");
  let twice = format_plugins(&once, "test.px", &sets).output.expect("formats");

  assert_eq!(once, twice);
  assert!(once.contains("  a = \"x\" // trailing\n"), "{once}");
  assert!(once.contains("  // first\n"), "{once}");
  assert!(once.contains("  // dangling"), "{once}");
}

#[test]
fn the_library_lists_its_zones() {
  let zones = plugins::load(&notes_plugin()).expect("loads");

  assert_eq!(zones.len(), 1);
  assert_eq!(zones[0].keyword, "notes");
  assert_eq!(zones[0].after, "types");
}

#[test]
fn a_missing_library_is_an_error() {
  let error = plugins::load("/no/such/plugin.so").unwrap_err();

  assert!(error.contains("cannot load the plugin"), "{error}");
}

#[test]
fn a_plugin_error_points_at_its_entry() {
  let src = "notes\n  greeting\n";
  let out = diagnostics(src, &["test"]);

  assert_eq!(out.len(), 1, "{out:#?}");
  assert_eq!(out[0].code, PluginError);
  assert_eq!(out[0].primary.span.start, src.find("greeting").unwrap());
}

#[test]
fn only_generating_below_is_allowed() {
  let notes = ZoneKind::Plugin(polar_compiler::syntax::ast::PluginId(0));

  assert!(!plugins::generates_below(
    ZoneKind::Builtin(Builtin::Types),
    Builtin::Uses
  ));
  assert!(plugins::generates_below(
    ZoneKind::Builtin(Builtin::Types),
    Builtin::Constants
  ));
  let _ = notes;
}
