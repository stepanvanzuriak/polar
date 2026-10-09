mod common;

use common::node::run_program;
use polar_compiler::{
  CompileOptions, compile,
  shared::{
    codes::DiagnosticCode::{
      self, AlternativeBindings, MatchPatternCount, MissingElse,
      NonExhaustiveMatch, PrivateTypeExposed, ReturnOutsideFunction,
      UnreachableArm,
    },
    diagnostic::Severity,
    modules::ModuleSource,
  },
};

fn program(types: &str, functions: &str, main: &str) -> String {
  let main: String = main.lines().flat_map(|l| ["    ", l, "\n"]).collect();

  format!(
    "uses\n  Std.List\n  Std.Option\n\ntypes\n  Shape = Circle(Int) | Square(Int) | Dot\n{types}\nfunctions\n{functions}\n  main() {{\n{main}  }}\n\nexports\n  main\n"
  )
}

fn run(src: &str) -> String {
  run_program(src, "app.px", &[]).unwrap_or_else(|e| panic!("{e}"))
}

fn codes(src: &str) -> Vec<DiagnosticCode> {
  compile(src, "app.px", &CompileOptions::default())
    .diagnostics
    .iter()
    .map(|d| d.code)
    .collect()
}

fn errors(src: &str) -> Vec<String> {
  compile(src, "app.px", &CompileOptions::default())
    .diagnostics
    .iter()
    .filter(|d| d.severity == Severity::Error)
    .map(|d| d.message.clone())
    .collect()
}

mod guards {
  use super::*;

  const SIZE: &str = "  size(shape: Shape) -> Int {\n    match shape {\n      Circle(n) if n > 10 -> 100,\n      Circle(n) -> n,\n      Square(n) -> n * n,\n      Dot -> 0,\n    }\n  }\n";

  #[test]
  fn a_failing_guard_falls_through() {
    let src = program(
      "",
      SIZE,
      "Log.info(\"#{size(Circle(3))} #{size(Circle(30))} #{size(Square(2))}\")",
    );

    assert_eq!(run(&src), "3 100 4\n");
  }

  #[test]
  fn a_guarded_arm_does_not_cover_its_pattern() {
    let src = program(
      "",
      "  size(shape: Shape) -> Int {\n    match shape {\n      Circle(n) if n > 10 -> 100,\n      Square(n) -> n,\n      Dot -> 0,\n    }\n  }\n",
      "Log.info(\"#{size(Dot)}\")",
    );

    assert_eq!(codes(&src), [NonExhaustiveMatch]);
  }

  #[test]
  fn the_guard_runs_only_after_the_pattern_matches() {
    let src = program(
      "",
      "  first(xs: List<Int>) -> Int {\n    match xs {\n      [x, .._] if x > 0 -> x,\n      _ -> -1,\n    }\n  }\n",
      "Log.info(\"#{first([])} #{first([0])} #{first([5])}\")",
    );

    assert_eq!(run(&src), "-1 -1 5\n");
  }

  #[test]
  fn a_guard_must_be_a_bool() {
    let src = program(
      "",
      "  f(n: Int) -> Int {\n    match n {\n      x if x -> 1,\n      _ -> 0,\n    }\n  }\n",
      "Log.info(\"#{f(1)}\")",
    );

    assert!(!errors(&src).is_empty());
  }
}

mod alternatives {
  use super::*;

  #[test]
  fn literals() {
    let src = program(
      "",
      "  sign(text: String) -> Int {\n    match text {\n      \"+\" | \"plus\" -> 1,\n      \"-\" | \"minus\" -> -1,\n      _ -> 0,\n    }\n  }\n",
      "Log.info(\"#{sign(\"plus\")} #{sign(\"-\")} #{sign(\"?\")}\")",
    );

    assert_eq!(run(&src), "1 -1 0\n");
  }

  #[test]
  fn alternatives_share_their_bindings() {
    let src = program(
      "",
      "  size(shape: Shape) -> Int {\n    match shape {\n      Circle(n) | Square(n) -> n,\n      Dot -> 0,\n    }\n  }\n",
      "Log.info(\"#{size(Circle(2))} #{size(Square(5))} #{size(Dot)}\")",
    );

    assert_eq!(run(&src), "2 5 0\n");
  }

  #[test]
  fn nested_alternatives() {
    let src = program(
      "",
      "  small(o: Option<Int>) -> Bool {\n    match o {\n      Some(1 | 2 | 3) -> true,\n      _ -> false,\n    }\n  }\n",
      "Log.info(\"#{small(Some(2))} #{small(Some(9))} #{small(None)}\")",
    );

    assert_eq!(run(&src), "true false false\n");
  }

