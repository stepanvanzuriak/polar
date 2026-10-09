use polar_compiler::{
  core::{
    dump::{dump_expr, dump_expr_shape, dump_module},
    ir::{CExpr, CExprKind, CModule},
    lower,
  },
  shared::codes::DiagnosticCode::{
    self, BuiltinModuleAsValue, CallArity, ConstructorArity,
    DuplicateDefinition, NumberOutOfRange, UnknownBuiltinMember, UnknownModule,
    UnknownName, UnknownUppercaseName,
  },
  shared::diagnostic::{Diagnostic, DiagnosticBag},
  shared::source::SourceFile,
  syntax::lexer::lex,
  syntax::parser::parse,
};

fn lower_src(src: &str) -> (CModule, Vec<Diagnostic>) {
  let file = SourceFile::new("test.px", src);
  let mut bag = DiagnosticBag::default();
  let lexed = lex(&file, &mut bag);
  let module = parse(&file, &lexed, &mut bag);

  assert!(!bag.has_errors(), "the fixture must parse: {:?}", bag.into_sorted());

  let core = lower(&module, &mut bag);

  (core, bag.into_sorted())
}

const PARAMS: &str = "a, b, c, f, g, p, r, s, x, y, body, user";

fn expr_module(expr: &str) -> CModule {
  let (module, diagnostics) =
    lower_src(&format!("functions\n  t({PARAMS}) {{ {expr} }}\n"));

  assert!(diagnostics.is_empty(), "unexpected diagnostics: {diagnostics:?}");

  module
}

fn core(expr: &str) -> String {
  let module = expr_module(expr);

  dump_expr_shape(&module.decls[0].body, &module.ctors)
}

fn codes(diagnostics: &[Diagnostic]) -> Vec<DiagnosticCode> {
  diagnostics.iter().map(|d| d.code).collect()
}

fn codes_of(src: &str) -> Vec<DiagnosticCode> {
  codes(&lower_src(src).1)
}

fn all_exprs(module: &CModule) -> Vec<&CExpr> {
  fn walk<'a>(expr: &'a CExpr, out: &mut Vec<&'a CExpr>) {
    out.push(expr);

    let mut each = |e: &'a CExpr| walk(e, out);

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
        each(value);
      }
      CExprKind::Try { body, handler, .. } => {
        each(body);
        each(handler);
      }
      CExprKind::Lam { body, .. } => each(body),
      CExprKind::App { func, args } => {
        each(func);
        args.iter().for_each(each);
      }
      CExprKind::Prim { args, .. }
      | CExprKind::Ctor { args, .. }
      | CExprKind::Dict { args, .. }
      | CExprKind::Concat { parts: args } => args.iter().for_each(each),
      CExprKind::Let { value, body, .. } => {
        each(value);
        each(body);
      }
      CExprKind::If { cond, then_branch, else_branch } => {
        each(cond);
        each(then_branch);
        each(else_branch);
      }
      CExprKind::Case { scrutinee, arms } => {
        each(scrutinee);
        for arm in arms {
          each(&arm.body);
        }
      }
      CExprKind::Record { fields } => fields.iter().for_each(|(_, v)| each(v)),
      CExprKind::Update { base, fields } => {
        each(base);
        for (_, value) in fields {
          each(value);
        }
      }
      CExprKind::Field { target, .. } | CExprKind::Test { target, .. } => {
        each(target);
      }
    }
  }

  let mut out = Vec::new();

  for decl in &module.decls {
    walk(&decl.body, &mut out);
  }

  out
}

const SHAPES: &str = "\
types
  Shape = Circle(Float) | Square(Float, Float) | Red
";

mod desugaring {
  use super::*;

  #[test]
  fn int_literal() {
    assert_eq!(core("1_000"), "(lit 1000)");
  }

  #[test]
  fn int_out_of_range() {
    assert_eq!(
      codes_of("functions\n  t() { 9007199254740993 }\n"),
      [NumberOutOfRange]
    );
    assert!(codes_of("functions\n  t() { 9007199254740991 }\n").is_empty());
  }

