use polar_compiler::{
  shared::codes::DiagnosticCode::{
    self, ChainedComparison, EmptyBlock, InterpolationInPattern,
    LocalFnDeclaration, MissingElse, RecordLiteralInCondition, SpreadNotFirst,
    StatementsOnSameLine, TrailingLet, UnexpectedToken,
  },
  shared::diagnostic::{Diagnostic, DiagnosticBag},
  shared::source::SourceFile,
  syntax::ast::{
    Block, Decl, Else, Expr, FnDecl, Module, Pattern, Stmt,
    dump::dump_node_shape,
    fields::{AsNode, NodeRef},
  },
  syntax::lexer::{lex, token::TokenKind},
  syntax::parser::{
    ParseOptions, parse, parse_with,
    precedence::{Assoc, InfixEntry, PREFIX_BINDING_POWER, PrecedenceTable},
  },
};

mod common;

use common::{
  invariants::check_invariants,
  sexp::{render, sexp},
};

fn parse_opts(src: &str, options: &ParseOptions) -> (Module, Vec<Diagnostic>) {
  let file = SourceFile::new("test.px", src);
  let mut bag = DiagnosticBag::default();
  let lexed = lex(&file, &mut bag);
  let module = parse_with(&file, &lexed, &mut bag, options);

  check_invariants(src, &module);

  (module, bag.into_sorted())
}

fn parse_src(src: &str) -> (Module, Vec<Diagnostic>) {
  parse_opts(src, &ParseOptions::default())
}

fn first_fn(module: &Module) -> &FnDecl {
  match &module.zones[0].decls[0] {
    Decl::Fn(f) => f,
    other => panic!("expected a function, got {other:?}"),
  }
}

fn shape(node: NodeRef<'_>) -> String {
  dump_node_shape(node)
    .replace("Binary ==", "Binary EQ")
    .replace("Binary !=", "Binary NE")
    .replace("Binary <=", "Binary LE")
    .replace("Binary >=", "Binary GE")
}

fn expr(src: &str) -> (String, Vec<Diagnostic>) {
  expr_opts(src, &ParseOptions::default())
}

fn expr_opts(src: &str, options: &ParseOptions) -> (String, Vec<Diagnostic>) {
  let (module, diagnostics) =
    parse_opts(&format!("functions\n  f() {{ {src} }}\n"), options);

  (shape(first_fn(&module).body.result.as_node()), diagnostics)
}

fn block(src: &str) -> (String, Vec<Diagnostic>) {
  let (module, diagnostics) = parse_src(&format!("functions\n  f() {src}\n"));

  (shape(first_fn(&module).body.as_node()), diagnostics)
}

fn pattern(src: &str) -> (String, Vec<Diagnostic>) {
  let (module, diagnostics) =
    parse_src(&format!("functions\n  f() {{ match x {{ {src} -> 0 }} }}\n"));

  let Expr::Match(m) = &*first_fn(&module).body.result else {
    panic!("expected a match")
  };

  (shape(m.arms[0].pattern.as_node()), diagnostics)
}

fn codes(diagnostics: &[Diagnostic]) -> Vec<DiagnosticCode> {
  diagnostics.iter().map(|d| d.code).collect()
}

fn clean(diagnostics: &[Diagnostic]) {
  assert!(diagnostics.is_empty(), "unexpected diagnostics: {diagnostics:?}");
}

fn assert_expr(src: &str, expected: &str) {
  let (dump, diagnostics) = expr(src);

  assert_eq!(dump, expected, "for `{src}`");
  clean(&diagnostics);
}

mod precedence {
  use super::*;

  #[test]
  fn table_matches_the_contract() {
    let table = PrecedenceTable::default();
    let entry = |kind| table.infix(kind).unwrap();

    assert_eq!(
      entry(TokenKind::PipeOp),
      InfixEntry { bp: 10, assoc: Assoc::Left }
    );
    assert_eq!(
      entry(TokenKind::OrOr),
      InfixEntry { bp: 20, assoc: Assoc::Right }
    );
    assert_eq!(
      entry(TokenKind::AndAnd),
      InfixEntry { bp: 30, assoc: Assoc::Right }
    );
    assert_eq!(entry(TokenKind::Lt), InfixEntry { bp: 40, assoc: Assoc::None });
    assert_eq!(
      entry(TokenKind::Plus),
      InfixEntry { bp: 50, assoc: Assoc::Left }
    );
    assert_eq!(
      entry(TokenKind::Percent),
      InfixEntry { bp: 60, assoc: Assoc::Left }
    );
    assert_eq!(
      entry(TokenKind::Dot),
      InfixEntry { bp: 80, assoc: Assoc::Left }
    );
    assert_eq!(
      entry(TokenKind::LParen),
      InfixEntry { bp: 80, assoc: Assoc::Left }
    );
    assert_eq!(PREFIX_BINDING_POWER, 70);
    assert_eq!(table.infix(TokenKind::Bang), None);
  }