  #[test]
  fn every_alternative_binds_the_same_names() {
    let src = program(
      "",
      "  size(shape: Shape) -> Int {\n    match shape {\n      Circle(n) | Dot -> 1,\n      Square(n) -> n,\n    }\n  }\n",
      "Log.info(\"#{size(Dot)}\")",
    );

    assert!(codes(&src).contains(&AlternativeBindings), "{:?}", codes(&src));
  }

  #[test]
  fn alternatives_count_for_exhaustiveness() {
    let src = program(
      "",
      "  size(shape: Shape) -> Int {\n    match shape {\n      Circle(n) | Square(n) -> n,\n      Dot -> 0,\n      _ -> 1,\n    }\n  }\n",
      "Log.info(\"#{size(Dot)}\")",
    );

    assert_eq!(codes(&src), [UnreachableArm]);
  }
}

mod subjects {
  use super::*;

  const BOTH: &str = "  both(a: Option<Int>, b: Option<Int>) -> String {\n    match a, b {\n      Some(x), Some(y) if x == y -> \"same #{x}\",\n      Some(x), Some(y) -> \"#{x} and #{y}\",\n      Some(x), None | None, Some(x) -> \"one #{x}\",\n      None, None -> \"none\",\n    }\n  }\n";

  #[test]
  fn several_values_are_matched_together() {
    let src = program(
      "",
      BOTH,
      "Log.info(both(Some(1), Some(1)))\nLog.info(both(Some(1), Some(2)))\nLog.info(both(None, Some(5)))\nLog.info(both(None, None))",
    );

    assert_eq!(run(&src), "same 1\n1 and 2\none 5\nnone\n");
  }

  #[test]
  fn a_missing_case_names_every_value() {
    let src = program(
      "",
      "  f(a: Option<Int>, b: Bool) -> Int {\n    match a, b {\n      Some(x), _ -> x,\n      None, true -> 0,\n    }\n  }\n",
      "Log.info(\"#{f(None, true)}\")",
    );
    let out = compile(&src, "app.px", &CompileOptions::default());
    let missing = &out.diagnostics[0];

    assert_eq!(missing.code, NonExhaustiveMatch);
    assert_eq!(
      missing.primary.message.as_deref(),
      Some("`None, false` is not covered")
    );
  }

  #[test]
  fn each_arm_has_one_pattern_per_value() {
    let src = program(
      "",
      "  f(a: Int, b: Int) -> Int {\n    match a, b {\n      1 -> 1,\n      _, _ -> 0,\n    }\n  }\n",
      "Log.info(\"#{f(1, 2)}\")",
    );

    assert!(codes(&src).contains(&MatchPatternCount), "{:?}", codes(&src));
  }
}

mod early_return {
  use super::*;

  #[test]
  fn return_leaves_the_function() {
    let src = program(
      "",
      "  parse(text: String) -> Option<Int> {\n    let pair = match String.split_once(text, \"=\") {\n      Some(p) -> p,\n      None -> return None,\n    }\n\n    if pair.before == \"\" { return None }\n\n    Int.parse(pair.after)\n  }\n",
      "Log.info(\"#{parse(\"a=5\")} #{parse(\"=5\")} #{parse(\"x\")}\")",
    );

    assert_eq!(run(&src), "Some(5) None None\n");
  }

  #[test]
  fn return_in_a_lambda_leaves_the_lambda() {
    let src = program(
      "",
      "  evens(xs: List<Int>) -> List<Int> {\n    List.filter(xs, function(x) {\n      if x < 0 { return false }\n\n      x % 2 == 0\n    })\n  }\n",
      "Log.info(List.join(List.map(evens([-2, 1, 2, 4]), Int.to_string), \",\"))",
    );

    assert_eq!(run(&src), "2,4\n");
  }

  #[test]
  fn a_returned_self_call_does_not_grow_the_stack() {
    let src = program(
      "",
      "  count(n: Int, acc: Int) -> Int {\n    if n == 0 { return acc }\n\n    let next = if n > 10 { return count(n - 1, acc + 1) } else { n - 1 }\n\n    count(next, acc + 1)\n  }\n",
      "Log.info(\"#{count(300000, 0)}\")",
    );

    assert_eq!(run(&src), "300000\n");
  }

  #[test]
  fn bare_return_gives_unit() {
    let src = program(
      "",
      "  greet(loud: Bool) {\n    if !loud { return }\n\n    Log.info(\"HI\")\n  }\n",
      "greet(false)\ngreet(true)",
    );

    assert_eq!(run(&src), "HI\n");
  }

