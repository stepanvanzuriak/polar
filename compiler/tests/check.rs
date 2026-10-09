use polar_compiler::{
  Stage,
  backend::codegen::builtins::BUILTINS,
  check::{self, decl::groups, report::report},
  core::{
    annotate::annotate,
    ir::{CExpr, CExprKind, PrimOp, TypeSlot},
    lower::{Lowered, Resolved, lower_with},
  },
  dump_stage,
  shared::codes::DiagnosticCode,
  shared::diagnostic::{Diagnostic, DiagnosticBag},
  shared::source::{SourceFile, Span},
  syntax::ast::Module,
  syntax::lexer::lex,
  syntax::parser::parse,
  types::{
    print::Printer,
    store::Store,
    ty::{Constraint, Type},
    unify::UnifyError,
  },
};
use std::sync::Arc;

const PRELUDE: &str = "types
  List<a> = Nil | Cons(a, List<a>)
  Option<a> = None | Some(a)
  Result<e, a> = Err(e) | Ok(a)
";

fn module(functions: &str) -> String {
  let indented: String =
    functions.lines().flat_map(|l| ["  ", l, "\n"]).collect();

  format!("{PRELUDE}\nfunctions\n{indented}")
}

fn types_of(src: &str) -> String {
  let result = dump_stage(src, "test.px", Stage::Types);

  assert!(
    result.diagnostics.is_empty(),
    "unexpected errors: {:#?}",
    result.diagnostics
  );

  result.output.unwrap()
}

fn type_of(src: &str, name: &str) -> String {
  let prefix = format!("{name} : ");

  types_of(src)
    .lines()
    .find_map(|l| l.strip_prefix(&prefix))
    .expect("no such declaration")
    .to_string()
}

fn diagnostics_of(src: &str) -> Vec<Diagnostic> {
  dump_stage(src, "test.px", Stage::Types).diagnostics
}

fn errors_of(src: &str) -> Vec<String> {
  let file = SourceFile::new("test.px", src);

  diagnostics_of(src)
    .iter()
    .map(|d| format!("{} at {:?}", d.code, file.slice(&d.primary.span)))
    .collect()
}

fn codes_of(src: &str) -> Vec<String> {
  diagnostics_of(src).iter().map(|d| d.code.to_string()).collect()
}

fn one_error(src: &str, expected: &str) {
  assert_eq!(errors_of(src), vec![expected.to_string()]);
}

fn example(name: &str) -> String {
  std::fs::read_to_string(format!(
    "{}/tests/fixtures/programs/{name}",
    env!("CARGO_MANIFEST_DIR")
  ))
  .unwrap()
}

fn std_source(name: &str) -> String {
  std::fs::read_to_string(format!(
    "{}/../std/{name}",
    env!("CARGO_MANIFEST_DIR")
  ))
  .unwrap()
}

fn function_from(src: &str, name: &str) -> String {
  let start = src
    .lines()
    .position(|l| l.trim_start().starts_with(&format!("{name}(")))
    .expect("no such function");
  let lines: Vec<&str> = src.lines().collect();
  let indent = lines[start].len() - lines[start].trim_start().len();
  let mut out = Vec::new();

  for line in &lines[start..] {
    out.push(&line[indent.min(line.len())..]);

    if line.trim_end() == format!("{}}}", " ".repeat(indent)) {
      break;
    }
  }

  out.join("\n")
}

fn parsed(src: &str) -> (SourceFile, Module) {
  let file = SourceFile::new("test.px", src);
  let mut bag = DiagnosticBag::default();
  let module = parse(&file, &lex(&file, &mut bag), &mut bag);

  assert!(!bag.has_errors(), "the fixture must parse");
  (file, module)
}

fn lowered(src: &str) -> (Module, Lowered) {
  let (_, module) = parsed(src);
  let mut bag = DiagnosticBag::default();
  let lowered = lower_with(&module, &[], &mut bag);

  assert!(!bag.has_errors(), "{:?}", bag.into_sorted());
  (module, lowered)
}