  #[test]
  fn with_returns_a_modified_copy() {
    let table = PrecedenceTable::default();
    let moved = table
      .clone()
      .with(TokenKind::Plus, InfixEntry { bp: 60, assoc: Assoc::Left });

    assert_eq!(table.infix(TokenKind::Plus).unwrap().bp, 50);
    assert_eq!(moved.infix(TokenKind::Plus).unwrap().bp, 60);
  }

  #[test]
  fn pipe_is_loosest() {
    assert_expr(
      "a |> f || g",
      &sexp!(Pipe (Var a) (Binary || (Var f) (Var g))),
    );
  }

  #[test]
  fn or_below_and() {
    assert_expr(
      "a || b && c",
      &sexp!(Binary || (Var a) (Binary && (Var b) (Var c))),
    );
  }

  #[test]
  fn and_below_comparison() {
    assert_expr(
      "a && b == c",
      &sexp!(Binary && (Var a) (Binary EQ (Var b) (Var c))),
    );
  }

  #[test]
  fn comparison_below_additive() {
    assert_expr(
      "a == b + c",
      &sexp!(Binary EQ (Var a) (Binary + (Var b) (Var c))),
    );
  }

  #[test]
  fn additive_below_multiplicative() {
    assert_expr(
      "a + b * c",
      &sexp!(Binary + (Var a) (Binary * (Var b) (Var c))),
    );
  }

  #[test]
  fn bitwise_between_comparison_and_additive() {
    assert_expr(
      "a | b ^ c & d + e == f",
      &sexp!(Binary EQ (Binary | (Var a) (Binary ^ (Var b) (Binary & (Var c) (Binary + (Var d) (Var e))))) (Var f)),
    );
  }

  #[test]
  fn bitwise_is_left_associative() {
    assert_expr(
      "a & b & c",
      &sexp!(Binary & (Binary & (Var a) (Var b)) (Var c)),
    );
  }

  #[test]
  fn bit_not_is_prefix() {
    assert_expr("~a & b", &sexp!(Binary & (Unary ~ (Var a)) (Var b)));
  }

  #[test]
  fn unary_binds_tighter_than_multiplicative() {
    assert_expr("-a * b", &sexp!(Binary * (Unary - (Var a)) (Var b)));
  }

  #[test]
  fn field_access_binds_tighter_than_unary() {
    assert_expr("-a.b", &sexp!(Unary - (FieldAccess b (Var a))));
  }

  #[test]
  fn call_binds_tighter_than_unary() {
    assert_expr("!f(x)", &render("(Unary ! (Call (Var f) (Var x)))"));
  }

  #[test]
  fn modulo_matches_multiplicative() {
    assert_expr(
      "a + b % c",
      &sexp!(Binary + (Var a) (Binary % (Var b) (Var c))),
    );
  }

  #[test]
  fn minus_is_left() {
    assert_expr(
      "a - b - c",
      &sexp!(Binary - (Binary - (Var a) (Var b)) (Var c)),
    );
  }

  #[test]
  fn divide_is_left() {
    assert_expr(
      "a / b / c",
      &sexp!(Binary / (Binary / (Var a) (Var b)) (Var c)),
    );
  }

  #[test]
  fn or_is_right() {
    assert_expr(
      "a || b || c",
      &sexp!(Binary || (Var a) (Binary || (Var b) (Var c))),
    );
  }

  #[test]
  fn and_is_right() {
    assert_expr(
      "a && b && c",
      &sexp!(Binary && (Var a) (Binary && (Var b) (Var c))),
    );
  }

  #[test]
  fn pipe_is_left() {
    assert_expr("a |> f |> g", &sexp!(Pipe (Pipe (Var a) (Var f)) (Var g)));
  }

  #[test]
  fn field_access_is_left() {
    assert_expr("a.b.c", &sexp!(FieldAccess c (FieldAccess b (Var a))));
  }

  #[test]
  fn chained_comparison_diagnoses() {
    let (_, diagnostics) = expr("a < b < c");

    assert_eq!(codes(&diagnostics), [ChainedComparison]);
    assert_eq!(diagnostics[0].help.as_deref(), Some("`a < b && b < c`"));
  }

  #[test]
  fn chained_comparison_still_builds_an_ast() {
    let (dump, _) = expr("a < b < c");

    assert_eq!(dump, sexp!(Binary < (Binary < (Var a) (Var b)) (Var c)));
  }

