mod common;

use common::node::run_program_with;
use polar_compiler::{
  Stage, dump_stage,
  shared::diagnostic::{Diagnostic, Severity},
  shared::source::SourceFile,
};

const PREAMBLE: &str = "module App

hosts
  Node

types
  Missing = Missing(String)
  Invalid = Empty | TooLong(Int)

effects
  Db in Node {
    load(id: Int) -> String
  }
";

fn with_errors(fns: &str) -> String {
  format!(
    "{PREAMBLE}\nfunctions\n  check(s: String) -> String / {{Throws<Invalid>}} {{\n    if s == \"\" {{ throw Empty }} else {{ s }}\n  }}\n\n  {fns}\n"
  )
}

fn type_of(src: &str, name: &str) -> String {
  let result = dump_stage(src, "test.px", Stage::Types);
  let all =
    result.output.unwrap_or_else(|| panic!("{:#?}", result.diagnostics));
  let prefix = format!("{name} : ");

  all
    .lines()
    .find_map(|l| l.strip_prefix(&prefix))
    .unwrap_or_else(|| panic!("no `{name}` in {all}"))
    .to_string()
}

fn diagnostics_of(src: &str) -> Vec<Diagnostic> {
  dump_stage(src, "test.px", Stage::Types)
    .diagnostics
    .into_iter()
    .filter(|d| d.severity == Severity::Error)
    .collect()
}

fn errors_of(src: &str) -> Vec<String> {
  let file = SourceFile::new("test.px", src);

  diagnostics_of(src)
    .iter()
    .map(|d| format!("{} at {:?}", d.code, file.slice(&d.primary.span)))
    .collect()
}

fn only(src: &str) -> Diagnostic {
  let diagnostics = diagnostics_of(src);

  assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
  diagnostics.into_iter().next().unwrap()
}

#[test]
fn throw_performs() {
  assert_eq!(
    type_of(&with_errors("f() { throw Empty }"), "f"),
    "function() -> a / {Throws<Invalid>}"
  );
}

#[test]
fn throw_fits_branch() {
  let src = with_errors(
    "f(s: String) -> String / {Throws<Invalid>} {\n    if s == \"\" { throw Empty } else { s }\n  }",
  );

  assert!(errors_of(&src).is_empty(), "{:?}", errors_of(&src));
}

#[test]
fn throw_needs_named_type() {
  assert_eq!(
    errors_of(&with_errors("f() { throw 1 }")),
    vec!["POLAR0809 at \"1\""]
  );
}

#[test]
fn throw_record_is_not_named() {
  let d = only(&with_errors("f() { throw { code: 1 } }"));

  assert_eq!(d.code.to_string(), "POLAR0809");
}

#[test]
fn throw_in_pure() {
  let d = only(&with_errors("f() -> String { throw Empty }"));

  assert_eq!(d.code.to_string(), "POLAR0808");
  assert!(d.message.contains("says it is pure"), "{}", d.message);
}

#[test]
fn try_removes_label() {
  let src = with_errors(
    "safe(s) {\n    try { check(s) } catch {\n      Empty -> \"e\",\n      TooLong(n) -> \"t\",\n    }\n  }",
  );

  assert_eq!(type_of(&src, "safe"), "function(String) -> String");
}

#[test]
fn try_keeps_others() {
  let src = with_errors(
    "f(s) {\n    try {\n      check(s)\n      throw Missing(\"x\")\n    } catch {\n      Empty -> \"e\",\n      TooLong(_) -> \"t\",\n    }\n  }",
  );

  assert_eq!(
    type_of(&src, "f"),
    "function(String) -> String / {Throws<Missing>}"
  );
}

#[test]
fn try_keeps_effects() {
  let src = with_errors(
    "f(s) {\n    try {\n      Db.load(1)\n      check(s)\n    } catch {\n      Empty -> \"e\",\n      TooLong(_) -> \"t\",\n    }\n  }",
  );

  assert_eq!(type_of(&src, "f"), "function(String) -> String / {Db}");
}

#[test]
fn try_callback_precise() {
  let src = with_errors("safe(f) { try { f() } catch { Missing(k) -> k } }");

  assert_eq!(
    type_of(&src, "safe"),
    "function(function() -> String / {Throws<Missing> | e}) -> String / {| e}"
  );
}

#[test]
fn catch_arms_unify() {
  let src = with_errors(
    "f(s) { try { check(s) } catch { Empty -> \"e\", TooLong(_) -> 1 } }",
  );

  assert_eq!(errors_of(&src), vec!["POLAR0501 at \"1\""]);
}

#[test]
fn catch_not_exhaustive() {
  let d =
    only(&with_errors("f(s) { try { check(s) } catch { Empty -> \"e\" } }"));

  assert_eq!(d.code.to_string(), "POLAR0601");
  assert_eq!(d.primary.message.as_deref(), Some("`TooLong(_)` is not covered"));
}

#[test]
fn catch_wildcard_one_type() {
  let src = with_errors(
    "f(s) { try { check(s) } catch { Empty -> \"e\", _ -> \"other\" } }",
  );

  assert_eq!(type_of(&src, "f"), "function(String) -> String");
}