fn span_of(src: &str, needle: &str, nth: usize) -> Span {
  let start = src
    .match_indices(needle)
    .nth(nth)
    .map(|(i, _)| i)
    .expect("needle not found");

  Span::new(Arc::from("test.px"), start, start + needle.len())
}

mod wiring {
  use super::*;

  #[test]
  fn resolves_parameter() {
    let src = "functions\n  f(x) { x }\n";
    let (_, lowered) = lowered(src);
    let param = lowered.resolutions.get(&span_of(src, "x", 0)).cloned();
    let body = lowered.resolutions.get(&span_of(src, "x", 1)).cloned();

    assert!(matches!(param, Some(Resolved::Local(_))), "{param:?}");
    assert_eq!(param, body);
  }

  #[test]
  fn resolves_ctor_and_builtin() {
    let src = format!(
      "{PRELUDE}\nfunctions\n  f() {{ Some(1) }}\n  g() {{ Log.info(\"a\") }}\n"
    );
    let (_, lowered) = lowered(&src);
    let ctor = lowered.resolutions.get(&span_of(&src, "Some", 1)).cloned();
    let builtin = lowered.resolutions.get(&span_of(&src, "info", 0)).cloned();

    assert!(matches!(ctor, Some(Resolved::Ctor(_))), "{ctor:?}");
    assert_eq!(
      builtin,
      Some(Resolved::Builtin { module: "Log", member: "info" })
    );
  }

  #[test]
  fn codes_registered() {
    for n in 501..=513 {
      assert!(
        DiagnosticCode::ALL.iter().any(|c| *c as u16 == n),
        "POLAR{n:04} is not registered"
      );
    }
  }

  #[test]
  fn literal_types() {
    let src = module("f() { 1 }\ng() { 1.5 }\nh() { true }\nk() { \"a\" }");

    assert_eq!(type_of(&src, "f"), "function() -> Int");
    assert_eq!(type_of(&src, "g"), "function() -> Float");
    assert_eq!(type_of(&src, "h"), "function() -> Bool");
    assert_eq!(type_of(&src, "k"), "function() -> String");
  }

  #[test]
  fn types_in_source_order() {
    let out = types_of(&module("b() { 1 }\na() { 2 }"));
    let b = out.find("b : ").unwrap();
    let a = out.find("a : ").unwrap();

    assert!(b < a, "{out}");
  }

  #[test]
  fn no_check_after_lowering_error() {
    assert_eq!(codes_of(&module("f() { nope }")), vec!["POLAR0301"]);
  }

  #[test]
  fn report_codes() {
    let store = Store::default();
    let span = Span::new(Arc::from("test.px"), 0, 1);
    let record = Type::unit();
    let cases = [
      (
        UnifyError::Mismatch { expected: Type::int(), found: Type::string() },
        "POLAR0501",
      ),
      (UnifyError::ParamCount { expected: 1, found: 2 }, "POLAR0508"),
      (
        UnifyError::Occurs {
          var: polar_compiler::types::ty::TVar(0),
          ty: Type::int(),
        },
        "POLAR0504",
      ),
      (
        UnifyError::MissingField {
          label: Arc::from("a"),
          record: record.clone(),
        },
        "POLAR0502",
      ),
      (UnifyError::ExtraField { label: Arc::from("a"), record }, "POLAR0503"),
      (
        UnifyError::NotNumeric {
          constraint: Constraint::Num,
          found: Type::string(),
        },
        "POLAR0512",
      ),
    ];
    let mut store = store;
    let _ = store.fresh();

    for (error, code) in cases {
      assert_eq!(report(&error, span.clone(), &store).code.to_string(), code);
    }
  }

  fn find_prim(expr: &CExpr) -> Option<&CExpr> {
    if let CExprKind::Prim { .. } = expr.kind {
      return Some(expr);
    }

    match &expr.kind {
      CExprKind::Let { value, body, .. } => {
        find_prim(value).or_else(|| find_prim(body))
      }
      _ => None,
    }
  }