  #[test]
  fn mixed_comparisons_chain() {
    let (_, diagnostics) = expr("a == b != c");

    assert_eq!(codes(&diagnostics), [ChainedComparison]);
  }

  #[test]
  fn parenthesised_comparison_is_accepted() {
    assert_expr(
      "(a < b) < c",
      &sexp!(Binary < (Binary < (Var a) (Var b)) (Var c)),
    );
  }

  #[test]
  fn table_is_injectable() {
    let options = ParseOptions {
      precedence: PrecedenceTable::default()
        .with(TokenKind::Plus, InfixEntry { bp: 60, assoc: Assoc::Left }),
    };
    let (dump, diagnostics) = expr_opts("a + b * c", &options);

    assert_eq!(dump, sexp!(Binary * (Binary + (Var a) (Var b)) (Var c)));
    clean(&diagnostics);
  }
}

mod prefix {
  use super::*;

  #[test]
  fn int() {
    assert_expr("42", &sexp!(IntLit raw="42"));
  }

  #[test]
  fn float() {
    assert_expr("1.5e-3", &sexp!(FloatLit raw="1.5e-3"));
  }

  #[test]
  fn bools() {
    assert_expr("true", &sexp!(BoolLit value=true));
    assert_expr("false", &sexp!(BoolLit value=false));
  }

  #[test]
  fn lower_var() {
    assert_expr("post", &sexp!(Var post));
  }

  #[test]
  fn upper_var() {
    assert_expr("String", &sexp!(Var String));
  }

  #[test]
  fn underscore_rejected() {
    let (_, diagnostics) = expr("_");

    assert_eq!(codes(&diagnostics), [UnexpectedToken]);
    assert_eq!(
      diagnostics[0].help.as_deref(),
      Some("`_` can only be used in patterns")
    );
  }

  #[test]
  fn plain_string() {
    assert_expr(
      r#""abc""#,
      &sexp!(StringLit (StringText raw="abc" value="abc")),
    );
  }

  #[test]
  fn interpolated_string() {
    assert_expr(
      r#""created #{post.id}""#,
      &sexp!(StringLit
        (StringText raw="created " value="created ")
        (StringInterp (FieldAccess id (Var post)))),
    );
  }

  #[test]
  fn escapes_keep_raw_and_decode_value() {
    assert_expr(
      r#""a\n\u{e9}""#,
      &sexp!(StringLit (StringText raw="a\\n\\u{e9}" value="a\né")),
    );
  }

