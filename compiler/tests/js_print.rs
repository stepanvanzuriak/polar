mod common;

use std::sync::{LazyLock, Mutex};

use common::node::{Evaluator, node_check_module};
use polar_compiler::{
  backend::js::{
    ast::{
      BinaryOp, Expr, Item, Literal, LiteralValue, LogicalOp, Program, Stmt,
      Switch, SwitchCase, UnaryOp, arrow, assign, await_, binary, boolean,
      break_, call, computed, conditional, const_, expr_stmt, function, ident,
      logical, member, number, object, prop, ret, spread, string, template,
      unary, var,
    },
    print::{number_to_string, print_expr, print_program},
  },
  shared::source::Span,
};

fn expr(e: &Expr) -> String {
  let module = program(vec![Item::Stmt(const_(ident("v"), e.clone()))]);

  node_check_module(&module).unwrap_or_else(|err| panic!("{module}\n{err}"));
  print_expr(e)
}

fn program(items: Vec<Item>) -> String {
  let code = print_program(&Program { items }).code;

  node_check_module(&code).unwrap_or_else(|err| panic!("{code}\n{err}"));
  code
}

fn in_function(body: Vec<Stmt>) -> String {
  program(vec![Item::Function(function(ident("f"), vec![], body, false))])
}

fn stmts(stmts: Vec<Stmt>) -> String {
  program(stmts.into_iter().map(Item::Stmt).collect())
}

fn a() -> Expr {
  var("a")
}

fn b() -> Expr {
  var("b")
}

fn c() -> Expr {
  var("c")
}

#[test]
fn binary_needs_parens() {
  let e = binary(BinaryOp::Mul, binary(BinaryOp::Add, a(), b()), c());

  assert_eq!(expr(&e), "(a + b) * c");
}

#[test]
fn binary_needs_none() {
  let e = binary(BinaryOp::Add, a(), binary(BinaryOp::Mul, b(), c()));

  assert_eq!(expr(&e), "a + b * c");
}

#[test]
fn right_operand_of_equal_precedence_is_parenthesised() {
  let e = binary(BinaryOp::Sub, a(), binary(BinaryOp::Sub, b(), c()));

  assert_eq!(expr(&e), "a - (b - c)");
  assert_eq!(
    expr(&binary(BinaryOp::Sub, binary(BinaryOp::Sub, a(), b()), c())),
    "a - b - c"
  );
}

#[test]
fn logical_precedence() {
  let e = logical(LogicalOp::Or, a(), logical(LogicalOp::And, b(), c()));

  assert_eq!(expr(&e), "a || b && c");
  assert_eq!(
    expr(&logical(LogicalOp::And, logical(LogicalOp::Or, a(), b()), c())),
    "(a || b) && c"
  );
}

#[test]
fn small_object_on_one_line() {
  let e = object(vec![
    prop("a", number(1.0)),
    prop("b", number(2.0)),
    prop("c", number(3.0)),
  ]);

  assert_eq!(expr(&e), "{ a: 1, b: 2, c: 3 }");
}

#[test]
fn large_object_breaks() {
  let e = object(vec![
    prop("a", number(1.0)),
    prop("b", number(2.0)),
    prop("c", number(3.0)),
    prop("d", number(4.0)),
  ]);

  assert_eq!(expr(&e), "{\n  a: 1,\n  b: 2,\n  c: 3,\n  d: 4,\n}");
}

#[test]
fn nested_object_breaks() {
  let e = object(vec![
    prop("a", number(1.0)),
    prop("b", object(vec![prop("c", number(2.0))])),
  ]);

  assert_eq!(expr(&e), "{\n  a: 1,\n  b: { c: 2 },\n}");
}

#[test]
fn broken_object_indents_with_its_statement() {
  let body = vec![ret(object(vec![
    prop("a", number(1.0)),
    prop("f", arrow(vec![], number(1.0), false)),
  ]))];

  assert_eq!(
    in_function(body),
    "function f() {\n  return {\n    a: 1,\n    f: () => 1,\n  };\n}\n"
  );
}

#[test]
fn object_spread() {
  let e = object(vec![spread(var("body")), prop("author_id", var("x"))]);

  assert_eq!(expr(&e), "{ ...body, author_id: x }");
}

#[test]
fn member_chain() {
  assert_eq!(expr(&member(member(a(), "b"), "c")), "a.b.c");
}

#[test]
fn switch_and_break() {
  let case = |tag: &str, value: f64| SwitchCase {
    test: Literal {
      value: LiteralValue::String(tag.to_string()),
      origin: None,
    },
    body: vec![expr_stmt(assign(ident("r"), number(value))), break_()],
    origin: None,
  };
  let switch = Stmt::Switch(Switch {
    discriminant: member(var("s"), "$"),
    cases: vec![case("A", 1.0), case("B", 2.0)],
    default: Some(vec![expr_stmt(assign(ident("r"), number(0.0)))]),
    origin: None,
  });

  assert_eq!(
    stmts(vec![switch]),
    "switch (s.$) {\n  case \"A\":\n    r = 1;\n    break;\n  \
     case \"B\":\n    r = 2;\n    break;\n  default:\n    r = 0;\n}\n"
  );
}