  #[test]
  fn prim_args_typed() {
    let src = "functions\n  f(a: Int, b: Int) { a == b }\n";
    let (module, lowered) = lowered(src);
    let mut bag = DiagnosticBag::default();
    let types = check::check(&module, &lowered, &mut bag);

    assert!(!bag.has_errors());

    let core = annotate(lowered.core, &types);
    let prim = find_prim(&core.decls[0].body).expect("a prim");
    let CExprKind::Prim { op: PrimOp::Eq, args } = &prim.kind else {
      panic!("not an equality")
    };

    for arg in args {
      assert_eq!(arg.ty, Some(TypeSlot(Type::int())));
    }
  }
}

mod builtins {
  use super::*;
  use polar_compiler::{
    check::env::{parse_signature, signature_expr},
    syntax::ast::TypeExpr,
  };

  #[test]
  fn every_signature_parses() {
    for info in BUILTINS {
      assert!(
        matches!(signature_expr(info), TypeExpr::Fn(_)),
        "{}.{}",
        info.module,
        info.member
      );
    }
  }

  #[test]
  fn signature_matches_arity() {
    for info in BUILTINS {
      let TypeExpr::Fn(f) = signature_expr(info) else {
        panic!("not a function")
      };

      assert_eq!(f.params.len(), info.arity, "{}.{}", info.module, info.member);
    }
  }

  #[test]
  fn signature_only_primitives() {
    fn primitive(ty: &Type) -> bool {
      match ty {
        Type::Con { name, args } if args.is_empty() => {
          ["Int", "Float", "String", "Bool"].contains(&&**name)
        }
        Type::Con { name, args } => {
          ["Std.Option.Option", "Std.List.List"].contains(&&**name)
            && args.iter().all(primitive)
        }
        Type::Record(row) => {
          row.tail.is_closed() && row.fields.iter().all(|(_, t)| primitive(t))
        }
        Type::Fn { params, ret, .. } => {
          params.iter().all(primitive) && primitive(ret)
        }
        _ => false,
      }
    }

    for info in BUILTINS {
      if polar_compiler::backend::codegen::builtins::std_owner(info.module)
        .is_none()
      {
        assert!(primitive(&parse_signature(info)), "{}", info.signature);
      }
    }
  }

  #[test]
  fn builtin_call() {
    let src = module("f(s: String) { String.length(s) }");

    assert_eq!(type_of(&src, "f"), "function(String) -> Int");
  }

  #[test]
  fn builtin_bad_argument() {
    one_error(&module("f() { String.length(1) }"), "POLAR0501 at \"1\"");
  }

  #[test]
  fn log_returns_unit() {
    assert_eq!(
      type_of(&module("f() { Log.info(\"a\") }"), "f"),
      "function() -> {}"
    );
  }

  #[test]
  fn builtin_as_value() {
    assert_eq!(
      type_of(&module("f() { String.length }"), "f"),
      "function() -> function(String) -> Int"
    );
  }
}

mod expressions {
  use super::*;

  #[test]
  fn interpolation_any_type() {
    let src = module("f(x: Float) { \"#{x} #{x == x}\" }");

    assert_eq!(type_of(&src, "f"), "function(Float) -> String");
  }

  #[test]
  fn identity() {
    assert_eq!(type_of(&module("id(x) { x }"), "id"), "function(a) -> a");
  }

  #[test]
  fn let_polymorphism() {
    let src = module(
      "f() {\n  let id = function(x) { x }\n  let a = id(1)\n  id(\"s\")\n}",
    );

    assert_eq!(type_of(&src, "f"), "function() -> String");
  }

  #[test]
  fn lambda_param_not_generalised() {
    one_error(
      &module("f(g) {\n  let a = g(1)\n  g(\"s\")\n}"),
      "POLAR0501 at \"\\\"s\\\"\"",
    );
  }

  #[test]
  fn ctor_values() {
    let src = module("a() { None }\nb() { Some }");

    assert_eq!(type_of(&src, "a"), "function() -> Option<a>");
    assert_eq!(type_of(&src, "b"), "function() -> function(a) -> Option<a>");
  }