  #[test]
  fn empty_interpolation() {
    let (dump, diagnostics) = expr(r##""#{}""##);

    assert_eq!(codes(&diagnostics), [UnexpectedToken]);
    assert!(diagnostics[0].message.starts_with("expected expression"));
    assert_eq!(dump, sexp!(StringLit(StringInterp(InvalidExpr))));
  }

  #[test]
  fn parens_make_no_node() {
    assert_expr("(a)", &sexp!(Var a));
  }

  #[test]
  fn empty_parens_rejected() {
    let (_, diagnostics) = expr("()");

    assert_eq!(codes(&diagnostics), [UnexpectedToken]);
    assert_eq!(
      diagnostics[0].help.as_deref(),
      Some("use `{}` for an empty value")
    );
  }

  #[test]
  fn empty_record() {
    assert_expr("{}", &sexp!(RecordLit));
  }

  #[test]
  fn record_literal() {
    assert_expr(
      "{ a: 1, b: 2 }",
      &sexp!(RecordLit
        (FieldInit a (IntLit raw="1"))
        (FieldInit b (IntLit raw="2"))),
    );
  }

  #[test]
  fn record_trailing_comma() {
    assert_expr("{ a: 1, }", &sexp!(RecordLit (FieldInit a (IntLit raw="1"))));
  }

  #[test]
  fn record_update() {
    assert_expr(
      "{ ..body, author_id: user.id }",
      &sexp!(RecordLit
        (Var body)
        (FieldInit author_id (FieldAccess id (Var user)))),
    );
  }

  #[test]
  fn nested_record_update() {
    assert_expr(
      "{ ..a, b: { ..c, d: 1 } }",
      &sexp!(RecordLit
        (Var a)
        (FieldInit b (RecordLit (Var c) (FieldInit d (IntLit raw="1"))))),
    );
  }

  #[test]
  fn spread_not_first() {
    let (module, diagnostics) =
      parse_src("functions\n  f() { { a: 1, ..b } }\n");

    assert_eq!(codes(&diagnostics), [SpreadNotFirst]);

    let Expr::Record(record) = &*first_fn(&module).body.result else {
      panic!("expected a record literal")
    };

    assert!(record.spread.is_some(), "the misplaced spread is still stored");
    assert_eq!(record.fields.len(), 1);
  }

  #[test]
  fn brace_with_a_statement_is_a_block() {
    assert_expr("{ x }", &sexp!(Block (Var x)));
  }

  #[test]
  fn duplicate_fields_are_not_an_error_here() {
    assert_expr(
      "{ a: 1, a: 2 }",
      &sexp!(RecordLit
        (FieldInit a (IntLit raw="1"))
        (FieldInit a (IntLit raw="2"))),
    );
  }

  #[test]
  fn lambda_bare() {
    assert_expr(
      "function(x) { x + 1 }",
      &sexp!(Lambda (Param x) (Block (Binary + (Var x) (IntLit raw="1")))),
    );
  }

  #[test]
  fn lambda_annotated() {
    assert_expr(
      "function(x: Int) -> Int / {Db} { x }",
      &sexp!(Lambda
        (Param x (TypeRef Int))
        (TypeRef Int)
        (EffectRow (TypeRef Db))
        (Block (Var x))),
    );
  }

  #[test]
  fn else_if_chain() {
    let (module, diagnostics) =
      parse_src("functions\n  f() { if a { 1 } else if b { 2 } else { 3 } }\n");

    clean(&diagnostics);

    let Expr::If(outer) = &*first_fn(&module).body.result else {
      panic!("expected an if")
    };
    let Else::If(inner) = &*outer.else_branch else {
      panic!("expected `else if`")
    };

    assert!(matches!(&*inner.else_branch, Else::Block(_)));
  }

  #[test]
  fn missing_else() {
    let (module, diagnostics) = parse_src("functions\n  f() { if a { 1 } }\n");

    assert_eq!(codes(&diagnostics), [MissingElse]);

    let Expr::If(node) = &*first_fn(&module).body.result else {
      panic!("expected an if")
    };
    let Else::Block(synthetic) = &*node.else_branch else {
      panic!("expected a synthesised block")
    };

    assert_eq!(synthetic.span.start, synthetic.span.end);
    assert!(matches!(&*synthetic.result, Expr::Invalid(_)));
  }

  #[test]
  fn match_arms() {
    for src in [
      r#"match x { 1 -> "a", _ -> "b" }"#,
      r#"match x { 1 -> "a", _ -> "b", }"#,
      r#"match x { 1 -> { "a" } _ -> "b" }"#,
    ] {
      let (module, diagnostics) =
        parse_src(&format!("functions\n  f() {{ {src} }}\n"));

      clean(&diagnostics);

      let Expr::Match(m) = &*first_fn(&module).body.result else {
        panic!("expected a match for `{src}`")
      };

      assert_eq!(m.arms.len(), 2, "for `{src}`");
    }
  }

  #[test]
  fn record_literal_in_a_condition() {
    let (dump, diagnostics) = expr("if { a: 1 } { 2 } else { 3 }");

    assert_eq!(codes(&diagnostics), [RecordLiteralInCondition]);
    assert_eq!(diagnostics[0].help.as_deref(), Some("wrap it in parentheses"));
    assert_eq!(
      dump,
      sexp!(If
        (RecordLit (FieldInit a (IntLit raw="1")))
        (Block (IntLit raw="2"))
        (Block (IntLit raw="3")))
    );
  }

  #[test]
  fn list_literals() {
    let (dump, diagnostics) = expr("[1, 2, ..rest]");

    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert_eq!(
      dump,
      sexp!(ListLit (IntLit raw="1") (IntLit raw="2") (Var rest))
    );

    let (_, diagnostics) = expr("[]");

    assert!(diagnostics.is_empty(), "{diagnostics:?}");
  }

  #[test]
  fn restriction_is_lifted_inside_delimiters() {
    for src in [
      "if ({ a: 1 }) { 2 } else { 3 }",
      "if f({ a: 1 }) { 2 } else { 3 }",
      "match g({ a: 1 }) { _ -> 0 }",
    ] {
      let (_, diagnostics) = expr(src);

      clean(&diagnostics);
    }
  }
}

mod blocks {
  use super::*;

  fn assert_block(src: &str, expected: &str) {
    let (dump, diagnostics) = block(src);

    assert_eq!(dump, expected, "for `{src}`");
    clean(&diagnostics);
  }

  #[test]
  fn single_expression_block() {
    assert_block("{ 1 }", &sexp!(Block (IntLit raw="1")));
  }

  #[test]
  fn let_then_result() {
    assert_block(
      "{ let x = 1\n x }",
      &sexp!(Block (LetStmt (PVar x) (IntLit raw="1")) (Var x)),
    );
  }

  #[test]
  fn let_with_annotation() {
    assert_block(
      "{ let x: Int = 1\n x }",
      &sexp!(Block (LetStmt (PVar x) (TypeRef Int) (IntLit raw="1")) (Var x)),
    );
  }