#[test]
fn async_function_with_await() {
  let mut g = function(
    ident("g"),
    vec![],
    vec![ret(await_(call(var("f"), vec![], None)))],
    true,
  );

  g.exported = true;

  assert_eq!(
    program(vec![Item::Function(g)]),
    "export async function g() {\n  return await f();\n}\n"
  );
}

#[test]
fn top_level_items_are_separated_by_a_blank_line() {
  let f = |name: &str| {
    Item::Function(function(ident(name), vec![], vec![ret(number(1.0))], false))
  };

  assert_eq!(
    program(vec![f("f"), f("g")]),
    "function f() {\n  return 1;\n}\n\nfunction g() {\n  return 1;\n}\n"
  );
}

#[test]
fn nested_arrows() {
  let e =
    arrow(vec![ident("x")], arrow(vec![ident("y")], var("x"), false), true);

  assert_eq!(expr(&e), "async (x) => (y) => x");
}

#[test]
fn template_literal() {
  let e = template(vec!["created ".into(), String::new()], vec![var("x")]);

  assert_eq!(expr(&e), "`created ${x}`");
}

#[test]
fn else_if_is_flattened() {
  use polar_compiler::backend::js::ast::if_;

  let code = in_function(vec![if_(
    a(),
    vec![ret(number(1.0))],
    Some(vec![if_(b(), vec![ret(number(2.0))], Some(vec![ret(number(3.0))]))]),
  )]);

  assert_eq!(
    code,
    "function f() {\n  if (a) {\n    return 1;\n  } else if (b) {\n    \
     return 2;\n  } else {\n    return 3;\n  }\n}\n"
  );
}

#[test]
fn arrow_returning_an_object() {
  let e = arrow(vec![], object(vec![prop("a", number(1.0))]), false);

  assert_eq!(expr(&e), "() => ({ a: 1 })");
}

#[test]
fn statement_starting_with_a_brace() {
  let s = expr_stmt(object(vec![prop("a", number(1.0))]));

  assert_eq!(stmts(vec![s]), "({ a: 1 });\n");

  let s = expr_stmt(member(object(vec![prop("a", number(1.0))]), "a"));

  assert_eq!(stmts(vec![s]), "({ a: 1 }.a);\n");
}

#[test]
fn statement_starting_with_function() {
  let s = expr_stmt(arrow(vec![], var("x"), true));

  assert_eq!(stmts(vec![s]), "async () => x;\n");
}

#[test]
fn statement_starting_with_let_bracket() {
  let s = expr_stmt(computed(var("let"), number(0.0)));

  assert_eq!(stmts(vec![s]), "let$[0];\n");
}

#[test]
fn arrow_as_callee() {
  let e = call(arrow(vec![], var("x"), true), vec![], None);

  assert_eq!(expr(&e), "(async () => x)()");
}

#[test]
fn await_as_member_target() {
  let e = member(await_(call(var("f"), vec![], None)), "x");

  assert_eq!(expr(&e), "(await f()).x");
}

#[test]
fn conditional_in_a_binary_operand() {
  let e = binary(BinaryOp::Add, conditional(a(), b(), c()), var("d"));

  assert_eq!(expr(&e), "(a ? b : c) + d");
}

#[test]
fn conditional_test_and_branches() {
  let nested =
    conditional(conditional(a(), b(), c()), a(), conditional(b(), c(), a()));

  assert_eq!(expr(&nested), "(a ? b : c) ? a : b ? c : a");
}

#[test]
fn assign_in_a_binary_operand() {
  let e = binary(BinaryOp::Add, assign(ident("x"), number(1.0)), var("y"));

  assert_eq!(expr(&e), "(x = 1) + y");
}

#[test]
fn adjacent_minus() {
  let e = binary(BinaryOp::Sub, a(), unary(UnaryOp::Negate, b()));

  assert_eq!(expr(&e), "a - -b");
  assert_eq!(
    expr(&unary(UnaryOp::Negate, unary(UnaryOp::Negate, a()))),
    "- -a"
  );
  assert_eq!(expr(&number(-1.0)), "-1");
}

#[test]
fn numeric_member_target() {
  assert_eq!(expr(&member(number(1.0), "x")), "(1).x");
}

#[test]
fn await_inside_unary() {
  let e = unary(UnaryOp::Not, await_(call(var("f"), vec![], None)));

  assert_eq!(expr(&e), "!(await f())");
}

