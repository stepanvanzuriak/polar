use polar_compiler::{
  core::{
    dump::{dump_expr, dump_expr_shape},
    ir::{CExpr, CExprKind, CModule},
    lower,
    matching::compile_matches,
  },
  shared::diagnostic::DiagnosticBag,
  shared::source::SourceFile,
  syntax::lexer::lex,
  syntax::parser::parse,
};

const TYPES: &str = "\
types
  Shape = Circle(Float) | Square(Float, Float) | Red
  List<a> = Nil | Cons(a, List<a>)
  Triple = T(Int, Int, Int)
  Wrap = W(Triple)
";

fn lowered(src: &str) -> CModule {
  let file = SourceFile::new("test.px", src);
  let mut bag = DiagnosticBag::default();
  let lexed = lex(&file, &mut bag);
  let module = parse(&file, &lexed, &mut bag);
  let core = lower(&module, &mut bag);

  assert!(!bag.has_errors(), "the fixture must lower: {:?}", bag.into_sorted());

  core
}

fn lowered_expr(expr: &str) -> CModule {
  lowered(&format!(
    "{TYPES}functions\n  t(a, b, c, f, l, p, r, s, x) {{ {expr} }}\n"
  ))
}

fn compiled(expr: &str) -> String {
  let module = compile_matches(lowered_expr(expr));

  dump_expr_shape(&module.decls[0].body, &module.ctors)
}

fn all_exprs(expr: &CExpr) -> Vec<&CExpr> {
  fn walk<'a>(expr: &'a CExpr, out: &mut Vec<&'a CExpr>) {
    out.push(expr);

    match &expr.kind {
      CExprKind::Lit(_)
      | CExprKind::Var(_)
      | CExprKind::Builtin { .. }
      | CExprKind::Std { .. }
      | CExprKind::User { .. }
      | CExprKind::Method { .. }
      | CExprKind::CtorFn { .. }
      | CExprKind::MatchFail
      | CExprKind::Op { .. }
      | CExprKind::Extern { .. } => {}
      CExprKind::Throw { value, .. } | CExprKind::Return { value } => {
        walk(value, out);
      }
      CExprKind::Try { body, handler, .. } => {
        walk(body, out);
        walk(handler, out);
      }
      CExprKind::Lam { body, .. } => walk(body, out),
      CExprKind::App { func, args } => {
        walk(func, out);
        for arg in args {
          walk(arg, out);
        }
      }
      CExprKind::Prim { args, .. }
      | CExprKind::Ctor { args, .. }
      | CExprKind::Dict { args, .. }
      | CExprKind::Concat { parts: args } => {
        for arg in args {
          walk(arg, out);
        }
      }
      CExprKind::Let { value, body, .. } => {
        walk(value, out);
        walk(body, out);
      }
      CExprKind::If { cond, then_branch, else_branch } => {
        walk(cond, out);
        walk(then_branch, out);
        walk(else_branch, out);
      }
      CExprKind::Case { scrutinee, arms } => {
        walk(scrutinee, out);
        for arm in arms {
          walk(&arm.body, out);
        }
      }
      CExprKind::Record { fields } => {
        for (_, value) in fields {
          walk(value, out);
        }
      }
      CExprKind::Update { base, fields } => {
        walk(base, out);
        for (_, value) in fields {
          walk(value, out);
        }
      }
      CExprKind::Field { target, .. } | CExprKind::Test { target, .. } => {
        walk(target, out);
      }
    }
  }

  let mut out = Vec::new();

  walk(expr, &mut out);

  out
}

#[test]
fn wildcard_needs_no_test() {
  assert_eq!(compiled("match x { _ -> 1 }"), "(lit 1)");
}

#[test]
fn variable_binds_the_path() {
  assert_eq!(compiled("match x { y -> y }"), "(let y (var x) (var y))");
}

#[test]
fn scrutinee_var_is_not_rebound() {
  assert_eq!(
    compiled("match x { 1 -> a, _ -> b }"),
    "(if (test (lit 1) (var x)) (var a) (var b))"
  );
}

#[test]
fn scrutinee_expression_is_bound_once() {
  assert_eq!(
    compiled("match f() { 1 -> a, _ -> b }"),
    "(let $s (app (var f)) (if (test (lit 1) (var $s)) (var a) (var b)))"
  );

  let module =
    compile_matches(lowered_expr("match f() { 1 -> a, 2 -> b, _ -> c }"));
  let dump = dump_expr(&module.decls[0].body, &module.ctors);
  let CExprKind::Let { sym: Some(s), .. } = &module.decls[0].body.kind else {
    panic!("expected the scrutinee binding first: {dump}");
  };
  let var = format!("(var $s/{})", s.id);

  assert_eq!(dump.matches("(app ").count(), 1, "{dump}");
  assert_eq!(dump.matches(&var).count(), 2, "{dump}");
}

#[test]
fn literal_uses_identity() {
  let dump = compiled("match x { 1 -> a, _ -> b }");

  assert!(dump.contains("(test (lit 1) (var x))"), "{dump}");
  assert!(!dump.contains("builtin"), "{dump}");
  assert!(!dump.contains("prim Eq"), "{dump}");
}