  #[test]
  fn let_with_record_pattern() {
    assert_block(
      "{ let { a: y } = r\n y }",
      &sexp!(Block
        (LetStmt (PRecord open=false (PField a (PVar y))) (Var r))
        (Var y)),
    );
  }

  #[test]
  fn several_statements() {
    assert_block(
      "{ let x = 1\n f(x)\n x }",
      &sexp!(Block
        (LetStmt (PVar x) (IntLit raw="1"))
        (ExprStmt (Call (Var f) (Var x)))
        (Var x)),
    );
  }

  #[test]
  fn nested_block_as_an_expression() {
    assert_block(
      "{ let x = { 1 }\n x }",
      &sexp!(Block (LetStmt (PVar x) (Block (IntLit raw="1"))) (Var x)),
    );
  }

  #[test]
  fn result_may_be_any_expression() {
    assert_block(
      "{ if a { 1 } else { 2 } }",
      &sexp!(Block
        (If (Var a) (Block (IntLit raw="1")) (Block (IntLit raw="2")))),
    );
  }

  #[test]
  fn statements_on_one_line() {
    let src = "functions\n  f() { let x = 1 let y = 2\n y }\n";
    let (_, diagnostics) = parse_src(src);
    let second = src.match_indices("let").nth(1).unwrap().0;

    assert_eq!(codes(&diagnostics), [StatementsOnSameLine]);
    assert_eq!(diagnostics[0].primary.span.start, second);
  }

  #[test]
  fn juxtaposition_help() {
    let (_, diagnostics) = block("{ f x }");

    assert_eq!(codes(&diagnostics), [StatementsOnSameLine]);
    assert_eq!(
      diagnostics[0].help.as_deref(),
      Some("function calls need parentheses: `f(x)`")
    );
  }

  #[test]
  fn no_newline_needed_before_close() {
    let (_, diagnostics) = block("{ let x = 1\n x}");

    clean(&diagnostics);
  }

  #[test]
  fn operators_continue_across_newlines() {
    assert_block(
      "{ let s = title\n  |> String.lowercase\n s }",
      &sexp!(Block
        (LetStmt
          (PVar s)
          (Pipe (Var title) (FieldAccess lowercase (Var String))))
        (Var s)),
    );
  }

  #[test]
  fn block_ending_in_let() {
    let (dump, diagnostics) = block("{ let x = 1 }");

    assert_eq!(codes(&diagnostics), [TrailingLet]);
    assert_eq!(
      diagnostics[0].help.as_deref(),
      Some("add the value to return after it")
    );
    assert_eq!(
      dump,
      sexp!(Block (LetStmt (PVar x) (IntLit raw="1")) (InvalidExpr))
    );
  }

  #[test]
  fn empty_block_in_block_position() {
    let (_, diagnostics) = block("{}");

    assert_eq!(codes(&diagnostics), [EmptyBlock]);
    assert_eq!(diagnostics[0].message, "empty block has no value");
  }

  #[test]
  fn local_fn_declaration() {
    let (dump, diagnostics) = block("{ function g(x) { x }\n g(1) }");

    assert_eq!(codes(&diagnostics), [LocalFnDeclaration]);
    assert_eq!(
      diagnostics[0].help.as_deref(),
      Some("use `let g = function(...) { ... }`")
    );
    assert_eq!(
      dump,
      sexp!(Block
        (LetStmt (PVar g) (Lambda (Param x) (Block (Var x))))
        (Call (Var g) (IntLit raw="1")))
    );
  }

  #[test]
  fn statement_recovery() {
    let (dump, diagnostics) = block("{ let x = \n let y = 2\n y }");

    assert_eq!(codes(&diagnostics), [UnexpectedToken]);
    assert_eq!(
      dump,
      sexp!(Block
        (LetStmt (PVar x) (InvalidExpr))
        (LetStmt (PVar y) (IntLit raw="2"))
        (Var y))
    );
  }
}

mod infix {
  use super::*;

  fn body_span(src: &str, pick: impl Fn(&Block) -> &Expr) -> (usize, usize) {
    let (module, diagnostics) = parse_src(src);

    clean(&diagnostics);

    let span = pick(&first_fn(&module).body).span();

    (span.start, span.end)
  }

  #[test]
  fn call_no_args() {
    assert_expr("f()", &sexp!(Call (Var f)));
  }

  #[test]
  fn call_args() {
    assert_expr("f(a, b)", &sexp!(Call (Var f) (Var a) (Var b)));
  }

  #[test]
  fn call_trailing_comma() {
    assert_expr("f(a,)", &sexp!(Call (Var f) (Var a)));
  }

