use polar_compiler::{
  CompileOptions, CompileOutput, compile,
  shared::codes::DiagnosticCode::{
    self, CallArity, DuplicateDefinition, UnknownBuiltinMember, UnknownModule,
  },
  shared::modules::{ModuleSource, file, imports},
};

const SHAPES: &str = "module Shapes

types
  Shape = Circle(Float) | Square(Float)

functions
  area(s: Shape) -> Float {
    match s {
      Circle(r) -> 3.0 * r * r,
      Square(w) -> w * w,
    }
  }

  hidden() {
    1
  }

exports
  area
";

fn shapes(path: &str) -> ModuleSource {
  ModuleSource {
    path: path.to_string(),
    source: SHAPES.to_string(),
    specifier: "./shapes.js".to_string(),
    plugins: Vec::new(),
  }
}

fn compile_with(src: &str, modules: Vec<ModuleSource>) -> CompileOutput {
  compile(
    src,
    "main.px",
    &CompileOptions { modules, ..CompileOptions::default() },
  )
}

fn codes(src: &str, modules: Vec<ModuleSource>) -> Vec<DiagnosticCode> {
  compile_with(src, modules).diagnostics.iter().map(|d| d.code).collect()
}

fn using(uses: &str, body: &str) -> String {
  format!("uses\n  {uses}\n\nfunctions\n  main() {{\n    {body}\n  }}\n")
}

#[test]
fn file_names() {
  assert_eq!(file("Complex"), "complex.px");
  assert_eq!(file("Geometry.HttpServer"), "geometry/http_server.px");
  assert_eq!(file("Utf8"), "utf8.px");
}

#[test]
fn finds_imports_outside_std() {
  let src = "uses\n  Std.List\n  Shapes\n  Web.Json as J\n  Shapes\n";

  assert_eq!(imports(src, "main.px", &[]), ["Shapes", "Web.Json"]);
}

#[test]
fn functions_and_ctors() {
  let out = compile_with(
    &using("Shapes", "Shapes.area(Circle(1.0)) + Shapes.area(Square(2.0))"),
    vec![shapes("Shapes")],
  );

  assert!(out.diagnostics.is_empty(), "{:#?}", out.diagnostics);
  assert!(out.js.contains("import * as $m$Shapes from \"./shapes.js\";"));
  assert!(out.js.contains("$m$Shapes.area("), "{}", out.js);
}

#[test]
fn alias_and_nested_path() {
  let src = using("Geometry.Shapes as G", "G.area(Square(2.0))");
  let out = compile_with(&src, vec![shapes("Geometry.Shapes")]);

  assert!(out.diagnostics.is_empty(), "{:#?}", out.diagnostics);
  assert!(out.js.contains("$m$Geometry$Shapes.area("), "{}", out.js);
}

#[test]
fn header_names_the_last_segment() {
  let wrong = ModuleSource {
    source: SHAPES.replace("module Shapes", "module Geometry"),
    ..shapes("Geometry.Shapes")
  };

  assert_eq!(
    codes(&using("Geometry.Shapes", "1"), vec![wrong]),
    [UnknownModule]
  );
}

#[test]
fn only_exports_are_visible() {
  assert_eq!(
    codes(&using("Shapes", "Shapes.hidden()"), vec![shapes("Shapes")]),
    [UnknownBuiltinMember]
  );
}

#[test]
fn arity_is_checked() {
  assert_eq!(
    codes(&using("Shapes", "Shapes.area()"), vec![shapes("Shapes")]),
    [CallArity]
  );
}

#[test]
fn unknown_module() {
  let out = compile_with("uses\n  Nope\n", vec![]);

  assert_eq!(out.diagnostics.len(), 1);
  assert_eq!(out.diagnostics[0].code, UnknownModule);
  assert!(
    out.diagnostics[0].help.as_deref().is_some_and(|h| h.contains("`nope.px`"))
  );
}

#[test]
fn imported_twice() {
  assert_eq!(
    codes("uses\n  Shapes\n  Std.List as Shapes\n", vec![shapes("Shapes")]),
    [DuplicateDefinition]
  );
}