#[test]
fn await_binds_tighter_than_binary() {
  let e = binary(
    BinaryOp::Add,
    number(1.0),
    await_(call(var("length"), vec![var("tail")], None)),
  );

  assert_eq!(expr(&e), "1 + await length(tail)");
}

#[test]
fn string_escaping() {
  let e = string("q\"b\\n\n\u{1}é😀");

  assert_eq!(expr(&e), "\"q\\\"b\\\\n\\n\\u0001é😀\"");
}

#[test]
fn template_escaping() {
  let e = template(vec!["a`b\\c${d $e\r".into()], vec![]);

  assert_eq!(expr(&e), "`a\\`b\\\\c\\${d $e\\r`");
}

#[test]
fn number_layout() {
  let cases = [
    (1e20, "100000000000000000000"),
    (1e21, "1e+21"),
    (1e-6, "0.000001"),
    (1e-7, "1e-7"),
    (0.1 + 0.2, "0.30000000000000004"),
    (0.0, "0"),
    (123.456, "123.456"),
    (1.5e-10, "1.5e-10"),
    (1.2345e25, "1.2345e+25"),
    (f64::MAX, "1.7976931348623157e+308"),
    (5e-324, "5e-324"),
  ];

  for (n, text) in cases {
    assert_eq!(number_to_string(n), text, "{n:e}");
  }

  assert_eq!(expr(&number(1e21)), "1e+21");
}

fn span(start: usize) -> Span {
  Span { file: "a.px".into(), start, end: start + 1 }
}

#[test]
fn mapped_nodes_record_their_first_character() {
  let e = call(
    var("f").at(Some(span(3))),
    vec![var("x").at(Some(span(5)))],
    Some(span(3)),
  );
  let printed =
    print_program(&Program { items: vec![Item::Stmt(expr_stmt(e))] });
  let points: Vec<(usize, usize)> =
    printed.mappings.iter().map(|m| (m.gen_line, m.gen_column)).collect();

  assert_eq!(printed.code, "f(x);\n");
  assert_eq!(points, [(0, 0), (0, 0), (0, 2)]);
}

#[test]
fn columns_count_utf16_units() {
  let e = call(var("f"), vec![string("😀"), var("x").at(Some(span(9)))], None);
  let printed =
    print_program(&Program { items: vec![Item::Stmt(expr_stmt(e))] });

  assert_eq!(printed.mappings[0].gen_column, 8);
}

#[test]
fn unmapped_node_after_a_mapped_one_gets_a_marker() {
  let e = binary(BinaryOp::Add, var("a").at(Some(span(0))), var("$t"));
  let printed =
    print_program(&Program { items: vec![Item::Stmt(expr_stmt(e))] });
  let marks: Vec<(usize, bool)> = printed
    .mappings
    .iter()
    .map(|m| (m.gen_column, m.origin.is_some()))
    .collect();

  assert_eq!(marks, [(0, true), (4, false)]);
}

#[test]
fn mangled_identifier_carries_its_source_name() {
  let printed = print_program(&Program {
    items: vec![Item::Stmt(expr_stmt(var("class").at(Some(span(0)))))],
  });

  assert_eq!(printed.code, "class$;\n");
  assert_eq!(printed.mappings[0].name.as_deref(), Some("class"));
}

static EVALUATOR: LazyLock<Mutex<Evaluator>> =
  LazyLock::new(|| Mutex::new(Evaluator::new()));

#[derive(Debug, Clone, Copy)]
enum Value {
  Number(f64),
  Bool(bool),
}

fn to_number(v: Value) -> f64 {
  match v {
    Value::Number(n) => n,
    Value::Bool(b) => f64::from(u8::from(b)),
  }
}

#[allow(
  clippy::cast_possible_truncation,
  clippy::cast_possible_wrap,
  clippy::cast_sign_loss
)]
fn to_i32(n: f64) -> i32 {
  if n.is_finite() {
    n.trunc().rem_euclid(4_294_967_296.0) as u32 as i32
  } else {
    0
  }
}

fn truthy(v: Value) -> bool {
  match v {
    Value::Number(n) => n != 0.0 && !n.is_nan(),
    Value::Bool(b) => b,
  }
}