  #[test]
  fn chained_call() {
    assert_expr("f(x)(y)", &sexp!(Call (Call (Var f) (Var x)) (Var y)));
  }

  #[test]
  fn field_then_call() {
    assert_expr(
      "a.b.c(d)",
      &sexp!(Call (FieldAccess c (FieldAccess b (Var a))) (Var d)),
    );
  }

  #[test]
  fn module_member_call() {
    assert_expr(
      "String.lowercase(s)",
      &sexp!(Call (FieldAccess lowercase (Var String)) (Var s)),
    );
  }

  #[test]
  fn upper_field_rejected() {
    let (_, diagnostics) = expr("a.B");

    assert_eq!(codes(&diagnostics), [UnexpectedToken]);
  }

  #[test]
  fn paren_on_a_new_line_is_not_a_call() {
    let (dump, diagnostics) = block("{ f\n(x)\n 0 }");

    clean(&diagnostics);
    assert_eq!(
      dump,
      sexp!(Block (ExprStmt (Var f)) (ExprStmt (Var x)) (IntLit raw="0"))
    );
  }

  #[test]
  fn paren_on_the_same_line_is_a_call() {
    let (dump, diagnostics) = block("{ f(x)\n 0 }");

    clean(&diagnostics);
    assert_eq!(
      dump,
      sexp!(Block (ExprStmt (Call (Var f) (Var x))) (IntLit raw="0"))
    );
  }

  #[test]
  fn pipe_continues_across_newlines() {
    let (dump, diagnostics) = block("{ x\n  |> f\n }");

    clean(&diagnostics);
    assert_eq!(dump, sexp!(Block (Pipe (Var x) (Var f))));
  }

  #[test]
  fn dot_continues_across_newlines() {
    let (dump, diagnostics) = block("{ x\n  .field }");

    clean(&diagnostics);
    assert_eq!(dump, sexp!(Block (FieldAccess field (Var x))));
  }

  #[test]
  fn binary_continues_across_newlines() {
    let (dump, diagnostics) = block("{ a\n  + b }");

    clean(&diagnostics);
    assert_eq!(dump, sexp!(Block (Binary + (Var a) (Var b))));
  }

  #[test]
  fn binary_span_includes_the_open_paren() {
    let src = "functions\n  f() { (a + b) * c }";

    assert_eq!(body_span(src, |b| &b.result), (18, 29));
    assert_eq!(
      body_span(src, |b| match &*b.result {
        Expr::Binary(outer) => &outer.left,
        _ => panic!("expected a binary"),
      }),
      (19, 24)
    );
  }

  #[test]
  fn call_span_covers_the_close_paren() {
    let src = "functions\n  f() { g(a) }";

    assert_eq!(body_span(src, |b| &b.result).1, src.rfind(')').unwrap() + 1);
  }

  #[test]
  fn unary_span_starts_at_the_operator() {
    let src = "functions\n  f() { -a }";

    assert_eq!(body_span(src, |b| &b.result).0, src.find('-').unwrap());
  }

  #[test]
  fn field_access_span_covers_both_sides() {
    let src = "functions\n  f() { a.b }";
    let start = src.find("a.b").unwrap();

    assert_eq!(body_span(src, |b| &b.result), (start, start + 3));
  }

  #[test]
  fn angle_is_comparison_in_expressions() {
    assert_expr("a < b", &sexp!(Binary < (Var a) (Var b)));
    assert_expr(
      "f(a < b, c)",
      &sexp!(Call (Var f) (Binary < (Var a) (Var b)) (Var c)),
    );
  }
}

mod patterns {
  use super::*;

  fn assert_pattern(src: &str, expected: &str) {
    let (dump, diagnostics) = pattern(src);

    assert_eq!(dump, expected, "for `{src}`");
    clean(&diagnostics);
  }

  #[test]
  fn wildcard() {
    assert_pattern("_", &sexp!(PWildcard));
  }

  #[test]
  fn variable() {
    assert_pattern("x", &sexp!(PVar x));
  }

  #[test]
  fn int_literal() {
    assert_pattern("1", &sexp!(PLit negative=false (IntLit raw="1")));
  }

  #[test]
  fn negative_int() {
    assert_pattern("-1", &sexp!(PLit negative=true (IntLit raw="1")));
  }

  #[test]
  fn negative_float() {
    assert_pattern("-1.5", &sexp!(PLit negative=true (FloatLit raw="1.5")));
  }

  #[test]
  fn bool_and_string_literals() {
    assert_pattern("true", &sexp!(PLit negative=false (BoolLit value=true)));
    assert_pattern(
      r#""a""#,
      &sexp!(PLit negative=false (StringLit (StringText raw="a" value="a"))),
    );
  }