  #[test]
  fn call_argument_blame() {
    one_error(
      &module("f(x: Int) { x }\ng() { f(\"a\") }"),
      "POLAR0501 at \"\\\"a\\\"\"",
    );
  }

  #[test]
  fn call_value_arity() {
    one_error(
      &module("f(g: function(Int) -> Int) { g(1, 2) }"),
      "POLAR0508 at \"g(1, 2)\"",
    );
  }

  #[test]
  fn not_a_function() {
    one_error(&module("f(x: Int) { x(1) }"), "POLAR0507 at \"x\"");
  }

  #[test]
  fn call_infers_function() {
    assert_eq!(
      type_of(&module("apply(f, x) { f(x) }"), "apply"),
      "function(function(a) -> b / {| e}, a) -> b / {| e}"
    );
  }

  #[test]
  fn pipe() {
    let src = module("f(s: String) { s |> String.replace(\"a\", \"b\") }");

    assert_eq!(type_of(&src, "f"), "function(String) -> String");
  }

  #[test]
  fn pipe_blame() {
    one_error(&module("f() { 1 |> String.lowercase }"), "POLAR0501 at \"1\"");
  }

  #[test]
  fn lambda_annotated() {
    assert_eq!(
      type_of(&module("f() { function(x: Int) -> Int { x } }"), "f"),
      "function() -> function(Int) -> Int"
    );
  }

  #[test]
  fn lambda_return_blame() {
    one_error(
      &module("f() { function(x: Int) -> String { x } }"),
      "POLAR0501 at \"x\"",
    );
  }

  #[test]
  fn statement_discarded() {
    assert_eq!(
      type_of(&module("f() {\n  1\n  \"a\"\n}"), "f"),
      "function() -> String"
    );
  }

  #[test]
  fn let_annotation() {
    one_error(
      &module("f() {\n  let x: Int = \"a\"\n  x\n}"),
      "POLAR0501 at \"\\\"a\\\"\"",
    );
  }

  #[test]
  fn if_condition() {
    one_error(
      &module("f(x: Int) { if x { 1 } else { 2 } }"),
      "POLAR0501 at \"x\"",
    );
  }

  #[test]
  fn if_branch_blame() {
    one_error(
      &module("f(c: Bool) { if c { 1 } else { \"a\" } }"),
      "POLAR0501 at \"\\\"a\\\"\"",
    );
  }

  #[test]
  fn else_if() {
    one_error(
      &module(
        "f(a: Bool, b: Bool) { if a { 1 } else if b { 2 } else { \"c\" } }",
      ),
      "POLAR0501 at \"\\\"c\\\"\"",
    );
  }

  #[test]
  fn arith_int() {
    assert_eq!(
      type_of(&module("f(a: Int, b: Int) { a + b * 2 }"), "f"),
      "function(Int, Int) -> Int"
    );
  }

  #[test]
  fn arith_float() {
    assert_eq!(
      type_of(&module("f(a: Float) { -a / 2.0 }"), "f"),
      "function(Float) -> Float"
    );
  }

  #[test]
  fn no_int_float_mixing() {
    one_error(&module("f(a: Float) { a * 2 }"), "POLAR0501 at \"2\"");
  }

  #[test]
  fn arith_strings_rejected() {
    one_error(&module("f() { \"a\" + \"b\" }"), "POLAR0512 at \"\\\"a\\\"\"");
  }

  #[test]
  fn compare_strings() {
    assert_eq!(
      type_of(&module("f(a: String, b: String) { a < b }"), "f"),
      "function(String, String) -> Bool"
    );
  }

  #[test]
  fn compare_bools_rejected() {
    one_error(&module("f() { true < false }"), "POLAR0512 at \"true\"");
  }

  #[test]
  fn equality_needs_eq() {
    assert_eq!(
      type_of(&module("f(a, b) { a == b }"), "f"),
      "function(a, a) -> Bool where Eq<a>"
    );
  }

