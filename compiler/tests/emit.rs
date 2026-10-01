mod common;

use common::node::node_check_module;
use polar_compiler::{
  CompileOptions,
  backend::codegen::emit::emit,
  backend::js::{
    ast::{Expr, Item, Program, Stmt},
    print::print_program,
  },
  core::{lower, matching::compile_matches},
  shared::diagnostic::DiagnosticBag,
  shared::source::SourceFile,
  syntax::lexer::lex,
  syntax::parser::parse,
};

const TYPES: &str = "\
types
  Shape = Circle(Float) | Square(Float, Float) | Red
";

fn program(src: &str) -> Program {
  let file = SourceFile::new("test.px", src);
  let mut bag = DiagnosticBag::default();
  let lexed = lex(&file, &mut bag);
  let module = parse(&file, &lexed, &mut bag);
  let core = lower(&module, &mut bag);

  assert!(!bag.has_errors(), "the fixture must lower: {:?}", bag.into_sorted());

  let program = emit(&compile_matches(core), &file, &CompileOptions::default());
  let printed = print_program(&program).code;

  node_check_module(&printed).unwrap_or_else(|err| panic!("{printed}\n{err}"));
  program
}

fn function(program: &Program, name: &str) -> String {
  for item in &program.items {
    if let Item::Function(f) = item
      && f.name.name == name
    {
      return print_program(&Program { items: vec![item.clone()] }).code;
    }
  }

  panic!("no function {name} in {}", print_program(program).code);
}

fn body(expr: &str) -> String {
  let src = format!(
    "{TYPES}functions\n  t(a, b, c, f, g, x, xs, body, p) {{ {expr} }}\n  \
     map(xs, f) {{ xs }}\n  h() {{ 1 }}\n"
  );
  let text = function(&program(&src), "t");

  text
    .lines()
    .skip(1)
    .take_while(|line| *line != "}")
    .map(|line| line.strip_prefix("  ").unwrap_or(line))
    .collect::<Vec<_>>()
    .join("\n")
}

#[test]
fn top_level_fn_is_async() {
  let js = function(&program("functions\n  f() { 1 }\nexports\n  f\n"), "f");

  assert_eq!(js, "export async function f() {\n  return 1;\n}\n");
}

#[test]
fn private_fn_is_not_exported() {
  let js = function(&program("functions\n  f() { 1 }\n"), "f");

  assert_eq!(js, "async function f() {\n  return 1;\n}\n");
}

#[test]
fn user_call_is_awaited() {
  let js = function(&program("functions\n  f() { 1 }\n  g() { f() }\n"), "g");

  assert_eq!(js, "async function g() {\n  return await f($rt.ASYNC);\n}\n");
}

#[test]
fn builtin_call_is_not_awaited() {
  assert_eq!(body("Log.info(\"x\")\n 1"), "$rt.Log.info(\"x\");\nreturn 1;");
}

#[test]
fn if_in_statement_position() {
  assert_eq!(
    body("if c { a } else { b }"),
    "if (c) {\n  return a;\n} else {\n  return b;\n}"
  );
}

#[test]
fn if_in_expression_position() {
  assert_eq!(
    body("f(if c { a } else { b })"),
    "return await f(c ? a : b, $rt.ASYNC);"
  );
}

#[test]
fn if_in_expression_position_with_statements() {
  assert_eq!(
    body("f(if c { let y = h()\n y } else { b })"),
    "let $t;\n\
     if (c) {\n  const y = await h($rt.ASYNC);\n  $t = y;\n} else {\n  $t = b;\n}\n\
     return await f($t, $rt.ASYNC);"
  );
}

#[test]
fn lambda_is_an_async_arrow() {
  assert_eq!(body("function(x) { x + 1 }"), "return async (x$1) => x$1 + 1;");
}

#[test]
fn lambda_with_statements_has_a_block_body() {
  assert_eq!(
    body("function(y) { let z = y\n z }"),
    "return async (y) => {\n  const z = y;\n  return z;\n};"
  );
}

#[test]
fn record_literal() {
  assert_eq!(body("{ a: 1, b: 2 }"), "return { a: 1, b: 2 };");
}

#[test]
fn record_update_is_a_spread() {
  assert_eq!(
    body("{ ..body, author_id: x }"),
    "return { ...body, author_id: x };"
  );
}

#[test]
fn variant_construction() {
  assert_eq!(body("Circle(1.0)"), "return { $: \"Circle\", _0: 1 };");
}

#[test]
fn nullary_variant() {
  assert_eq!(body("Red"), "return { $: \"Red\" };");
}

#[test]
fn ctor_as_a_value() {
  assert_eq!(
    body("map(xs, Circle)"),
    "return await map(xs, (_0) => ({ $: \"Circle\", _0: _0 }), $rt.ASYNC);"
  );
}

#[test]
fn equality_without_evidence_is_strict() {
  assert_eq!(body("a == b"), "return a === b;");
  assert_eq!(body("a != b"), "return a !== b;");
}

#[test]
fn interpolation_is_a_template() {
  assert_eq!(body("\"id #{p.id}\""), "return `id ${p.id}`;");
}

#[test]
fn shadowing_gets_a_suffix() {
  assert_eq!(
    body("let y = 1\n let y = y + 1\n y"),
    "const y = 1;\nconst y$1 = y + 1;\nreturn y$1;"
  );
}

#[test]
fn match_is_an_if_chain() {
  assert_eq!(
    body("match p { Circle(r) -> r, Red -> 0.0 }"),
    "if (p.$ === \"Circle\") {\n  const r = p._0;\n  return r;\n} \
     else if (p.$ === \"Red\") {\n  return 0;\n} else {\n  \
     $rt.matchFailure(\"test.px\", 4, 38);\n}"
  );
}