#[test]
fn catch_wildcard_alone() {
  let src = with_errors("f(s) { try { check(s) } catch { e -> \"x\" } }");

  assert_eq!(errors_of(&src), vec!["POLAR0810 at \"e\""]);
}

#[test]
fn catch_wildcard_ambiguous() {
  let src = with_errors(
    "f(s) { try { check(s) } catch { Empty -> \"\", Missing(_) -> \"\", _ -> \"x\" } }",
  );

  assert_eq!(errors_of(&src), vec!["POLAR0810 at \"_\""]);
}

#[test]
fn catch_literal_arm() {
  let src = with_errors("f(s) { try { check(s) } catch { \"x\" -> \"x\" } }");

  assert_eq!(errors_of(&src), vec!["POLAR0810 at \"\\\"x\\\"\""]);
}

#[test]
fn throw_in_catch_arm() {
  let src = with_errors(
    "f(s) {\n    try { check(s) } catch {\n      Empty -> throw Missing(\"x\"),\n      TooLong(_) -> \"\",\n    }\n  }",
  );

  assert_eq!(
    type_of(&src, "f"),
    "function(String) -> String / {Throws<Missing>}"
  );
}

#[test]
fn main_may_throw() {
  let src = with_errors("main() {\n    Log.info(\"x\")\n    check(\"\")\n  }");

  assert!(errors_of(&src).is_empty(), "{:?}", errors_of(&src));
  assert_eq!(type_of(&src, "main"), "function() -> String / {Throws<Invalid>}");
}

fn run(src: &str, files: &[(&str, &str)]) -> Result<String, String> {
  run_program_with(src, "app.px", &[], Some("Node"), files)
}

#[test]
fn catch_skips_other_types() {
  let src = with_errors(
    "nested(s: String) -> String {\n    try {\n      try {\n        check(s)\n      } catch {\n        Missing(k) -> \"inner #{k}\",\n      }\n    } catch {\n      Empty -> \"outer\",\n      TooLong(_) -> \"long\",\n    }\n  }\n\n  main() {\n    Log.info(nested(\"\"))\n  }\n\nexports\n  main",
  );

  assert_eq!(run(&src, &[]).unwrap(), "outer\n");
}

#[test]
fn catch_skips_js_errors() {
  let src = with_errors(
    "failing(n: Int) -> String / {Db} {\n    try {\n      Db.load(n)\n    } catch {\n      Empty -> \"never\",\n      TooLong(_) -> \"never\",\n    }\n  }\n\n  main() {\n    Log.info(failing(2))\n  }\n\nexports\n  main",
  )
  .replace(
    "\nfunctions\n",
    "\nexterns\n  explode(n: Int) -> String = \"./boom.js\" explode\n\nbinds\n  Db in Node {\n    load(id) {\n      explode(id)\n    }\n  }\n\nfunctions\n",
  );
  let boom =
    "export function explode(n) {\n  throw new TypeError(`boom ${n}`);\n}\n";
  let err = run(&src, &[("boom.js", boom)]).unwrap_err();

  assert!(err.contains("TypeError: boom 2"), "{err}");
  assert!(err.contains("boom.js"), "no stack: {err}");
}

#[test]
fn throw_expression_position() {
  let src = with_errors(
    "g(n: Int) -> Int {\n    n\n  }\n\n  f(c: Bool) -> Int / {Throws<Invalid>} {\n    g(if c { throw Empty } else { 1 })\n  }\n\n  main() {\n    Log.info(\"#{f(false)}\")\n    Log.info(try { \"#{f(true)}\" } catch { Empty -> \"raised\", TooLong(_) -> \"\" })\n  }\n\nexports\n  main",
  );

  assert_eq!(run(&src, &[]).unwrap(), "1\nraised\n");
}

#[test]
fn extern_throws_polar_error() {
  let src = with_errors(
    "get(id: Int) -> String / {Db} {\n    try {\n      Db.load(id)\n    } catch {\n      Missing(k) -> \"missing #{k}\",\n    }\n  }\n\n  main() {\n    Log.info(get(1))\n    Log.info(get(2))\n  }\n\nexports\n  main",
  )
  .replace(
    "  Db in Node {\n    load(id: Int) -> String\n  }\n",
    "  Db in Node {\n    load(id: Int) -> String / {Throws<Missing>}\n  }\n\nexterns\n  lookup(id: Int) -> String / {Throws<Missing>} = \"./lookup.js\" lookup\n\nbinds\n  Db in Node {\n    load(id) {\n      lookup(id)\n    }\n  }\n",
  );
  let lookup = "export function lookup(id) {\n  if (id === 1) return \"found\";\n  throw { [Symbol.for(\"polar.error\")]: true, type: \"App.Missing\", value: { $: \"Missing\", _0: `#${id}` } };\n}\n";

  assert_eq!(
    run(&src, &[("lookup.js", lookup)]).unwrap(),
    "found\nmissing #2\n"
  );
}