  #[test]
  fn equality_same_type() {
    one_error(&module("f() { 1 == \"a\" }"), "POLAR0501 at \"\\\"a\\\"\"");
  }

  #[test]
  fn list_literal() {
    assert_eq!(
      type_of(&module("f() { [1, 2] }"), "f"),
      "function() -> List<Int>"
    );
  }

  #[test]
  fn list_mixed() {
    one_error(&module("f() { [1, \"a\"] }"), "POLAR0501 at \"\\\"a\\\"\"");
  }

  #[test]
  fn infinite_type() {
    assert_eq!(codes_of(&module("f(x) { f([x]) }")), vec!["POLAR0504"]);
  }

  #[test]
  fn one_error_per_mistake() {
    let src = module("f() {\n  let x = 1 + \"a\"\n  let y = x + 1\n  x + 2\n}");

    assert_eq!(errors_of(&src).len(), 1, "{:?}", errors_of(&src));
  }
}

mod patterns {
  use super::*;

  #[test]
  fn wildcard() {
    assert_eq!(
      type_of(&module("f(x) { match x { _ -> 1 } }"), "f"),
      "function(a) -> Int"
    );
  }

  #[test]
  fn variable() {
    assert_eq!(
      type_of(&module("f(x: Int) { match x { n -> n + 1 } }"), "f"),
      "function(Int) -> Int"
    );
  }

  #[test]
  fn literal() {
    let src = module("f(x) { match x { 0 -> \"zero\", _ -> \"other\" } }");

    assert_eq!(type_of(&src, "f"), "function(Int) -> String");
  }

  #[test]
  fn literal_wrong_type() {
    one_error(
      &module("f(x: String) { match x { 0 -> 1, _ -> 2 } }"),
      "POLAR0501 at \"0\"",
    );
  }

  #[test]
  fn negative_float() {
    assert_eq!(
      type_of(&module("f(x) { match x { -1.5 -> 1, _ -> 0 } }"), "f"),
      "function(Float) -> Int"
    );
  }

  #[test]
  fn constructor() {
    let src = module("f(o) { match o { Some(x) -> x, None -> 0 } }");

    assert_eq!(type_of(&src, "f"), "function(Option<Int>) -> Int");
  }

  #[test]
  fn constructor_wrong_type() {
    one_error(
      &module("f(o: Option<Int>) { match o { Ok(x) -> x, _ -> 0 } }"),
      "POLAR0501 at \"Ok(x)\"",
    );
  }

  #[test]
  fn nested() {
    let src = module("f(o) { match o { Some(Some(x)) -> x, _ -> 0 } }");

    assert_eq!(type_of(&src, "f"), "function(Option<Option<Int>>) -> Int");
  }

  #[test]
  fn list_patterns() {
    let src = module(&function_from(&example("blog.px"), "length"));

    assert_eq!(type_of(&src, "length"), "function(List<a>) -> Int");
  }

  #[test]
  fn arm_blame() {
    one_error(
      &module("f(o: Option<Int>) { match o { Some(_) -> 1, None -> \"a\" } }"),
      "POLAR0501 at \"\\\"a\\\"\"",
    );
  }

  #[test]
  fn pattern_var_not_generalised() {
    let src = module(
      "f() {\n  let id = function(x) { x }\n  match id {\n    g -> {\n      let a = g(1)\n      g(\"a\")\n    },\n  }\n}",
    );

    one_error(&src, "POLAR0501 at \"\\\"a\\\"\"");
  }

  #[test]
  fn refutable_let() {
    let src = module("f(o: Option<Int>) {\n  let Some(x) = o\n  x\n}");
    let (module, lowered) = lowered(&src);
    let mut bag = DiagnosticBag::default();
    let types = check::check(&module, &lowered, &mut bag);
    let scheme = types.scheme("f").expect("f has a type");
    let codes: Vec<String> =
      bag.into_sorted().iter().map(|d| d.code.to_string()).collect();

    assert_eq!(
      Printer::new(&[&scheme.ty], false).print(&scheme.ty),
      "function(Option<Int>) -> Int"
    );
    assert_eq!(codes, vec!["POLAR0602"]);
  }
}

