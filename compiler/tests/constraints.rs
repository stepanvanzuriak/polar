mod common;

use common::types::{Names, ty};
use polar_compiler::{
  Stage,
  check::{self, Types, solve::Evidence},
  core::lower::lower_with,
  dump_stage,
  shared::diagnostic::{Diagnostic, DiagnosticBag},
  shared::source::SourceFile,
  syntax::lexer::lex,
  syntax::parser::parse,
  types::{
    generalise::{generalise_with, instantiate_with_preds},
    print::print_scheme,
    store::Store,
    ty::{Pred, Scheme, Type},
  },
};

fn pred(trait_path: &str, ty: Type) -> Pred {
  Pred { trait_path: trait_path.into(), ty }
}

#[test]
fn print_scheme_with_preds() {
  let mut store = Store::default();
  let mut names = Names::default();

  store.enter_level();

  let ty = ty("function(List<a>) -> String", &mut store, &mut names);
  let a = Type::Var(names.get("a", &mut store));

  store.leave_level();

  let scheme = generalise_with(&store, &ty, &[pred("Describe", a)], false);

  assert_eq!(
    print_scheme(&scheme),
    "function(List<a>) -> String where Describe<a>"
  );
}

#[test]
fn print_two_preds_sorted() {
  let mut store = Store::default();
  let mut names = Names::default();

  store.enter_level();

  let ty = ty("function(a, b) -> Bool", &mut store, &mut names);
  let a = Type::Var(names.get("a", &mut store));
  let b = Type::Var(names.get("b", &mut store));

  store.leave_level();

  let scheme =
    generalise_with(&store, &ty, &[pred("Show", b), pred("Eq", a)], false);

  assert_eq!(
    print_scheme(&scheme),
    "function(a, b) -> Bool where Eq<a>, Show<b>"
  );
}

#[test]
fn same_letters() {
  let mut store = Store::default();
  let mut names = Names::default();

  store.enter_level();

  let ty = ty("function(x, y) -> y", &mut store, &mut names);
  let y = Type::Var(names.get("y", &mut store));

  store.leave_level();

  let scheme = generalise_with(&store, &ty, &[pred("Show", y)], false);

  assert_eq!(print_scheme(&scheme), "function(a, b) -> b where Show<b>");
}

#[test]
fn trait_paths_print_bare() {
  let scheme = Scheme {
    count: 1,
    ty: Type::func(vec![Type::Gen(0)], Type::string()),
    preds: vec![pred("Shapes.Describe", Type::Gen(0))],
  };

  assert_eq!(print_scheme(&scheme), "function(a) -> String where Describe<a>");
}

#[test]
fn instantiate_shares_placeholders() {
  let mut store = Store::default();
  let scheme = Scheme {
    count: 1,
    ty: Type::func(vec![Type::Gen(0)], Type::Gen(0)),
    preds: vec![pred("Eq", Type::Gen(0))],
  };
  let (ty, preds) = instantiate_with_preds(&mut store, &scheme);
  let Type::Fn { params, .. } = ty else { panic!("not a function") };

  assert_eq!(preds.len(), 1);
  assert_eq!(preds[0].ty, params[0]);
  assert!(matches!(preds[0].ty, Type::Var(_)));
}

#[test]
fn generalise_drops_outer_preds() {
  let mut store = Store::default();
  let mut names = Names::default();
  let outer = Type::Var(names.get("o", &mut store));

  store.enter_level();

  let ty = ty("function(a) -> a", &mut store, &mut names);
  let a = Type::Var(names.get("a", &mut store));

  store.leave_level();

  let scheme =
    generalise_with(&store, &ty, &[pred("Show", outer), pred("Eq", a)], false);

  assert_eq!(print_scheme(&scheme), "function(a) -> a where Eq<a>");
}

#[test]
fn no_preds_prints_as_before() {
  let scheme = Scheme::new(1, Type::func(vec![Type::Gen(0)], Type::Gen(0)));

  assert_eq!(print_scheme(&scheme), "function(a) -> a");
}

const DESCRIBE: &str = "traits
  Describe<a> {
    describe(value: a) -> String
  }

types
  Status = Draft | Published(Int)
  List<a> = Nil | Cons(a, List<a>)
";