fn eval(e: &Expr) -> Value {
  match e {
    Expr::Literal(Literal { value: LiteralValue::Number(n), .. }) => {
      Value::Number(*n)
    }
    Expr::Literal(Literal { value: LiteralValue::Bool(b), .. }) => {
      Value::Bool(*b)
    }
    Expr::Binary(bin) => {
      let (left, right) = (eval(&bin.left), eval(&bin.right));
      let (x, y) = (to_number(left), to_number(right));

      match bin.op {
        BinaryOp::Add => Value::Number(x + y),
        BinaryOp::Sub => Value::Number(x - y),
        BinaryOp::Mul => Value::Number(x * y),
        BinaryOp::Div => Value::Number(x / y),
        BinaryOp::Mod => Value::Number(x % y),
        BinaryOp::Lt => Value::Bool(x < y),
        BinaryOp::Le => Value::Bool(x <= y),
        BinaryOp::Gt => Value::Bool(x > y),
        BinaryOp::Ge => Value::Bool(x >= y),
        BinaryOp::StrictEq => Value::Bool(strict_eq(left, right)),
        BinaryOp::StrictNe => Value::Bool(!strict_eq(left, right)),
        BinaryOp::BitAnd => Value::Number(f64::from(to_i32(x) & to_i32(y))),
        BinaryOp::BitOr => Value::Number(f64::from(to_i32(x) | to_i32(y))),
        BinaryOp::BitXor => Value::Number(f64::from(to_i32(x) ^ to_i32(y))),
      }
    }
    Expr::Logical(log) => {
      let l = eval(&log.left);

      match (log.op, truthy(l)) {
        (LogicalOp::And, false) | (LogicalOp::Or, true) => l,
        (LogicalOp::And, true) | (LogicalOp::Or, false) => eval(&log.right),
      }
    }
    Expr::Unary(u) => match u.op {
      UnaryOp::Negate => Value::Number(-to_number(eval(&u.arg))),
      UnaryOp::Not => Value::Bool(!truthy(eval(&u.arg))),
      UnaryOp::BitNot => {
        Value::Number(f64::from(!to_i32(to_number(eval(&u.arg)))))
      }
    },
    Expr::Conditional(c) => {
      if truthy(eval(&c.test)) {
        eval(&c.consequent)
      } else {
        eval(&c.alternate)
      }
    }
    other => panic!("not generated: {other:?}"),
  }
}

fn strict_eq(l: Value, r: Value) -> bool {
  match (l, r) {
    (Value::Number(x), Value::Number(y)) => x == y,
    (Value::Bool(x), Value::Bool(y)) => x == y,
    (Value::Number(_), Value::Bool(_)) | (Value::Bool(_), Value::Number(_)) => {
      false
    }
  }
}

fn render(v: Value) -> String {
  match v {
    Value::Bool(b) => b.to_string(),
    Value::Number(n) if n.is_nan() => "NaN".to_string(),
    Value::Number(n) if n == 0.0 && n.is_sign_negative() => "-0".to_string(),
    Value::Number(n) if n.is_infinite() => {
      if n > 0.0 { "Infinity" } else { "-Infinity" }.to_string()
    }
    Value::Number(n) if n < 0.0 => format!("-{}", number_to_string(-n)),
    Value::Number(n) => number_to_string(n),
  }
}

mod property {
  use super::*;
  use proptest::prelude::*;

  fn tree() -> impl Strategy<Value = Expr> {
    let leaf = prop_oneof![
      prop::sample::select(vec![
        0.0, 1.0, 2.0, 3.0, 0.5, 7.0, 10.0, 1e21, 1e-7
      ])
      .prop_map(number),
      any::<bool>().prop_map(boolean),
    ];

    leaf.prop_recursive(6, 64, 3, |inner| {
      prop_oneof![
        (
          prop::sample::select(vec![
            BinaryOp::Add,
            BinaryOp::Sub,
            BinaryOp::Mul,
            BinaryOp::Div,
            BinaryOp::Mod,
            BinaryOp::Lt,
            BinaryOp::Le,
            BinaryOp::Gt,
            BinaryOp::Ge,
            BinaryOp::StrictEq,
            BinaryOp::StrictNe,
            BinaryOp::BitAnd,
            BinaryOp::BitOr,
            BinaryOp::BitXor,
          ]),
          inner.clone(),
          inner.clone()
        )
          .prop_map(|(op, l, r)| binary(op, l, r)),
        (
          prop::sample::select(vec![LogicalOp::And, LogicalOp::Or]),
          inner.clone(),
          inner.clone()
        )
          .prop_map(|(op, l, r)| logical(op, l, r)),
        (
          prop::sample::select(vec![
            UnaryOp::Negate,
            UnaryOp::Not,
            UnaryOp::BitNot
          ]),
          inner.clone()
        )
          .prop_map(|(op, arg)| unary(op, arg)),
        (inner.clone(), inner.clone(), inner)
          .prop_map(|(t, c, a)| conditional(t, c, a)),
      ]
    })
  }

  proptest! {
    #![proptest_config(ProptestConfig::with_cases(1000))]

    #[test]
    fn printed_expressions_evaluate_like_the_tree(e in tree()) {
      let js = print_expr(&e);
      let got = EVALUATOR.lock().unwrap_or_else(std::sync::PoisonError::into_inner).eval(&js);

      prop_assert_eq!(got, render(eval(&e)), "{}", js);
    }
  }
}