const BLOG_TYPES: &str = "types
  Id<a> = Int
  Status = Draft | Published(Int)
  List<a> = Nil | Cons(a, List<a>)
  User = { id: Id<User>, name: String }
  Post = {
    id: Id<Post>,
    title: String,
    body: String,
    author_id: Id<User>,
    status: Status,
  }
";

fn blog_module(functions: &str) -> String {
  let indented: String =
    functions.lines().flat_map(|l| ["  ", l, "\n"]).collect();

  format!("{BLOG_TYPES}\nfunctions\n{indented}")
}

const POST: &str = "{ author_id: Int, body: String, id: Int, status: Status, \
                    title: String } as Post";

mod records {
  use super::*;

  #[test]
  fn literal_is_closed() {
    assert_eq!(
      type_of(&module("f() { { id: 1, name: \"Ada\" } }"), "f"),
      "function() -> { id: Int, name: String }"
    );
  }

  #[test]
  fn duplicate_field() {
    let src = module("f() { { a: 1, a: 2 } }");
    let errors = diagnostics_of(&src);

    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].code.to_string(), "POLAR0513");
    let second = span_of(&src, "a: 2", 0);

    assert_eq!(errors[0].primary.span.start, second.start);
    assert_eq!(errors[0].primary.span.end, second.start + 1);
  }

  #[test]
  fn access_open() {
    assert_eq!(
      type_of(&module("get(r) { r.title }"), "get"),
      "function({ title: a | r }) -> a"
    );
  }

  const TITLE_OF: &str =
    "title_of(r: { title: String | rest }) -> String {\n  r.title\n}";

  #[test]
  fn title_of_two_shapes() {
    let src = blog_module(&format!(
      "{TITLE_OF}\nf(p: Post) {{\n  let a = title_of(p)\n  title_of({{ title: \"About\", body: \"Who we are.\" }})\n}}"
    ));

    assert_eq!(errors_of(&src), Vec::<String>::new());
  }

  #[test]
  fn title_of_missing() {
    let src =
      module(&format!("{TITLE_OF}\nf() {{ title_of({{ body: \"x\" }}) }}"));

    one_error(&src, "POLAR0502 at \"{ body: \\\"x\\\" }\"");
  }

  #[test]
  fn keeps_the_rest() {
    let src = module(
      "keep(r: { title: String | rest }) -> { title: String | rest } { r }\nf() { keep({ title: \"a\", n: 1 }).n + 1 }",
    );

    assert_eq!(errors_of(&src), Vec::<String>::new());
  }

  #[test]
  fn access_on_non_record() {
    one_error(
      &module("f() {\n  let x = 1\n  x.title\n}"),
      "POLAR0501 at \"title\"",
    );
  }

  #[test]
  fn update_replaces() {
    let src = blog_module(&function_from(&example("blog.px"), "publish"));

    assert_eq!(
      type_of(&src, "publish"),
      format!("function({POST}, Int) -> {POST}")
    );
  }

  #[test]
  fn update_adds() {
    assert_eq!(
      type_of(
        &module("f(b: { title: String }) { { ..b, author_id: 1 } }"),
        "f"
      ),
      "function({ title: String }) -> { author_id: Int, title: String }"
    );
  }

  #[test]
  fn update_changes_type() {
    assert_eq!(
      type_of(&module("f(b: { n: Int }) { { ..b, n: \"a\" } }"), "f"),
      "function({ n: Int }) -> { n: String }"
    );
  }

  #[test]
  fn update_unknown_base() {
    assert_eq!(
      type_of(&module("f(b) { { ..b, n: 1 } }"), "f"),
      "function({ n: a | r }) -> { n: Int | r }"
    );
  }

  #[test]
  fn update_non_record() {
    one_error(&module("f() { { ..1, n: 1 } }"), "POLAR0501 at \"1\"");
  }

  #[test]
  fn pattern_open_and_closed() {
    let src = module(
      "f(r) { match r { { a: x, .. } -> x } }\ng(r) { match r { { a: x } -> x } }",
    );

    assert_eq!(type_of(&src, "f"), "function({ a: a | r }) -> a");
    assert_eq!(type_of(&src, "g"), "function({ a: a }) -> a");
  }

  #[test]
  fn describe() {
    let src = blog_module(&function_from(&example("blog.px"), "describe"));

    assert_eq!(
      type_of(&src, "describe"),
      format!("function({POST}) -> String")
    );
  }
}

