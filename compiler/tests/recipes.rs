mod common;

use common::node::run_program;
use polar_compiler::{
  CompileOptions, Stage, dump_stage_with, shared::diagnostic::Diagnostic,
  shared::modules::ModuleSource, shared::source::SourceFile,
};

const RECIPE: &str = "  Describe<a> {
    describe(value: a) -> String
  } derive {
    record(fields: List<{ name: String, value: String }>) -> String {
      let parts = List.map(fields, function(f) { \"#{f.name}: #{f.value}\" })

      \"{ #{List.join(parts, \", \")} }\"
    }

    variant(name: String, args: List<String>) -> String {
      match args {
        [] -> name,
        _ -> \"#{name}(#{List.join(args, \", \")})\",
      }
    }
  }
";

const PRIMITIVES: &str = "impls
  Describe for Int {
    describe(value) {
      Int.to_string(value)
    }
  }

  Describe for String {
    describe(value) {
      value
    }
  }
";

fn shapes() -> String {
  format!(
    "module Shapes\n\nuses\n  Std.List\n\ntraits\n{RECIPE}\n{PRIMITIVES}\nexports\n  Describe {{ describe }}\n"
  )
}

fn modules() -> Vec<ModuleSource> {
  vec![ModuleSource {
    path: "Shapes".to_string(),
    source: shapes(),
    specifier: "./shapes.js".to_string(),
    plugins: Vec::new(),
  }]
}

fn main_module(types: &str, main: &str, impls: &str) -> String {
  let main: String = main.lines().flat_map(|l| ["    ", l, "\n"]).collect();

  format!(
    "module Main\n\nuses\n  Shapes {{ describe }}\n\ntypes\n{types}\n\nfunctions\n  main() {{\n{main}  }}\n\n{impls}exports\n  main\n"
  )
}

fn run(types: &str, main: &str, impls: &str) -> Vec<String> {
  run_program(&main_module(types, main, impls), "main.px", &modules())
    .unwrap_or_else(|e| panic!("{e}"))
    .lines()
    .map(str::to_string)
    .collect()
}

fn diagnostics_with(src: &str, modules: Vec<ModuleSource>) -> Vec<Diagnostic> {
  let options = CompileOptions { modules, ..CompileOptions::default() };

  dump_stage_with(src, "main.px", Stage::Types, &options).diagnostics
}

fn errors_with(src: &str, modules: Vec<ModuleSource>) -> Vec<String> {
  let file = SourceFile::new("main.px", src);

  diagnostics_with(src, modules)
    .iter()
    .map(|d| format!("{} at {:?}", d.code, file.slice(&d.primary.span)))
    .collect()
}

fn local(traits: &str, rest: &str) -> String {
  format!("uses\n  Std.List\n\ntraits\n{traits}\n{rest}")
}

const TYPES: &str = "  Status = Draft | Published(Int) derive(Describe)
  Post = { title: String, status: Status } derive(Describe)";

#[test]
fn recipe_record_runs() {
  let src = local(
    RECIPE,
    &format!(
      "types\n  Post = {{ title: String, id: Int }} derive(Describe)\n\nfunctions\n  main() {{\n    let post: Post = {{ title: \"Hi\", id: 1 }}\n    Log.info(describe(post))\n  }}\n\n{PRIMITIVES}\nexports\n  main\n"
    ),
  );

  assert_eq!(run_program(&src, "t.px", &[]).unwrap(), "{ title: Hi, id: 1 }\n");
}

#[test]
fn recipe_variant_runs() {
  assert_eq!(
    run(
      TYPES,
      "Log.info(describe(Published(2026)))\nLog.info(describe(Draft))",
      ""
    ),
    ["Published(2026)", "Draft"]
  );
}

#[test]
fn cross_module() {
  assert_eq!(
    run(
      TYPES,
      "let post: Post = { title: \"Hi\", status: Published(2026) }\nLog.info(describe(post))",
      ""
    ),
    ["{ title: Hi, status: Published(2026) }"]
  );
}

#[test]
fn hand_written_wins() {
  let types = "  Status = Draft | Published(Int)\n  Post = { title: String, status: Status } derive(Describe)";
  let impls = "impls\n  Describe for Status {\n    describe(value) {\n      \"S\"\n    }\n  }\n\n";

  assert_eq!(
    run(
      types,
      "let post: Post = { title: \"Hi\", status: Draft }\nLog.info(describe(post))",
      impls
    ),
    ["{ title: Hi, status: S }"]
  );
}

#[test]
fn generic_type() {
  let types = "  Status = Draft | Published(Int) derive(Describe)\n  Box<a> = Box(a) derive(Describe)";

  assert_eq!(run(types, "Log.info(describe(Box(Draft)))", ""), ["Box(Draft)"]);
}

#[test]
fn not_derivable() {
  let src = "traits\n  Describe<a> {\n    describe(value: a) -> String\n  }\n\ntypes\n  Status = A | B derive(Describe)\n";
  let diagnostics = diagnostics_with(src, Vec::new());

  assert_eq!(errors_with(src, Vec::new()), ["POLAR0710 at \"Describe\""]);
  assert_eq!(
    diagnostics[0].message,
    "`Describe` can't be derived: it has no `derive { … }` recipe"
  );
  assert_eq!(
    diagnostics[0].help.as_deref(),
    Some("write the impl in the `impls` zone instead")
  );
}

#[test]
fn missing_case() {
  let traits = "  Describe<a> {\n    describe(value: a) -> String\n  } derive {\n    variant(name: String, args: List<String>) -> String {\n      name\n    }\n  }\n";
  let src =
    local(traits, "types\n  Post = { title: String } derive(Describe)\n");
  let diagnostics = diagnostics_with(&src, Vec::new());

  assert_eq!(errors_with(&src, Vec::new()), ["POLAR0710 at \"Describe\""]);
  assert!(diagnostics[0].message.contains("`record`"), "{diagnostics:#?}");
}

#[test]
fn two_methods() {
  let traits = "  Describe<a> {\n    describe(value: a) -> String\n    label(value: a) -> String\n  } derive {\n    variant(name: String, args: List<String>) -> String {\n      name\n    }\n  }\n";
  let src = local(traits, "");
  let diagnostics = diagnostics_with(&src, Vec::new());

  assert_eq!(errors_with(&src, Vec::new()), ["POLAR0711 at \"derive\""]);
  assert_eq!(diagnostics[0].message, "a recipe needs exactly one method");
}

#[test]
fn producer_trait() {
  let traits = "  Make<a> {\n    make() -> a\n  } derive {\n    variant(name: String, args: List<a>) -> a {\n      name\n    }\n  }\n";
  let src = local(traits, "");
  let diagnostics = diagnostics_with(&src, Vec::new());

  assert_eq!(errors_with(&src, Vec::new()), ["POLAR0711 at \"derive\""]);
  assert_eq!(
    diagnostics[0].message,
    "a recipe's method must take one parameter of type `a`"
  );
}

#[test]
fn returns_the_value() {
  let traits = "  Copy<a> {\n    copy(value: a) -> a\n  } derive {\n    variant(name: String, args: List<String>) -> String {\n      name\n    }\n  }\n";
  let src = local(traits, "");
  let diagnostics = diagnostics_with(&src, Vec::new());

  assert_eq!(diagnostics[0].message, "a recipe's method must not return `a`");
}

#[test]
fn wrong_case_signature() {
  let traits = "  Describe<a> {\n    describe(value: a) -> String\n  } derive {\n    record(fields: List<String>) -> String {\n      \"\"\n    }\n  }\n";
  let src = local(traits, "");
  let diagnostics = diagnostics_with(&src, Vec::new());

  assert_eq!(errors_with(&src, Vec::new()), ["POLAR0711 at \"record\""]);
  assert!(
    diagnostics[0]
      .help
      .iter()
      .any(|h| h.contains("List<{ name: String, value: String }>")),
    "{diagnostics:#?}"
  );
}

#[test]
fn field_without_impl() {
  let src = main_module(
    "  Post = { title: String, on_click: function() -> {} } derive(Describe)",
    "Log.info(\"\")",
    "",
  );
  let found = errors_with(&src, modules());

  assert_eq!(found, ["POLAR0707 at \"on_click: function() -> {}\""]);
}

#[test]
fn own_list_type() {
  let types = "  List<a> = Nil | Cons(a, List<a>)\n  Status = Draft | Published(Int) derive(Describe)";

  assert_eq!(
    run(types, "let xs = [1, 2]\nLog.info(describe(Published(1)))", ""),
    ["Published(1)"]
  );
}

#[test]
fn field_order() {
  let types = "  Pair = { b: Int, a: Int } derive(Describe)";

  assert_eq!(
    run(types, "let p: Pair = { a: 1, b: 2 }\nLog.info(describe(p))", ""),
    ["{ b: 2, a: 1 }"]
  );
}

#[test]
fn recipe_functions_are_hidden() {
  let src = "uses\n  Shapes\n\nfunctions\n  f() {\n    Shapes.recipe\n  }\n";
  let found = errors_with(src, modules());

  assert_eq!(found.len(), 1, "{found:?}");
  assert!(
    !diagnostics_with(src, modules())[0].help.iter().any(|h| h.contains('$'))
  );
}
