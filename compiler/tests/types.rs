use polar_compiler::{
  shared::diagnostic::Diagnostic,
  syntax::ast::{dump::dump_node_shape, fields::AsNode},
  syntax::lexer::token::TokenKind,
  syntax::parser::MAX_DEPTH,
};

mod common;

use common::{codes, drive, sexp::sexp};

mod types {
  use super::*;

  #[track_caller]
  fn check_type(src: &str, expected: &str) {
    let mut dump = String::new();

    let diagnostics = drive(src, |parser| {
      let ty = parser.type_expr();

      dump = dump_node_shape(ty.as_node());
    });

    assert_eq!(dump, expected);
    assert!(
      diagnostics.is_empty(),
      "unexpected diagnostics: {:?}",
      codes(&diagnostics)
    );
  }

  #[test]
  fn type_ref() {
    check_type("Int", &sexp!(TypeRef Int));
  }

  #[test]
  fn generic() {
    check_type("Id<Post>", &sexp!(TypeRef Id (TypeRef Post)));
  }

  #[test]
  fn nested_generic_closes() {
    check_type(
      "Result<Id<Post>, E>",
      &sexp!(TypeRef Result (TypeRef Id (TypeRef Post)) (TypeRef E)),
    );
  }

  #[test]
  fn generic_trailing_comma() {
    check_type("Result<A, B,>", &sexp!(TypeRef Result (TypeRef A) (TypeRef B)));
  }

  #[test]
  fn type_var() {
    check_type("rest", &sexp!(TypeVar rest));
  }

  #[test]
  fn parens_make_no_node() {
    check_type("(Int)", &sexp!(TypeRef Int));
  }

  #[test]
  fn empty_record() {
    check_type("{}", &sexp!(RecordType));
  }

  #[test]
  fn record_fields() {
    check_type(
      "{ a: Int, b: String }",
      &sexp!(RecordType
        (FieldType a (TypeRef Int))
        (FieldType b (TypeRef String))),
    );
  }

  #[test]
  fn record_trailing_comma() {
    check_type("{ a: Int, }", &sexp!(RecordType (FieldType a (TypeRef Int))));
  }

  #[test]
  fn any_record() {
    check_type("{ | r }", &sexp!(RecordType r));
  }

  #[test]
  fn row_tail() {
    check_type(
      "{ title: String | rest }",
      &sexp!(RecordType rest (FieldType title (TypeRef String))),
    );
  }

  #[test]
  fn row_tail_after_trailing_comma() {
    check_type(
      "{ a: Int, | r }",
      &sexp!(RecordType r (FieldType a (TypeRef Int))),
    );
  }

  #[test]
  fn fn_type() {
    check_type(
      "function(Int, String) -> Bool",
      &sexp!(FnType (TypeRef Int) (TypeRef String) (TypeRef Bool)),
    );
  }

  #[test]
  fn fn_type_no_params() {
    check_type("function() -> Bool", &sexp!(FnType (TypeRef Bool)));
  }

  #[test]
  fn fn_type_with_effects() {
    check_type(
      "function(Int) -> Bool / {Db}",
      &sexp!(FnType (TypeRef Int) (TypeRef Bool) (EffectRow (TypeRef Db))),
    );
  }

  #[test]
  fn effects_bind_to_the_nearest_fn() {
    check_type(
      "function(A) -> function(B) -> C / {E}",
      &sexp!(FnType
        (TypeRef A)
        (FnType (TypeRef B) (TypeRef C) (EffectRow (TypeRef E)))),
    );
  }

  #[test]
  fn parens_move_the_row_outward() {
    let diagnostics = drive("(function(B) -> C) / {E}", |parser| {
      let ty = parser.type_expr();

      assert_eq!(
        dump_node_shape(ty.as_node()),
        sexp!(FnType (TypeRef B) (TypeRef C)),
        "the parenthesised function type does not take the row"
      );

      let row = parser.effect_row().expect("the `/` is still on the cursor");

      assert_eq!(
        dump_node_shape(row.as_node()),
        sexp!(EffectRow (TypeRef E)),
        "the row is left for the enclosing type or signature"
      );
    });

    assert!(diagnostics.is_empty(), "{:?}", codes(&diagnostics));
  }
}

mod effect_rows {
  use super::*;

  fn check_row(src: &str, expected: &str) -> Vec<Diagnostic> {
    let mut dump = String::new();

    let diagnostics = drive(src, |parser| {
      let row = parser.effect_row().expect("an effect row");

      dump = dump_node_shape(row.as_node());
    });

    assert_eq!(dump, expected);

    diagnostics
  }

  #[test]
  fn no_row_without_a_slash() {
    drive("Int", |parser| assert!(parser.effect_row().is_none()));
  }

  #[test]
  fn empty_effect_row() {
    assert!(check_row("/ {}", &sexp!(EffectRow)).is_empty());
  }

  #[test]
  fn effect_tail() {
    assert!(
      check_row("/ {Db | e}", &sexp!(EffectRow e (TypeRef Db))).is_empty()
    );
  }

  #[test]
  fn lowercase_effect_without_bar() {
    let diagnostics = check_row("/ {Db, e}", &sexp!(EffectRow e (TypeRef Db)));

    assert_eq!(codes(&diagnostics), ["POLAR0205@7..8"]);
    assert_eq!(
      diagnostics[0].help.as_deref(),
      Some("effect variables go after `|`: `{Db | e}`")
    );
  }

  #[test]
  fn row_span_starts_at_the_slash() {
    drive("/ {Db}", |parser| {
      let row = parser.effect_row().expect("an effect row");

      assert_eq!((row.span.start, row.span.end), (0, 6));
    });
  }
}

mod type_recovery {
  use super::*;

  #[test]
  fn truncated_types_terminate() {
    for src in ["{ a: ", "Id<", "function(Int", "{ a: Int", "/ {Db"] {
      let diagnostics = drive(src, |parser| {
        parser.type_expr();
      });

      assert!(!diagnostics.is_empty(), "{src:?} reported nothing");
    }
  }

  #[test]
  fn a_token_that_cannot_start_a_type() {
    let diagnostics = drive("= 1", |parser| {
      let ty = parser.type_expr();

      assert_eq!(
        dump_node_shape(ty.as_node()),
        sexp!(InvalidType),
        "a bad type still yields a node"
      );

      assert_eq!(
        parser.peek(0).kind,
        TokenKind::Eq,
        "the unexpected token is left for recovery"
      );
    });

    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].message, "expected a type, found `=`");
  }

  #[test]
  fn deep_nesting_is_refused_not_overflowed() {
    let depth = MAX_DEPTH as usize + 1;
    let src = format!("{}Int{}", "(".repeat(depth), ")".repeat(depth));

    let diagnostics = drive(&src, |parser| {
      parser.type_expr();
    });

    assert!(
      diagnostics.iter().any(|d| d.message == "nesting too deep"),
      "expected the depth guard to fire: {:?}",
      codes(&diagnostics)
    );
  }
}