#[test]
fn constructor_tests_the_tag() {
  assert_eq!(
    compiled("match s { Circle(r) -> r, _ -> 0.0 }"),
    "(if (test (tag Circle) (var s)) \
       (let r (field (var s) _0) (var r)) \
       (lit 0))"
  );
}

#[test]
fn nested_constructor() {
  assert_eq!(
    compiled("match l { Cons(x, Cons(y, _)) -> x + y, _ -> 0 }"),
    "(if (prim And (test (tag Cons) (var l)) (test (tag Cons) (field (var l) _1))) \
       (let x (field (var l) _0) \
         (let y (field (field (var l) _1) _0) \
           (prim Add (var x) (var y)))) \
       (lit 0))"
  );
}

#[test]
fn record_pattern_has_no_test() {
  assert_eq!(
    compiled("match r { { a: x } -> x }"),
    "(let x (field (var r) a) (var x))"
  );
}

#[test]
fn repeated_path_gets_a_temporary() {
  let dump = compiled("match p { W(T(i, j, k)) -> i + j + k }");

  assert_eq!(
    dump,
    "(if (prim And (test (tag W) (var p)) (test (tag T) (field (var p) _0))) \
       (let $t (field (var p) _0) \
         (let i (field (var $t) _0) \
           (let j (field (var $t) _1) \
             (let k (field (var $t) _2) \
               (prim Add (prim Add (var i) (var j)) (var k)))))) \
       (match-fail))"
  );
}

#[test]
fn single_use_path_gets_no_temporary() {
  let dump = compiled("match l { Cons(x, Cons(y, _)) -> x + y, _ -> 0 }");

  assert!(!dump.contains("$t"), "{dump}");
}

#[test]
fn first_match_wins() {
  assert_eq!(
    compiled("match x { 1 -> \"a\", 1 -> \"b\", _ -> \"c\" }"),
    "(if (test (lit 1) (var x)) (lit \"a\") \
       (if (test (lit 1) (var x)) (lit \"b\") (lit \"c\")))"
  );
}

#[test]
fn final_else_is_match_fail() {
  let before = lowered_expr("match s { Circle(r) -> r, Red -> 0.0 }");
  let case_origin = all_exprs(&before.decls[0].body)
    .into_iter()
    .find(|e| matches!(e.kind, CExprKind::Case { .. }))
    .and_then(|e| e.origin.clone())
    .expect("the lowered match has an origin");

  let after = compile_matches(before);
  let dump = dump_expr_shape(&after.decls[0].body, &after.ctors);
  let fails: Vec<&CExpr> = all_exprs(&after.decls[0].body)
    .into_iter()
    .filter(|e| matches!(e.kind, CExprKind::MatchFail))
    .collect();

  assert!(dump.ends_with("(match-fail)))"), "{dump}");
  assert_eq!(fails.len(), 1, "{dump}");
  assert_eq!(fails[0].origin, Some(case_origin));
}

#[test]
fn refutable_let_becomes_a_test() {
  assert_eq!(
    compiled("let Circle(r) = s\n r"),
    "(if (test (tag Circle) (var s)) \
       (let r (field (var s) _0) (var r)) \
       (match-fail))"
  );
}

#[test]
fn record_field_can_hold_a_refutable_pattern() {
  assert_eq!(
    compiled("match r { { a: 1, b: y } -> y, _ -> 0 }"),
    "(if (test (lit 1) (field (var r) a)) (let y (field (var r) b) (var y)) (lit 0))"
  );
}

#[test]
fn nested_matches_are_compiled() {
  let module = compile_matches(lowered_expr(
    "match f(match x { 1 -> a, _ -> b }) { Red -> match s { Circle(r) -> r, _ -> 0.0 }, _ -> c }",
  ));
  let exprs = all_exprs(&module.decls[0].body);

  assert!(
    !exprs.iter().any(|e| matches!(e.kind, CExprKind::Case { .. })),
    "{}",
    dump_expr_shape(&module.decls[0].body, &module.ctors)
  );
}

#[test]
fn fresh_symbols_do_not_collide() {
  let before = lowered_expr("match f() { W(T(i, j, _)) -> i + j, _ -> 0 }");
  let before_max = all_exprs(&before.decls[0].body)
    .into_iter()
    .filter_map(|e| match &e.kind {
      CExprKind::Var(s) => Some(s.id),
      _ => None,
    })
    .chain(before.decls[0].params.iter().map(|p| p.id))
    .max()
    .unwrap();

  let after = compile_matches(before);
  let fresh: Vec<u32> = all_exprs(&after.decls[0].body)
    .into_iter()
    .filter_map(|e| match &e.kind {
      CExprKind::Let { sym: Some(s), .. } if s.name.starts_with('$') => {
        Some(s.id)
      }
      _ => None,
    })
    .collect();

  assert_eq!(fresh.len(), 2, "one `$s` and one `$t`");
  assert!(fresh.iter().all(|id| *id > before_max), "{fresh:?} vs {before_max}");
  assert_ne!(fresh[0], fresh[1]);
}