mod declarations {
  use super::*;

  #[test]
  fn ctor_schemes() {
    let src =
      "types\n  Result<e, a> = Err(e) | Ok(a)\n\nfunctions\n  f() { Err }\n";

    assert_eq!(type_of(src, "f"), "function() -> function(a) -> Result<a, b>");
  }

  #[test]
  fn any_order_in_zone() {
    let src =
      "types\n  A = { b: B }\n  B = X | Y\n\nfunctions\n  f(a: A) { a.b }\n";

    assert_eq!(type_of(src, "f"), "function({ b: B } as A) -> B");
  }

  #[test]
  fn duplicate_type() {
    let src = "types\n  A = { x: Int }\n  A = { y: Int }\n";

    assert_eq!(errors_of(src), vec!["POLAR0302 at \"A\""]);
    assert_eq!(diagnostics_of(src)[0].primary.span, span_of(src, "A", 1));
  }

  #[test]
  fn wrong_type_arg_count() {
    one_error(
      &module("f(x: List<Int, Int>) { x }"),
      "POLAR0506 at \"List<Int, Int>\"",
    );
  }

  #[test]
  fn phantom_alias() {
    let src = "types\n  Id<a> = Int\n  Post = { id: Id<Post> }\n\nfunctions\n  f(x: Id<Post>) -> Int { x }\n";

    assert_eq!(type_of(src, "f"), "function(Int) -> Int");
  }

  #[test]
  fn generic_alias() {
    let src = "types\n  Pair<a> = { l: a, r: a }\n\nfunctions\n  f(p: Pair<Int>) { p.l }\n";

    assert_eq!(type_of(src, "f"), "function({ l: Int, r: Int }) -> Int");
  }

  #[test]
  fn recursive_alias() {
    let src = "types\n  A = { next: A }\n";

    assert_eq!(errors_of(src), vec!["POLAR0510 at \"A\""]);
  }

  #[test]
  fn recursion_through_variant() {
    let src = "types\n  Tree = Leaf | Node({ left: Tree, right: Tree })\n";

    assert_eq!(errors_of(src), Vec::<String>::new());
  }

  #[test]
  fn unknown_type() {
    let src = "types\n  Post = { id: Int }\n\nfunctions\n  f(p: Pots) { p }\n";
    let diagnostics = diagnostics_of(src);

    assert_eq!(errors_of(src), vec!["POLAR0505 at \"Pots\""]);
    assert!(
      diagnostics[0].help.as_deref().is_some_and(|h| h.contains("Post")),
      "{:?}",
      diagnostics[0].help
    );
  }

  #[test]
  fn unbound_type_variable() {
    assert_eq!(
      errors_of("types\n  Box = Box(a)\n"),
      vec!["POLAR0511 at \"a\""]
    );
  }

  #[test]
  fn effect_rows_ignored() {
    let codes = codes_of(&example("effects.px"));

    assert!(!codes.contains(&"POLAR0505".to_string()), "{codes:?}");
  }

  #[test]
  fn blog_types() {
    let src = blog_module("f(p: Post) { p.status }");

    assert_eq!(type_of(&src, "f"), format!("function({POST}) -> Status"));
  }
}

mod top_level {
  use super::*;

  #[test]
  fn rigid_signature() {
    let src = module(&function_from(&std_source("List.px"), "map"));

    assert_eq!(
      type_of(&src, "map"),
      "function(List<a>, function(a) -> b / {| e}) -> List<b> / {| e}"
    );
  }

