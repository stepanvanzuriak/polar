mod common;

use common::expect_ice;
use polar_compiler::{
  backend::js::ast::{
    Expr, Literal, LiteralValue, LogicalOp, Unary, UnaryOp, ident,
    ident_renamed, logical, number, var,
  },
  shared::source::Span,
};

#[test]
fn nan_is_rejected() {
  let err = expect_ice(|| {
    let _ = Literal::number(f64::NAN);
  });

  assert!(err.message.contains("non-finite"), "{}", err.message);
}

#[test]
fn infinity_is_rejected() {
  expect_ice(|| {
    let _ = Literal::number(f64::INFINITY);
  });
  expect_ice(|| {
    let _ = number(f64::NEG_INFINITY);
  });
}

#[test]
fn negative_zero_is_rejected() {
  let err = expect_ice(|| {
    let _ = number(-0.0);
  });

  assert!(err.message.contains("negative zero"), "{}", err.message);
}

#[test]
fn negative_numbers_are_unary() {
  let Expr::Unary(Unary { op: UnaryOp::Negate, arg, .. }) = number(-1.0) else {
    panic!("expected Unary(Negate, …)");
  };

  assert_eq!(
    *arg,
    Expr::Literal(Literal { value: LiteralValue::Number(1.0), origin: None })
  );

  expect_ice(|| {
    let _ = Literal::number(-1.0);
  });
}

#[test]
fn origin_defaults_to_none() {
  assert_eq!(ident("x").origin, None);
  assert_eq!(var("x").origin(), None);

  let span = Span { file: "a.px".into(), start: 3, end: 4 };

  assert_eq!(var("x").at(Some(span.clone())).origin(), Some(&span));
}

#[test]
fn ident_carries_original_name() {
  let id = ident_renamed("class$", "class");

  assert_eq!(id.name, "class$");
  assert_eq!(id.original_name.as_deref(), Some("class"));
  assert_eq!(ident("x").original_name, None);
}

#[test]
fn logical_is_not_binary() {
  let e = logical(LogicalOp::And, var("a"), var("b"));

  assert!(matches!(e, Expr::Logical(_)), "{e:?}");
}

#[test]
fn backend_has_no_polar_imports() {
  let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/src/backend/js");
  let mut checked = 0;

  for entry in std::fs::read_dir(dir).expect("read src/backend/js") {
    let path = entry.expect("dir entry").path();

    if path.extension().is_none_or(|ext| ext != "rs") {
      continue;
    }

    let text = std::fs::read_to_string(&path).expect("read source");

    let code: String = text
      .lines()
      .filter(|line| !line.trim_start().starts_with("//"))
      .collect::<Vec<_>>()
      .join("\n");

    for forbidden in
      ["crate::syntax::ast", "crate::syntax::parser", "crate::syntax::lexer"]
    {
      assert!(
        !code.contains(forbidden),
        "{} uses `{forbidden}`",
        path.display()
      );
    }

    checked += 1;
  }

  assert!(checked > 0, "no .rs files under {dir}");
}