  #[test]
  fn nullary_constructor() {
    assert_pattern("Red", &sexp!(PCtor Red));
  }

  #[test]
  fn constructor_with_arguments() {
    assert_pattern("Circle(r)", &sexp!(PCtor Circle (PVar r)));
  }

  #[test]
  fn nested_constructor() {
    assert_pattern(
      "Cons(x, Cons(y, Nil))",
      &sexp!(PCtor Cons (PVar x) (PCtor Cons (PVar y) (PCtor Nil))),
    );
  }

  #[test]
  fn closed_record_pattern() {
    assert_pattern(
      "{ a: x, b: _ }",
      &sexp!(PRecord open=false (PField a (PVar x)) (PField b (PWildcard))),
    );
  }

  #[test]
  fn open_record_pattern() {
    assert_pattern(
      "{ a: x, .. }",
      &sexp!(PRecord open=true (PField a (PVar x))),
    );
    assert_pattern("{ .. }", &sexp!(PRecord open=true));
  }

  #[test]
  fn interpolation_in_a_pattern() {
    let (dump, diagnostics) = pattern(r#""a #{x}""#);

    assert_eq!(codes(&diagnostics), [InterpolationInPattern]);
    assert_eq!(
      dump,
      sexp!(PLit negative=false
        (StringLit (StringText raw="a " value="a ") (StringInterp (Var x))))
    );
  }

  #[test]
  fn let_takes_a_pattern() {
    let (module, diagnostics) =
      parse_src("functions\n  f() { let Some(x) = y\n x }\n");

    clean(&diagnostics);

    let Stmt::Let(stmt) = &first_fn(&module).body.stmts[0] else {
      panic!("expected a let")
    };

    assert!(matches!(stmt.pattern, Pattern::Ctor(_)));
  }
}

mod wiring {
  use super::*;
  use std::{fs, path::Path};

