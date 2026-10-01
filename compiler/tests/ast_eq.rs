use polar_compiler::{
  shared::diagnostic::DiagnosticBag,
  shared::source::SourceFile,
  syntax::ast::{
    Module,
    eq::{ast_diff, ast_eq},
    fields::{AsNode, Field, children},
  },
  syntax::lexer::lex,
  syntax::parser::parse,
};

fn module(src: &str) -> Module {
  let file = SourceFile::new("test.px", src);
  let mut bag = DiagnosticBag::default();
  let lexed = lex(&file, &mut bag);
  let module = parse(&file, &lexed, &mut bag);

  assert!(!bag.has_errors(), "{src}\n{:#?}", bag.into_sorted());

  module
}

fn in_fn(body: &str) -> Module {
  module(&format!("functions\n  f() {{\n    {body}\n  }}\n"))
}

#[test]
fn differently_formatted_sources_compare_equal() {
  let a = module("functions\n f(){1}");
  let b = module("functions\n  f() {\n    1\n  }");

  assert!(ast_eq(&a, &b));
  assert_eq!(ast_diff(&a, &b), None);
}

#[test]
fn different_raw_compares_unequal() {
  assert!(!ast_eq(&in_fn("let x = 1_000\nx"), &in_fn("let x = 1000\nx")));
}

#[test]
fn different_names_compare_unequal() {
  assert!(!ast_eq(
    &module("functions\n  f() {1}"),
    &module("functions\n  g() {1}")
  ));
}

#[test]
fn different_shapes_compare_unequal() {
  let (a, b) = (in_fn("a + b"), in_fn("a * b"));

  assert!(!ast_eq(&a, &b));
  assert!(ast_diff(&a, &b).is_some_and(|d| d.contains("op")));
}

#[test]
fn different_node_kinds_compare_unequal() {
  let diff = ast_diff(&in_fn("a"), &in_fn("1"));

  assert!(
    diff.as_deref().is_some_and(|d| d.contains("Var vs IntLit")),
    "{diff:?}"
  );
}

#[test]
fn docs_are_ignored() {
  let a = module("functions\n  f() {\n    1\n  }\n");
  let b = module("functions\n  /// what f does\n  f() {\n    1\n  }\n");

  assert!(ast_eq(&a, &b));
}

#[test]
fn derived_eq_still_sees_spans() {
  let a = module("functions\n f(){1}");
  let b = module("functions\n  f() {\n    1\n  }");

  assert_ne!(a, b);
  assert!(ast_eq(&a, &b));
}

#[test]
fn diff_names_the_path() {
  let a = module("functions\n  f() { 1 }");
  let b = module("functions\n  f() { 2 }");
  let diff = ast_diff(&a, &b).expect("the modules differ");

  assert_eq!(diff, r#"zones[0].decls[0].body.result.raw: "1" vs "2""#);
}

#[test]
fn spans_are_only_ever_spans() {
  let src = r#"module M

uses
  A.B as C

types
  /// doc
  T<a> = { x: Int | r } derive(Eq)

  V = A(Int) | B

functions
  /// doc
  f(a: Int, g: function(Int) -> Int / {E | e}) -> Int / {Db} {
    let { x: y, .. } = a
    let z = -a.b(1) |> g
    match "s#{z}" {
      A(-1) -> { ..r, q: 1.5 },
      _ -> if a && b == c { function(x) { x } } else { true },
    }
  }

exports
  f
"#;
  let root = module(src);
  let mut work = vec![root.as_node()];
  let mut seen = 0;

  while let Some(node) = work.pop() {
    for (key, field) in node.fields() {
      match field {
        Field::Span(_) => {
          let allowed =
            key == "span" || (key == "op_span" && node.kind() == "Binary");

          assert!(
            allowed,
            "{}.{key} is a `Span` that is not a node's own",
            node.kind()
          );
          seen += 1;
        }
        Field::Spans(_) => {
          assert_eq!(key, "docs", "{}.{key} holds spans", node.kind());
        }
        _ => {}
      }
    }

    work.extend(children(node));
  }

  assert!(seen > 50, "the walk visited too little of the tree ({seen})");
}