  #[test]
  fn float_out_of_range() {
    assert_eq!(codes_of("functions\n  t() { 1e999 }\n"), [NumberOutOfRange]);
  }

  #[test]
  fn plain_string() {
    assert_eq!(core(r#""a""#), r#"(lit "a")"#);
    assert_eq!(core(r#""""#), r#"(lit "")"#);
  }

  #[test]
  fn interpolated_string() {
    assert_eq!(
      core(r#""id #{p.id}""#),
      r#"(concat (lit "id ") (field (var p) id))"#
    );
  }

  #[test]
  fn pipe_bare() {
    assert_eq!(core("a |> f"), "(app (var f) (var a))");
  }

  #[test]
  fn pipe_with_args() {
    assert_eq!(core("a |> f(b)"), "(app (var f) (var a) (var b))");
  }

  #[test]
  fn pipe_chain() {
    assert_eq!(
      core("a |> f(b) |> g"),
      "(app (var g) (app (var f) (var a) (var b)))"
    );
  }

  #[test]
  fn goal_slug() {
    assert_eq!(
      core(r#"r.title |> String.lowercase |> String.replace(" ", "-")"#),
      "(app (builtin String replace) \
         (app (builtin String lowercase) (field (var r) title)) \
         (lit \" \") (lit \"-\"))"
    );
  }

  #[test]
  fn if_expression() {
    assert_eq!(core("if c { a } else { b }"), "(if (var c) (var a) (var b))");
  }

  #[test]
  fn else_if_nests() {
    assert_eq!(
      core("if a { 1 } else if b { 2 } else { 3 }"),
      "(if (var a) (lit 1) (if (var b) (lit 2) (lit 3)))"
    );
  }

  #[test]
  fn block_becomes_nested_lets() {
    let module = expr_module("{ let x = 1\n f(x)\n x }");

    assert_eq!(
      dump_expr(&module.decls[0].body, &module.ctors),
      "(let x/13 (lit 1) (let _ (app (var f/4) (var x/13)) (var x/13)))"
    );
  }

  #[test]
  fn let_wildcard_discards() {
    assert_eq!(
      core("{ let _ = f(a)\n b }"),
      "(let _ (app (var f) (var a)) (var b))"
    );
  }

  #[test]
  fn refutable_let_becomes_case() {
    assert_eq!(
      core("{ let { a: z } = r\n z }"),
      "(case (var r) (arm (record (a z)) (var z)))"
    );
  }

  #[test]
  fn record_literal() {
    assert_eq!(core("{ a: 1, b: 2 }"), "(record (a (lit 1)) (b (lit 2)))");
    assert_eq!(core("{}"), "(record)");
  }

  #[test]
  fn record_update() {
    assert_eq!(
      core("{ ..body, author_id: user.id }"),
      "(update (var body) (author_id (field (var user) id)))"
    );
  }

  #[test]
  fn primops() {
    assert_eq!(core("a && b"), "(prim And (var a) (var b))");
    assert_eq!(core("!a"), "(prim Not (var a))");
    assert_eq!(core("-a"), "(prim Neg (var a))");
    assert_eq!(core("a + b"), "(prim Add (var a) (var b))");
    assert_eq!(core("a == b"), "(prim Eq (var a) (var b))");
    assert_eq!(core("a != b"), "(prim Ne (var a) (var b))");
    assert_eq!(core("a % b"), "(prim Mod (var a) (var b))");
  }

  #[test]
  fn lambda() {
    assert_eq!(
      core("function(z) { z + 1 }"),
      "(lam (z) (prim Add (var z) (lit 1)))"
    );
  }

  #[test]
  fn shadowing() {
    let module = expr_module("{ let x = 1\n let x = x + 1\n x }");

    assert_eq!(
      dump_expr(&module.decls[0].body, &module.ctors),
      "(let x/13 (lit 1) (let x/14 (prim Add (var x/13) (lit 1)) (var x/14)))"
    );
  }

  #[test]
  fn pipe_origin_is_the_stage() {
    let src = format!("functions\n  t({PARAMS}) {{ a |> f }}\n");
    let (module, _) = lower_src(&src);
    let origin = module.decls[0].body.origin.clone().unwrap();

    assert_eq!(&src[origin.start..origin.end], "f");
  }

  #[test]
  fn constructors() {
    let (module, diagnostics) = lower_src(&format!(
      "{SHAPES}functions\n  t(f) {{ {{ let a = Circle(1.0)\n let b = Red\n f(Square) }} }}\n"
    ));

    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert_eq!(
      dump_expr_shape(&module.decls[0].body, &module.ctors),
      "(let a (ctor Circle (lit 1)) (let b (ctor Red) (app (var f) (ctorfn Square))))"
    );
  }

  #[test]
  fn patterns() {
    let (module, diagnostics) = lower_src(&format!(
      "{SHAPES}functions\n  t(s) {{ match s {{ Circle(r) -> r, Square(_, w) -> w, -1 -> 0.0, \"a\" -> 1.0, {{ k: true }} -> 2.0, _ -> 3.0 }} }}\n"
    ));

    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert_eq!(
      dump_expr_shape(&module.decls[0].body, &module.ctors),
      "(case (var s) \
         (arm (Circle r) (var r)) \
         (arm (Square _ w) (var w)) \
         (arm -1 (lit 0)) \
         (arm \"a\" (lit 1)) \
         (arm (record (k true)) (lit 2)) \
         (arm _ (lit 3)))"
    );
  }

  #[test]
  fn builtin_member_call() {
    assert_eq!(
      core(r#"Log.info("hi")"#),
      r#"(app (builtin Log info) (lit "hi"))"#
    );
  }

  #[test]
  fn exports_are_marked() {
    let (module, diagnostics) =
      lower_src("functions\n  a() { 1 }\n  b() { 2 }\nexports\n  b\n");

    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert!(!module.decls[0].exported);
    assert!(module.decls[1].exported);
  }
}

mod diagnostics {
  use super::*;

  fn only(src: &str, code: DiagnosticCode) -> Diagnostic {
    let (_, diagnostics) = lower_src(src);

    assert_eq!(codes(&diagnostics), [code], "{diagnostics:?}");

    diagnostics.into_iter().next().unwrap()
  }

  #[test]
  fn unknown_lowercase() {
    let d = only("functions\n  f(post) { pots }\n", UnknownName);

    assert_eq!(d.message, "cannot find `pots` in this scope");
    assert_eq!(d.help.as_deref(), Some("did you mean `post`?"));
  }

  #[test]
  fn unknown_lowercase_no_suggestion() {
    let d = only("functions\n  f(post) { zzzzzz }\n", UnknownName);

    assert_eq!(d.help, None);
  }

  #[test]
  fn unknown_uppercase() {
    only("functions\n  f() { Nope }\n", UnknownUppercaseName);
  }

  #[test]
  fn builtin_module_as_a_value() {
    only("functions\n  f() { Log }\n", BuiltinModuleAsValue);
  }

  #[test]
  fn ctor_as_a_value() {
    let (_, diagnostics) = lower_src(&format!(
      "{SHAPES}functions\n  map(xs, f) {{ xs }}\n  t(xs) {{ map(xs, Circle) }}\n"
    ));

    assert!(diagnostics.is_empty(), "{diagnostics:?}");
  }

  #[test]
  fn unknown_builtin_member() {
    let d = only("functions\n  f(s) { Log.nope(s) }\n", UnknownBuiltinMember);

    assert_eq!(d.message, "`Log` has no member `nope`");
    assert_eq!(d.help.as_deref(), Some("available members: `info`, `error`"));
  }

  #[test]
  fn duplicate_fn() {
    let d = only("functions\n  f() { 1 }\n  f() { 2 }\n", DuplicateDefinition);

    assert_eq!(d.secondary.len(), 1, "a secondary label on the first");
    assert_eq!(d.secondary[0].span.start, "functions\n  ".len());
  }

  #[test]
  fn duplicate_parameter() {
    only("functions\n  f(a, a) { a }\n", DuplicateDefinition);
  }

  #[test]
  fn duplicate_binding_in_a_pattern() {
    let src = "types\n  P = P(Int, Int)\nfunctions\n  f(p) { match p { P(x, x) -> x } }\n";

    only(src, DuplicateDefinition);
  }

  #[test]
  fn type_named_like_a_builtin_module() {
    only("types\n  String = Int\n", DuplicateDefinition);
  }

  #[test]
  fn ctor_arity() {
    let src = format!(
      "{SHAPES}functions\n  t() {{ {{ let a = Circle()\n Red(1) }} }}\n"
    );

    assert_eq!(codes_of(&src), [ConstructorArity, ConstructorArity]);
  }

  #[test]
  fn ctor_pattern_arity() {
    let src = format!(
      "{SHAPES}functions\n  t(s) {{ match s {{ Circle(a, b) -> a }} }}\n"
    );

    assert_eq!(codes_of(&src), [ConstructorArity]);
  }

  #[test]
  fn fn_arity() {
    let d = only("functions\n  f(a) { a }\n  g() { f(1, 2) }\n", CallArity);

    assert_eq!(d.message, "`f` takes 1 argument but 2 were given");
  }

  #[test]
  fn builtin_arity() {
    only("functions\n  f() { Log.info(1, 2) }\n", CallArity);
  }

  #[test]
  fn local_shadows_a_top_level_fn_for_arity() {
    let (_, diagnostics) =
      lower_src("functions\n  f(a) { a }\n  g(f) { f(1, 2) }\n");

    assert!(diagnostics.is_empty(), "{diagnostics:?}");
  }

  #[test]
  fn unknown_user_module() {
    only("uses\n  Foo\n", UnknownModule);
  }

  #[test]
  fn unknown_export() {
    let d = only("functions\n  main() { 1 }\nexports\n  mian\n", UnknownName);

    assert_eq!(d.help.as_deref(), Some("did you mean `main`?"));
  }
}

mod ir_properties {
  use super::*;

  const PROGRAM: &str = "\
types
  List = Nil | Cons(Int, List)
functions
  a(xs) { b(xs) }
  b(xs) {
    let n = 1
    match xs {
      Nil -> n,
      Cons(h, t) -> h + a(t),
    }
  }
exports
  a
";

  #[test]
  fn top_level_fns_are_mutually_visible() {
    let (_, diagnostics) = lower_src(PROGRAM);

    assert!(diagnostics.is_empty(), "{diagnostics:?}");
  }

  #[test]
  fn ids_are_per_compilation() {
    assert_eq!(
      dump_module(&lower_src(PROGRAM).0),
      dump_module(&lower_src(PROGRAM).0)
    );
  }

  #[test]
  fn slots_are_unset() {
    let (module, _) = lower_src(PROGRAM);

    assert!(
      all_exprs(&module).iter().all(|e| e.ty.is_none() && e.effects.is_none())
    );
  }

  #[test]
  fn origin_is_on_every_node() {
    let (module, _) = lower_src(PROGRAM);

    assert!(all_exprs(&module).iter().all(|e| e.origin.is_some()));
  }

  #[test]
  fn shadowing_gets_distinct_ids() {
    let module = expr_module("{ let x = 1\n let x = x + 1\n x }");
    let dump = dump_expr(&module.decls[0].body, &module.ctors);

    assert!(dump.contains("x/13") && dump.contains("x/14"), "{dump}");
  }
}

mod examples {
  use super::*;

  fn example(name: &str) -> String {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/programs/");

    std::fs::read_to_string(format!("{path}{name}")).expect("read the example")
  }

  #[test]
  fn hello_lowers_cleanly() {
    let (module, diagnostics) = lower_src(&example("hello.px"));

    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert!(module.decls[0].exported);
  }

  #[test]
  fn blog_lowers_cleanly() {
    let (_, diagnostics) = lower_src(&example("blog.px"));

    assert!(diagnostics.is_empty(), "{diagnostics:?}");
  }
}
