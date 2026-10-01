use polar_compiler::{
  Stage, dump_stage,
  shared::diagnostic::Diagnostic,
  shared::source::SourceFile,
  stdlib::{self, MODULES},
};

fn result(src: &str) -> (Option<String>, Vec<Diagnostic>) {
  let result = dump_stage(src, "test.px", Stage::Types);

  (result.output, result.diagnostics)
}

fn type_of(src: &str, name: &str) -> String {
  let (output, diagnostics) = result(src);

  assert!(diagnostics.is_empty(), "unexpected errors: {diagnostics:#?}");

  let prefix = format!("{name} : ");

  output
    .unwrap()
    .lines()
    .find_map(|l| l.strip_prefix(&prefix))
    .expect("no such declaration")
    .to_string()
}

fn errors_of(src: &str) -> Vec<String> {
  let file = SourceFile::new("test.px", src);

  result(src)
    .1
    .iter()
    .map(|d| format!("{} at {:?}", d.code, file.slice(&d.primary.span)))
    .collect()
}

fn with_uses(uses: &str, rest: &str) -> String {
  format!("uses\n  {uses}\n\n{rest}")
}

#[test]
fn local_and_std_types_differ() {
  let src = with_uses(
    "Std.Option",
    "types\n  Maybe<a> = Nothing | Just(a)\n\nfunctions\n  f(o: Option<Int>) -> Maybe<Int> { o }\n",
  );

  assert_eq!(errors_of(&src), vec!["POLAR0501 at \"o\""]);
}

#[test]
fn printer_disambiguates() {
  let src = with_uses(
    "Std.Option",
    "types\n  Option<a> = None2 | Some2(a)\n\nfunctions\n  f(o: Option<Int>) { o == Some(1) }\n",
  );
  let (_, diagnostics) = result(&src);

  assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");

  let label = diagnostics[0].primary.message.clone().unwrap_or_default();

  assert!(label.contains("`Option<Int>`"), "{label}");
  assert!(label.contains("`Std.Option.Option<Int>`"), "{label}");
}

#[test]
fn std_modules_check() {
  for m in MODULES {
    let (_, diagnostics) = result(m.source);

    assert!(diagnostics.is_empty(), "Std.{}: {diagnostics:#?}", m.name);
  }
}

#[test]
fn std_interfaces_have_every_scheme() {
  for m in MODULES {
    let interface = stdlib::interface(m.name).unwrap();

    for (name, _) in &interface.functions {
      assert!(interface.scheme(name).is_some(), "Std.{}.{name}", m.name);
    }

    for name in &interface.constants {
      assert!(interface.scheme(name).is_some(), "Std.{}.{name}", m.name);
    }
  }
}

#[test]
fn std_list_map_type() {
  let src = with_uses("Std.List", "functions\n  f() { List.map }\n");

  assert_eq!(
    type_of(&src, "f"),
    "function() -> function(List<a>, function(a) -> b / {| e}) -> List<b> / {| e}"
  );
}

#[test]
fn std_constant() {
  let src = with_uses("Std.Math", "functions\n  f() { Math.pi }\n");

  assert_eq!(type_of(&src, "f"), "function() -> Float");
}

#[test]
fn std_constructor() {
  let src = with_uses("Std.Option", "functions\n  f() { Some(1) }\n");

  assert_eq!(type_of(&src, "f"), "function() -> Option<Int>");
}

#[test]
fn std_misuse_caught() {
  let src = with_uses(
    "Std.List",
    "functions\n  f(g: function(Int) -> Int) { List.map(1, g) }\n",
  );

  assert_eq!(errors_of(&src), vec!["POLAR0501 at \"1\""]);
}

#[test]
fn std_types_usable_in_annotations() {
  let src = "uses\n  Std.List\n  Std.Option\n\nfunctions\n  f(xs: List<Int>) -> Option<Int> { List.head(xs) }\n";

  assert_eq!(type_of(src, "f"), "function(List<Int>) -> Option<Int>");
}