const IMPLS: &str = "impls
  Describe for Status {
    describe(value) {
      \"status\"
    }
  }

  Describe for List<a> where Describe<a> {
    describe(value) {
      match value {
        Nil -> \"\",
        Cons(head, tail) -> String.concat(describe(head), describe(tail)),
      }
    }
  }

  Describe for Int {
    describe(value) {
      Int.to_string(value)
    }
  }
";

fn indent(src: &str) -> String {
  src.lines().flat_map(|l| ["  ", l, "\n"]).collect()
}

fn with_describe(fns: &str) -> String {
  format!("{DESCRIBE}\nfunctions\n{}\n{IMPLS}", indent(fns))
}

fn types_of(src: &str) -> String {
  let result = dump_stage(src, "test.px", Stage::Types);

  assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
  result.output.unwrap()
}

fn type_of(src: &str, name: &str) -> String {
  let prefix = format!("{name} : ");

  types_of(src)
    .lines()
    .find_map(|l| l.strip_prefix(&prefix))
    .unwrap_or_else(|| panic!("no `{name}` in {}", types_of(src)))
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

fn checked(src: &str) -> Types {
  let file = SourceFile::new("test.px", src);
  let mut bag = DiagnosticBag::default();
  let module = parse(&file, &lex(&file, &mut bag), &mut bag);
  let lowered = lower_with(&module, &[], &mut bag);
  let types = check::check(&module, &lowered, &mut bag);

  assert!(!bag.has_errors(), "{:#?}", bag.into_sorted());
  types
}

fn evidence_at(src: &str, needle: &str, nth: usize) -> Vec<Evidence> {
  let types = checked(src);
  let (needle, len) = match needle.split_once('|') {
    Some((before, after)) => (format!("{before}{after}"), before.len()),
    None => (needle.to_string(), needle.len()),
  };
  let start = src.match_indices(&needle).nth(nth).expect("needle").0;

  types.evidence.get(&(start, start + len)).cloned().unwrap_or_else(|| {
    panic!("no evidence at `{needle}`: {:#?}", types.evidence)
  })
}

fn show(evidence: &Evidence) -> String {
  match evidence {
    Evidence::Param(k) => format!("Param({k})"),
    Evidence::Impl { trait_path, target, args, .. } if args.is_empty() => {
      format!("Impl({trait_path}, {target})")
    }
    Evidence::Impl { trait_path, target, args, .. } => {
      let args: Vec<String> = args.iter().map(show).collect();

      format!("Impl({trait_path}, {target}, [{}])", args.join(", "))
    }
  }
}

fn shown(evidence: &[Evidence]) -> Vec<String> {
  evidence.iter().map(show).collect()
}

#[test]
fn method_value_type() {
  let src = with_describe("f() {\n  describe\n}\n");

  assert_eq!(
    type_of(&src, "f"),
    "function() -> function(a) -> String where Describe<a>"
  );
}

#[test]
fn method_call_type() {
  let src = with_describe("f(x: Status) {\n  describe(x)\n}\n");

  assert_eq!(type_of(&src, "f"), "function(Status) -> String");
}

#[test]
fn method_level_vars() {
  let src = "traits
  Pick<a> {
    pick(value: a, other: b) -> b
  }
";
  let types = checked(src);
  let def = &types.traits[0];
  let (_, scheme) = &def.methods[0];

  assert_eq!(scheme.count, 2);
  assert_eq!(print_scheme(scheme), "function(a, b) -> b where Pick<a>");
}

#[test]
fn impl_target_args_must_be_vars() {
  let src = format!(
    "{DESCRIBE}\nimpls\n  Describe for List<Int> {{\n    describe(value) {{\n      \"\"\n    }}\n  }}\n"
  );

  assert_eq!(errors_of(&src), ["POLAR0506 at \"List<Int>\""]);
}

#[test]
fn impl_bound_var_unknown() {
  let src = format!(
    "{DESCRIBE}\nimpls\n  Describe for List<a> where Describe<b> {{\n    describe(value) {{\n      \"\"\n    }}\n  }}\n"
  );

  assert_eq!(errors_of(&src), ["POLAR0511 at \"b\""]);
}

#[test]
fn impl_body_checked() {
  let src = format!(
    "{DESCRIBE}\nimpls\n  Describe for Status {{\n    describe(value) {{\n      1\n    }}\n  }}\n"
  );
  let diagnostics = diagnostics_of(&src);

  assert_eq!(errors_of(&src), ["POLAR0501 at \"1\""]);
  assert_eq!(diagnostics[0].secondary.len(), 1, "{diagnostics:#?}");

  let secondary = &diagnostics[0].secondary[0];
  let file = SourceFile::new("test.px", src.as_str());

  assert_eq!(file.slice(&secondary.span), "describe(value: a) -> String");
  assert_eq!(
    secondary.message.as_deref(),
    Some("the trait says `function(Status) -> String`")
  );
}

#[test]
fn impl_annotation_checked() {
  let src = format!(
    "{DESCRIBE}\nimpls\n  Describe for Status {{\n    describe(value: Int) -> String {{\n      \"\"\n    }}\n  }}\n"
  );

  assert_eq!(errors_of(&src), ["POLAR0501 at \"Int\""]);
}

#[test]
fn impl_uses_given() {
  let src = with_describe("f() {\n  1\n}\n");

  assert!(errors_of(&src).is_empty(), "{:?}", errors_of(&src));
}

#[test]
fn where_var_must_be_in_signature() {
  let src =
    with_describe("f(x: Int) -> String where Describe<a> {\n  \"\"\n}\n");

  assert_eq!(errors_of(&src), ["POLAR0511 at \"a\""]);
}

#[test]
fn inferred_where() {
  let src =
    with_describe("twice(x) {\n  String.concat(describe(x), describe(x))\n}\n");

  assert_eq!(type_of(&src, "twice"), "function(a) -> String where Describe<a>");
}

#[test]
fn solved_by_impl() {
  let src = with_describe("f() {\n  describe(Draft)\n}\n");

  assert_eq!(type_of(&src, "f"), "function() -> String");
  assert_eq!(
    shown(&evidence_at(&src, "describe", 1)),
    ["Impl(Describe, Status)"]
  );
}

#[test]
fn conditional_impl() {
  let src = with_describe("f() {\n  describe(Cons(Draft, Nil))\n}\n");

  assert_eq!(
    shown(&evidence_at(&src, "describe", 1)),
    ["Impl(Describe, List, [Impl(Describe, Status)])"]
  );
}

#[test]
fn missing_impl() {
  let src = with_describe("f() {\n  describe(function(x) { x })\n}\n");

  assert_eq!(errors_of(&src), ["POLAR0707 at \"describe\""]);
}

#[test]
fn missing_impl_nested() {
  let src = with_describe(
    "f() {\n  describe(Cons(function(x: Int) -> Int { x }, Nil))\n}\n",
  );
  let diagnostics = diagnostics_of(&src);

  assert_eq!(errors_of(&src), ["POLAR0707 at \"describe\""]);
  assert!(
    diagnostics[0].message.contains("function(Int) -> Int"),
    "{diagnostics:#?}"
  );
  assert!(
    diagnostics[0]
      .notes
      .iter()
      .any(|n| n.contains("Describe<List<function(Int) -> Int>>")),
    "{diagnostics:#?}"
  );
}

#[test]
fn given_used() {
  let src =
    with_describe("f(x: a) -> String where Describe<a> {\n  describe(x)\n}\n");

  assert_eq!(shown(&evidence_at(&src, "describe", 1)), ["Param(0)"]);
}

#[test]
fn missing_where() {
  let src = with_describe("f(x: a) -> String {\n  describe(x)\n}\n");

  assert_eq!(errors_of(&src), ["POLAR0709 at \"describe\""]);
}

#[test]
fn annotated_cannot_grow() {
  let src = with_describe("f(x) -> String {\n  describe(x)\n}\n");

  assert_eq!(type_of(&src, "f"), "function(a) -> String where Describe<a>");
}

#[test]
fn ambiguous() {
  let src = format!(
    "{DESCRIBE}\ntraits\n  Make<a> {{\n    make() -> a\n  }}\n\nfunctions\n  f() -> String {{\n    describe(make())\n  }}\n"
  )
  .replacen("types\n", "", 0);
  let src = src.replace(
    "traits\n  Describe<a> {\n    describe(value: a) -> String\n  }\n",
    "traits\n  Describe<a> {\n    describe(value: a) -> String\n  }\n\n  Make<a> {\n    make() -> a\n  }\n",
  );
  let src = src.replace("\ntraits\n  Make<a> {\n    make() -> a\n  }\n", "");

  assert_eq!(errors_of(&src), ["POLAR0708 at \"make\""]);
}

#[test]
fn local_let_not_generalised() {
  let src = with_describe(
    "f() {\n  let d = function(x) { describe(x) }\n  d(Draft)\n}\n",
  );

  assert_eq!(type_of(&src, "f"), "function() -> String");
  assert_eq!(
    shown(&evidence_at(&src, "describe", 1)),
    ["Impl(Describe, Status)"]
  );
}

#[test]
fn local_let_two_types() {
  let src = with_describe(
    "f() {\n  let d = function(x) { describe(x) }\n  let a = d(Draft)\n  d(1)\n}\n",
  );

  assert_eq!(errors_of(&src), ["POLAR0501 at \"1\""]);
}

#[test]
fn recursive_group() {
  let src = with_describe(
    "even(x, n) {\n  if n == 0 { describe(x) } else { odd(x, n - 1) }\n}\n\nodd(x, n) {\n  if n == 0 { describe(x) } else { even(x, n - 1) }\n}\n",
  );

  assert_eq!(
    type_of(&src, "even"),
    "function(a, Int) -> String where Describe<a>"
  );
  assert_eq!(
    type_of(&src, "odd"),
    "function(a, Int) -> String where Describe<a>"
  );
  assert_eq!(shown(&evidence_at(&src, "describe", 1)), ["Param(0)"]);
  assert_eq!(shown(&evidence_at(&src, "odd|(x, n - 1)", 0)), ["Param(0)"]);
  assert_eq!(shown(&evidence_at(&src, "even|(x, n - 1)", 0)), ["Param(0)"]);
}

#[test]
fn numbers_default_first() {
  let src = with_describe("f(x) {\n  describe(x + x)\n}\n");

  assert_eq!(type_of(&src, "f"), "function(Int) -> String");
}

#[test]
fn evidence_order() {
  let src = "traits\n  Describe<a> {\n    describe(value: a) -> String\n  }\n\n  Show2<a> {\n    show2(value: a) -> String\n  }\n\ntypes\n  Status = Draft | Published(Int)\n\nfunctions\n  g(x, y) {\n    String.concat(describe(y), show2(x))\n  }\n\n  h() {\n    g(1, Draft)\n  }\n\nimpls\n  Describe for Status {\n    describe(value) {\n      \"\"\n    }\n  }\n\n  Show2 for Int {\n    show2(value) {\n      \"\"\n    }\n  }\n".to_string();

  assert_eq!(
    type_of(&src, "g"),
    "function(a, b) -> String where Show2<a>, Describe<b>"
  );
  assert_eq!(
    shown(&evidence_at(&src, "g|(1, Draft)", 0)),
    ["Impl(Show2, Int)", "Impl(Describe, Status)"]
  );
}

#[test]
fn decl_preds_follow_the_scheme() {
  let src = with_describe("twice(x) {\n  describe(x)\n}\n");
  let types = checked(&src);

  assert_eq!(types.decl_preds["twice"].len(), 1);
  assert_eq!(&*types.decl_preds["twice"][0].trait_path, "Describe");
}

#[test]
fn imported_trait_scheme() {
  let shapes = "module Shapes

traits
  Describe<a> {
    describe(value: a) -> String
  }

exports
  Describe { describe }
";
  let src =
    "uses\n  Shapes { describe }\n\nfunctions\n  f() {\n    describe\n  }\n";
  let options = polar_compiler::CompileOptions {
    modules: vec![polar_compiler::shared::modules::ModuleSource {
      path: "Shapes".to_string(),
      source: shapes.to_string(),
      specifier: "./shapes.js".to_string(),
      plugins: Vec::new(),
    }],
    ..polar_compiler::CompileOptions::default()
  };
  let result =
    polar_compiler::dump_stage_with(src, "main.px", Stage::Types, &options);

  assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
  assert_eq!(
    result.output.unwrap(),
    "f : function() -> function(a) -> String where Describe<a>\n"
  );
}