#[test]
fn exhaustive_match_skips_the_last_test() {
  assert_eq!(
    body("match p { Circle(r) -> r, Square(w, _) -> w, Red -> 0.0 }"),
    "if (p.$ === \"Circle\") {\n  const r = p._0;\n  return r;\n} \
     else if (p.$ === \"Square\") {\n  const w = p._0;\n  return w;\n} \
     else {\n  return 0;\n}"
  );
}

#[test]
fn nested_test_keeps_the_last_test() {
  assert_eq!(
    body(
      "match p { Circle(1.0) -> 1.0, Square(w, _) -> w, Red -> 0.0, Circle(r) -> r }"
    ),
    "if (p.$ === \"Circle\" && p._0 === 1) {\n  return 1;\n} \
     else if (p.$ === \"Square\") {\n  const w = p._0;\n  return w;\n} \
     else if (p.$ === \"Red\") {\n  return 0;\n} \
     else if (p.$ === \"Circle\") {\n  const r = p._0;\n  return r;\n} else {\n  \
     $rt.matchFailure(\"test.px\", 4, 38);\n}"
  );
}

#[test]
fn returning_if_guards_a_longer_else() {
  assert_eq!(
    body("if c { a } else { let y = h()\n y }"),
    "if (c) {\n  return a;\n}\nconst y = await h($rt.ASYNC);\nreturn y;"
  );
}

#[test]
fn match_failure_column_counts_code_points() {
  let src = format!(
    "{TYPES}functions\n  t(f, s) {{\n    f(\"é🎉\", match s {{ Red -> 1 }})\n  }}\n"
  );
  let js = function(&program(&src), "t");
  let line = src.lines().position(|l| l.contains("match s")).unwrap() + 1;
  let text = src.lines().nth(line - 1).unwrap();
  let column = text[..text.find("match").unwrap()].chars().count() + 1;

  assert!(
    js.contains(&format!("$rt.matchFailure(\"test.px\", {line}, {column});")),
    "{js}"
  );
  assert_eq!(column, 13);
}

#[test]
fn let_of_an_if_declares_then_assigns() {
  assert_eq!(
    body("let y = if c { let z = h()\n z } else { b }\n y"),
    "let y;\nif (c) {\n  const z = await h($rt.ASYNC);\n  y = z;\n} else {\n  y = b;\n}\n\
     return y;"
  );
}

#[test]
fn let_of_a_plain_if_is_a_conditional() {
  assert_eq!(
    body("let y = if c { a } else { b }\n y"),
    "const y = c ? a : b;\nreturn y;"
  );
}

#[test]
fn hoisting_keeps_evaluation_order() {
  assert_eq!(
    body("f(h(), if c { let y = h()\n y } else { b })"),
    "const $t = await h($rt.ASYNC);\n\
     let $t1;\n\
     if (c) {\n  const y = await h($rt.ASYNC);\n  $t1 = y;\n} else {\n  $t1 = b;\n}\n\
     return await f($t, $t1, $rt.ASYNC);"
  );
}

#[test]
fn pure_operands_are_not_bound() {
  assert_eq!(
    body("f(a, if c { let y = h()\n y } else { b })"),
    "let $t;\n\
     if (c) {\n  const y = await h($rt.ASYNC);\n  $t = y;\n} else {\n  $t = b;\n}\n\
     return await f(a, $t, $rt.ASYNC);"
  );
}

#[test]
fn logical_keeps_its_short_circuit() {
  assert_eq!(
    body("c && if a { let y = h()\n y == 1 } else { false }"),
    "let $t;\n\
     if (c) {\n  if (a) {\n    const y = await h($rt.ASYNC);\n    $t = y === 1;\n  } \
     else {\n    $t = false;\n  }\n} else {\n  $t = false;\n}\n\
     return $t;"
  );
}

#[test]
fn local_never_shadows_a_top_level_function() {
  let js = function(
    &program("functions\n  h() { 1 }\n  t() { let h = h()\n h }\n"),
    "t",
  );

  assert_eq!(
    js,
    "async function t() {\n  const h$1 = await h($rt.ASYNC);\n  return h$1;\n}\n"
  );
}

#[test]
fn discarded_pure_value_emits_nothing() {
  assert_eq!(body("let _ = a\n b"), "return b;");
}

#[test]
fn runtime_is_imported() {
  let program = program("functions\n  f() { 1 }\n");

  assert_eq!(
    print_program(&program).code.lines().next(),
    Some("import * as $rt from \"./_polar/runtime.js\";")
  );
}

#[test]
fn origins_mark_user_code_only() {
  let program =
    program("functions\n  f(a, b) { g(a, b == a) }\n  g(a, b) { a }\n");
  let Some(Item::Function(f)) = program.items.get(1) else {
    panic!("expected f");
  };
  let Stmt::Return(ret) = &f.body[0] else { panic!("expected a return") };
  let Some(Expr::Await(await_)) = &ret.arg else { panic!("expected await") };
  let Expr::Call(user_call) = &*await_.arg else { panic!("expected a call") };
  let Expr::Binary(_) = &user_call.args[1] else { panic!("expected ===") };

  assert!(ret.origin.is_some());
  assert!(user_call.origin.is_some());
  assert!(user_call.args[0].origin().is_some(), "the identifier `a`");
  assert!(user_call.args[1].origin().is_some(), "the comparison `b == a`");
}