  #[test]
  fn the_returned_value_must_fit_the_return_type() {
    let src = program(
      "",
      "  f(n: Int) -> Int {\n    if n > 0 { return \"big\" }\n\n    n\n  }\n",
      "Log.info(\"#{f(1)}\")",
    );

    assert!(!errors(&src).is_empty());
  }

  #[test]
  fn return_outside_a_function() {
    let src = "constants\n  limit: Int = return 3\n";

    assert!(codes(src).contains(&ReturnOutsideFunction), "{:?}", codes(src));
  }

  #[test]
  fn if_without_else_needs_a_unit_branch() {
    let src = program(
      "",
      "  f(n: Int) -> Int {\n    if n > 0 { 1 }\n\n    n\n  }\n",
      "Log.info(\"#{f(1)}\")",
    );

    assert_eq!(codes(&src), [MissingElse]);
  }
}

mod private_types {
  use super::*;

  fn lib(exports: &str) -> String {
    format!(
      "module Lib\n\ntypes\n  Shape = Circle(Int)\n\n  Secret = Secret(Int)\n\nfunctions\n  make() -> Shape {{\n    Circle(1)\n  }}\n\n  hide() -> Secret {{\n    Secret(1)\n  }}\n\nexports\n{exports}"
    )
  }

  fn with_lib(exports: &str, main: &str) -> Vec<String> {
    let modules = vec![ModuleSource {
      path: "Lib".to_string(),
      source: lib(exports),
      specifier: "./lib.js".to_string(),
      plugins: Vec::new(),
    }];

    compile(
      &format!("uses\n  Lib\n\nfunctions\n  main() {{\n{main}\n  }}\n\nexports\n  main\n"),
      "main.px",
      &CompileOptions { modules, ..CompileOptions::default() },
    )
    .diagnostics
    .iter()
    .filter(|d| d.severity == Severity::Error)
    .map(|d| d.message.clone())
    .collect()
  }

  #[test]
  fn an_exported_type_and_its_constructors_are_visible() {
    let found = with_lib(
      "  Shape\n  make\n",
      "    match Lib.make() {\n      Circle(n) -> Log.info(\"#{n}\"),\n    }",
    );

    assert!(found.is_empty(), "{found:?}");
  }

  #[test]
  fn an_unexported_type_is_private() {
    let found = with_lib("  Shape\n  make\n", "    Log.info(\"#{Secret(1)}\")");

    assert!(
      found.iter().any(|m| m.contains("cannot find constructor `Secret`")),
      "{found:?}"
    );
  }

  #[test]
  fn an_export_may_not_expose_a_private_type() {
    let src = lib("  Shape\n  make\n  hide\n");
    let out = compile(&src, "lib.px", &CompileOptions::default());

    assert_eq!(out.diagnostics.len(), 1, "{:#?}", out.diagnostics);
    assert_eq!(out.diagnostics[0].code, PrivateTypeExposed);
    assert_eq!(
      out.diagnostics[0].message,
      "`hide` is exported, but it uses the private type `Secret`"
    );
  }

  #[test]
  fn an_exported_type_may_not_expose_a_private_one() {
    let src = "types\n  Inner = Inner(Int)\n\n  Outer = Outer(Inner)\n\nexports\n  Outer\n";

    assert_eq!(codes(src), [PrivateTypeExposed]);
  }

  #[test]
  fn a_private_type_is_left_out_of_the_dts() {
    let src = lib("  Shape\n  make\n");
    let out = compile(&src, "lib.px", &CompileOptions::default());

    assert!(out.dts.contains("export type Shape"), "{}", out.dts);
    assert!(!out.dts.contains("Secret"), "{}", out.dts);
  }
}

mod helpers {
  use super::*;

  #[test]
  fn split_once_chars_and_from_list() {
    let src = "uses\n  Std.List\n  Std.Map\n  Std.Option\n\nfunctions\n  main() {\n    let m = Map.from_list([{ key: \"a\", value: 1 }, { key: \"a\", value: 2 }])\n\n    Log.info(\"#{Map.get(m, \"a\")} #{Map.size(m)}\")\n    Log.info(List.join(String.chars(\"héllo\"), \"|\"))\n    match String.split_once(\"k=v=w\", \"=\") {\n      Some(p) -> Log.info(\"#{p.before} / #{p.after}\"),\n      None -> Log.info(\"none\"),\n    }\n    match String.split_once(\"kv\", \"=\") {\n      Some(_) -> Log.info(\"some\"),\n      None -> Log.info(\"none\"),\n    }\n  }\n\nexports\n  main\n";

    assert_eq!(run(src), "Some(2) 1\nh|é|l|l|o\nk / v=w\nnone\n");
  }
}