  #[test]
  fn too_general() {
    one_error(&module("f(x: a) -> a { 1 }"), "POLAR0509 at \"1\"");
  }

  #[test]
  fn same_name_same_rigid() {
    assert_eq!(
      type_of(&module("f(x: a, y: a) -> a { y }"), "f"),
      "function(a, a) -> a"
    );
  }

  #[test]
  fn partial_annotation() {
    let src = module(&function_from(&example("blog.px"), "map"));

    assert_eq!(
      type_of(&src, "map"),
      "function(List<a>, function(a) -> b) -> List<b>"
    );
  }

  #[test]
  fn polymorphic_use_in_module() {
    let src = module("id(x) { x }\nf() {\n  let a = id(1)\n  id(\"s\")\n}");

    assert_eq!(errors_of(&src), Vec::<String>::new());
  }

  #[test]
  fn mutual_recursion() {
    let src = module(
      "even(n) { if n == 0 { true } else { odd(n - 1) } }\nodd(n) { if n == 0 { false } else { even(n - 1) } }",
    );

    assert_eq!(type_of(&src, "even"), "function(Int) -> Bool");
    assert_eq!(type_of(&src, "odd"), "function(Int) -> Bool");
  }

  #[test]
  fn forward_call() {
    assert_eq!(
      type_of(&module("a() { b() }\nb() { 1 }"), "a"),
      "function() -> Int"
    );
  }

  #[test]
  fn annotated_shortcut() {
    let src = module(
      "len(xs: List<a>) -> Int {\n  match xs {\n    [] -> 0,\n    [_, ..rest] -> 1 + len(rest),\n  }\n}\nf() { len([1]) + len([\"a\"]) }",
    );

    assert_eq!(errors_of(&src), Vec::<String>::new());
  }

  #[test]
  fn default_to_int() {
    assert_eq!(
      type_of(&module("add(a, b) { a + b }"), "add"),
      "function(Int, Int) -> Int"
    );
  }

  #[test]
  fn default_is_per_group() {
    one_error(
      &module("add(a, b) { a + b }\nf() { add(1.5, 2.0) }"),
      "POLAR0501 at \"1.5\"",
    );
  }

  #[test]
  fn constants() {
    let src = "constants\n  pi: Float = 3.14\n  tau: Float = 2.0 * pi\n";

    assert_eq!(type_of(src, "pi"), "Float");
    assert_eq!(type_of(src, "tau"), "Float");
  }

  #[test]
  fn constant_strict_numbers() {
    one_error(
      "constants\n  day: Float = 24 * 60 * 60\n",
      "POLAR0501 at \"24 * 60 * 60\"",
    );
  }

  #[test]
  fn unit_3_infinite_type() {
    assert_eq!(codes_of(&module("f(x) { f([x]) }")), vec!["POLAR0504"]);
  }

  #[test]
  fn blog_checks() {
    assert_eq!(
      type_of(&example("blog.px"), "slug"),
      "function({ title: String | r }) -> String"
    );
  }

  #[test]
  fn blog_missing_title() {
    let src = example("blog.px").replacen(
      "    Log.info(slug(post))",
      "    Log.info(title_of({ body: \"x\" }))\n    Log.info(slug(post))",
      1,
    );
    let diagnostics = diagnostics_of(&src);

    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert_eq!(diagnostics[0].code.to_string(), "POLAR0502");
    assert!(
      diagnostics[0].message.contains("title"),
      "{}",
      diagnostics[0].message
    );
  }

  #[test]
  fn effects_checks() {
    assert_eq!(errors_of(&example("effects.px")), Vec::<String>::new());
  }

  #[test]
  fn hello_checks() {
    assert_eq!(type_of(&example("hello.px"), "main"), "function() -> {}");
  }

  #[test]
  fn groups_in_dependency_order() {
    let edges = vec![vec![1, 5, 3], vec![2], vec![], vec![4], vec![3], vec![5]];

    assert_eq!(
      groups(&edges),
      vec![vec![2], vec![1], vec![5], vec![3, 4], vec![0]]
    );
  }
}
