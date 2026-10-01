mod common;

use common::node::run_program;
use polar_compiler::{
  CompileOptions, Stage, compile, dump_stage, shared::diagnostic::Diagnostic,
  shared::source::SourceFile,
};

fn types_of(src: &str) -> String {
  let result = dump_stage(src, "test.px", Stage::Types);

  assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
  result.output.unwrap()
}

fn type_of(src: &str, name: &str) -> String {
  let prefix = format!("{name} : ");

  types_of(src)
    .lines()
    .find_map(|l| l.strip_prefix(&prefix))
    .expect("no such declaration")
    .to_string()
}

fn diagnostics_of(src: &str) -> Vec<Diagnostic> {
  dump_stage(src, "test.px", Stage::Types).diagnostics
}

fn errors_of(src: &str) -> Vec<String> {
  let file = SourceFile::new("test.px", src);

  diagnostics_of(src)
    .iter()
    .map(|d| format!("{} at {:?}", d.code, file.slice(&d.primary.span)))
    .collect()
}

fn functions(body: &str) -> String {
  let indented: String = body.lines().flat_map(|l| ["  ", l, "\n"]).collect();

  format!("functions\n{indented}")
}

fn program(types: &str, main: &str) -> String {
  let main: String = main.lines().flat_map(|l| ["    ", l, "\n"]).collect();

  format!(
    "types\n{types}\n\nfunctions\n  main() {{\n{main}  }}\n\nexports\n  main\n"
  )
}

fn run(src: &str) -> Vec<String> {
  run_program(src, "test.px", &[])
    .unwrap_or_else(|e| panic!("{e}"))
    .lines()
    .map(str::to_string)
    .collect()
}

const STATUS_POST: &str = "  Status = Draft | Published(Int) derive(Eq, Show)
  Post = { id: Int, title: String, status: Status } derive(Eq, Show)";

#[test]
fn prelude_in_scope() {
  assert!(errors_of("types\n  Status = A | B derive(Eq)\n").is_empty());
}

#[test]
fn prelude_methods_not_bare() {
  let src = functions("show(x) {\n  x\n}\n\nf() {\n  show(1)\n}\n");

  assert_eq!(type_of(&src, "f"), "function() -> Int");
  assert!(
    errors_of(&functions("f() {\n  eq(1, 1)\n}\n"))
      .iter()
      .all(|e| e.starts_with("POLAR0301"))
  );
}

#[test]
fn complex_project_keeps_its_show() {
  let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
  let complex =
    std::fs::read_to_string(root.join("projects/complex/src/complex.px"))
      .unwrap();
  let main =
    std::fs::read_to_string(root.join("projects/complex/src/main.px")).unwrap();
  let modules = vec![polar_compiler::shared::modules::ModuleSource {
    path: "Complex".to_string(),
    source: complex,
    specifier: "./complex.js".to_string(),
    plugins: Vec::new(),
  }];
  let out = compile(
    &main,
    "main.px",
    &CompileOptions { modules, ..CompileOptions::default() },
  );

  assert!(out.diagnostics.is_empty(), "{:#?}", out.diagnostics);
}

#[test]
fn prelude_qualified_call() {
  assert_eq!(
    type_of(&functions("f() {\n  Prelude.show(1)\n}\n"), "f"),
    "function() -> String"
  );
}

#[test]
fn prelude_brought_in_bare() {
  let src = format!(
    "uses\n  Std.Prelude {{ show }}\n\n{}",
    functions("f() {\n  show(1)\n}\n")
  );

  assert_eq!(type_of(&src, "f"), "function() -> String");
}

#[test]
fn eq_on_function_rejected() {
  let src = functions("f() {\n  function(x) { x } == function(y) { y }\n}\n");
  let diagnostics = diagnostics_of(&src);

  assert_eq!(errors_of(&src), ["POLAR0707 at \"==\""]);
  assert!(
    diagnostics[0].message.contains("function(a) -> a"),
    "{diagnostics:#?}"
  );
}

#[test]
fn eq_on_anonymous_record_rejected() {
  let src = functions("f() {\n  { a: 1 } == { a: 1 }\n}\n");

  assert_eq!(errors_of(&src), ["POLAR0707 at \"==\""]);
}