  const GOAL_CREATE_POST: &str = "\
functions
  create_post(body: NewPost) -> Post / {Db, Auth, Throws<Invalid>} {
    let user = Auth.require()
    let post = Db.insert(Posts, { ..body, author_id: user.id })
    Log.info(\"created #{post.id}\")
    post
  }
";

  fn examples() -> Vec<(String, String)> {
    let dir =
      Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/programs");
    let mut out: Vec<_> = fs::read_dir(dir)
      .expect("read tests/fixtures/programs/")
      .map(|entry| entry.unwrap().path())
      .filter(|path| path.extension().is_some_and(|ext| ext == "px"))
      .map(|path| {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();

        (name, fs::read_to_string(&path).unwrap())
      })
      .collect();

    out.sort();
    out
  }

  #[test]
  fn goal_create_post() {
    let (module, diagnostics) = parse_src(GOAL_CREATE_POST);
    let body = &first_fn(&module).body;

    clean(&diagnostics);
    assert_eq!(body.stmts.len(), 3);
    assert!(matches!(&*body.result, Expr::Var(v) if v.name.text == "post"));
  }

  #[test]
  fn goal_slug() {
    let src = "\
functions
  slug(r: { title: String | rest }) -> String {
    r.title |> String.lowercase |> String.replace(\" \", \"-\")
  }
";
    let (module, diagnostics) = parse_src(src);

    clean(&diagnostics);
    assert_eq!(
      shape(first_fn(&module).body.result.as_node()),
      sexp!(Pipe
        (Pipe
          (FieldAccess title (Var r))
          (FieldAccess lowercase (Var String)))
        (Call
          (FieldAccess replace (Var String))
          (StringLit (StringText raw=" " value=" "))
          (StringLit (StringText raw="-" value="-"))))
    );
  }

  #[test]
  fn blog_example_parses_cleanly() {
    let (_, blog) = examples()
      .into_iter()
      .find(|(name, _)| name == "blog.px")
      .expect("compiler/tests/fixtures/programs/blog.px");
    let (module, diagnostics) = parse_src(&blog);

    clean(&diagnostics);
    assert_eq!(module.zones.len(), 4);
  }

  #[test]
  fn body_recovery_is_statement_local() {
    let src = "functions\n  f() {\n    let a = 1\n    let b = )\n    a\n  }\n";
    let (dump, diagnostics) = block_of(src);

    assert_eq!(codes(&diagnostics), [UnexpectedToken]);
    assert_eq!(
      dump,
      sexp!(Block
        (LetStmt (PVar a) (IntLit raw="1"))
        (LetStmt (PVar b) (InvalidExpr))
        (Var a))
    );
  }

  fn block_of(src: &str) -> (String, Vec<Diagnostic>) {
    let (module, diagnostics) = parse_src(src);

    (shape(first_fn(&module).body.as_node()), diagnostics)
  }

  #[test]
  fn unclosed_block_falls_back() {
    let src = "functions\n  f() {\n    let a = 1\n  g() { 2 }\n";
    let (module, diagnostics) = parse_src(src);

    assert_eq!(codes(&diagnostics), [UnexpectedToken]);
    assert_eq!(diagnostics[0].message, "this function body is never closed");

    let names: Vec<_> = module.zones[0]
      .decls
      .iter()
      .map(|decl| match decl {
        Decl::Fn(f) => f.name.text.as_str(),
        other => panic!("expected a function, got {other:?}"),
      })
      .collect();

    assert_eq!(names, ["g"], "the next function still parses");
  }

  #[test]
  fn unclosed_block_stops_at_the_next_zone() {
    let src = "functions\n  f() {\n    1\nexports\n  f\n";
    let (module, _) = parse_src(src);

    assert_eq!(module.zones.len(), 2);
  }

  #[test]
  fn span_invariant() {
    for (name, src) in examples() {
      let file = SourceFile::new(&name, src.as_str());
      let mut bag = DiagnosticBag::default();
      let lexed = lex(&file, &mut bag);
      let module = parse(&file, &lexed, &mut bag);

      check_invariants(&src, &module);
    }
  }

  #[test]
  fn parse_with_default_equals_parse() {
    for (name, src) in examples() {
      let file = SourceFile::new(&name, src.as_str());
      let mut bag = DiagnosticBag::default();
      let lexed = lex(&file, &mut bag);
      let plain = parse(&file, &lexed, &mut bag);
      let with = parse_with(&file, &lexed, &mut bag, &ParseOptions::default());

      assert_eq!(plain, with, "{name}");
    }
  }

  #[test]
  fn codes_are_registered() {
    let new: Vec<u16> = DiagnosticCode::ALL
      .iter()
      .map(|&code| code as u16)
      .filter(|&n| (211..=220).contains(&n))
      .collect();

    assert_eq!(new, (211..=220).collect::<Vec<_>>());
    assert!(
      DiagnosticCode::ALL
        .iter()
        .all(|&c| c as u16 / 100 != 2 || c as u16 <= 299)
    );
  }
}

mod properties {
  use super::*;
  use proptest::prelude::*;
  use std::thread;

  const PIECES: &[&str] = &[
    "a", "b", "f", "Post", "String", "1", "2.5", "true", "\"s\"", "\"#{x}\"",
    "(", ")", "{", "}", "[", "]", ",", ":", ".", "..", "->", "|>", "=", "==",
    "<", ">", "+", "-", "*", "!", "&&", "||", "_", "let", "if", "else",
    "match", "function",
  ];

  const SEPARATORS: &[&str] = &[" ", " ", " ", "\n    "];

  fn body_soup() -> impl Strategy<Value = String> {
    prop::collection::vec(
      (prop::sample::select(PIECES), prop::sample::select(SEPARATORS)),
      0..80,
    )
    .prop_map(|pieces| {
      let body: String = pieces.into_iter().flat_map(|(p, s)| [p, s]).collect();

      format!("functions\n  f() {{ {body} }}\n  g() {{ 0 }}\n")
    })
  }

  fn on_small_stack(src: String) {
    thread::Builder::new()
      .stack_size(1024 * 1024)
      .spawn(move || drop(parse_src(&src)))
      .expect("spawn the parser thread")
      .join()
      .expect("parsing and dropping on a small stack completed");
  }

  proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn body_soup_parses_with_valid_spans(src in body_soup()) {
      parse_src(&src);
    }
  }

  #[test]
  fn deep_nesting_parses_and_drops_on_a_small_stack() {
    for open in [
      "(",
      "{",
      "-",
      "!",
      "f(",
      "a.b(",
      "{ a: ",
      "{ ..a, b: ",
      "\"#{",
      "function(x) { ",
      "{ let x = ",
      "if a { ",
      "if a { 1 } else ",
      "match x { _ -> ",
      "match x { _ -> { ",
    ] {
      on_small_stack(format!("functions\n  f() {{ {} }}\n", open.repeat(3000)));
    }

    on_small_stack(format!(
      "functions\n  f() {{ match x {{ {} -> 0 }} }}\n",
      "Cons(".repeat(3000)
    ));
  }

  #[test]
  fn long_chains_stay_shallow() {
    for (piece, sep) in
      [(".b", ""), ("(x)", ""), (" + b", ""), (" |> f", ""), (" && b", "")]
    {
      let chain = format!("a{}", format!("{piece}{sep}").repeat(10_000));

      on_small_stack(format!("functions\n  f() {{ {chain} }}\n"));
    }
  }
}