#[test]
fn generic_eq_inferred() {
  assert_eq!(
    type_of(&functions("same(a, b) {\n  a == b\n}\n"), "same"),
    "function(a, a) -> Bool where Eq<a>"
  );
}

#[test]
fn interpolation_needs_show() {
  let src = functions("f() {\n  \"#{function(x) { x }}\"\n}\n");

  assert_eq!(errors_of(&src), ["POLAR0707 at \"function(x) { x }\""]);
}

#[test]
fn interpolation_string_free() {
  let out = dump_stage(
    &functions("f(s: String) {\n  \"#{s}\"\n}\n"),
    "test.px",
    Stage::Dictionaries,
  );

  assert!(out.diagnostics.is_empty(), "{:#?}", out.diagnostics);
  assert!(!out.output.unwrap().contains("(dict"));
}

#[test]
fn derive_eq_variant_runs() {
  let src = program(
    STATUS_POST,
    "Log.info(\"#{Published(1) == Published(1)}\")\nLog.info(\"#{Draft == Published(1)}\")\nLog.info(\"#{Published(1) != Published(2)}\")",
  );

  assert_eq!(run(&src), ["true", "false", "true"]);
}

#[test]
fn derive_eq_record_runs() {
  let src = program(
    STATUS_POST,
    "let a: Post = { id: 1, title: \"Hi\", status: Draft }\nlet b: Post = { id: 1, title: \"Hi\", status: Draft }\nlet c: Post = { ..b, status: Published(2026) }\nLog.info(\"#{a == b}\")\nLog.info(\"#{a == c}\")",
  );

  assert_eq!(run(&src), ["true", "false"]);
}

#[test]
fn derive_show_runs() {
  let src = program(
    STATUS_POST,
    "let post: Post = { id: 1, title: \"Hi\", status: Draft }\nLog.info(\"#{post}\")\nLog.info(\"#{Published(2026)}\")",
  );

  assert_eq!(
    run(&src),
    ["{ id: 1, title: \"Hi\", status: Draft }", "Published(2026)"]
  );
}

#[test]
fn derive_generic() {
  let src = program(
    "  Box<a> = Box(a) derive(Eq, Show)",
    "Log.info(\"#{Box(1) == Box(1)}\")\nLog.info(\"#{Box(\"x\")}\")",
  );

  assert_eq!(run(&src), ["true", "Box(x)"]);
}

#[test]
fn derive_needs_field_impl() {
  let src = "types\n  Post = {\n    id: Int,\n    on_click: function() -> {},\n  } derive(Eq)\n";
  let diagnostics = diagnostics_of(src);

  assert_eq!(errors_of(src), ["POLAR0707 at \"on_click: function() -> {}\""]);
  assert_eq!(diagnostics[0].message, "`Post` can't derive `Eq`");
  assert_eq!(
    diagnostics[0].primary.message.as_deref(),
    Some("`function() -> {}` has no `Eq` impl")
  );
}

#[test]
fn derive_and_impl_clash() {
  let src = "types\n  Status = A | B derive(Eq)\n\nimpls\n  Eq for Status {\n    eq(x, y) {\n      true\n    }\n  }\n";

  assert_eq!(errors_of(src), ["POLAR0705 at \"Eq\""]);
}

#[test]
fn std_derives() {
  let src = "uses\n  Std.Option\n  Std.List\n\nfunctions\n  main() {\n    Log.info(\"#{Some(1) == Some(1)} #{Some(1) == None}\")\n    Log.info(\"#{Some([1, 2])}\")\n  }\n\nexports\n  main\n";

  assert_eq!(run(src), ["true false", "Some(Cons(1, Cons(2, Nil)))"]);
}

#[test]
fn m1_07_primitives_still_strict() {
  let out = compile(
    &functions("f(a: Int, b: Int) {\n  a == b\n}\n"),
    "t.px",
    &CompileOptions::default(),
  );

  assert!(out.diagnostics.is_empty(), "{:#?}", out.diagnostics);
  assert!(out.js.contains("a === b"), "{}", out.js);
  assert!(!out.js.contains("Prelude"), "{}", out.js);
}

#[test]
fn derive_expansion_is_visible_in_the_ast() {
  let out =
    dump_stage("types\n  Status = A | B derive(Eq)\n", "test.px", Stage::Ast);

  assert!(out.output.unwrap().contains("(ImplDecl Eq"));
}
