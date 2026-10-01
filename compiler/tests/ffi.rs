mod common;

use common::node::run_program_with;
use polar_compiler::{CompileOptions, compile};

const FFI_JS: &str = "export function sum(xs) {
  if (!Array.isArray(xs)) throw new Error(`not an array: ${xs}`);
  return xs.reduce((a, b) => a + b, 0);
}

export function lookup(key) {
  return key === \"known\" ? \"found\" : null;
}

export function first_or_null(maybe) {
  return maybe === null ? \"none\" : String(maybe);
}

export function tag_count(post) {
  return `${post.title}: ${post.tags.length}`;
}

export function make_post(title) {
  return { title, tags: [\"a\", \"b\"] };
}

export function shout(s, ...rest) {
  if (rest.length) throw new Error(`extra arguments: ${rest.length}`);
  return s.toUpperCase();
}

export async function later(n) {
  return n + 1;
}
";

const EXTERNS: &str = "externs
  sum(xs: List<Int>) -> Int = \"./ffi.js\" sum
  lookup(key: String) -> Option<String> = \"./ffi.js\" lookup
  first_or_null(maybe: Option<Int>) -> String = \"./ffi.js\" first_or_null
  tag_count(post: { title: String, tags: List<String> }) -> String = \"./ffi.js\" tag_count
  make_post(title: String) -> { title: String, tags: List<String> } = \"./ffi.js\" make_post
  shout(s: String) -> String = \"./ffi.js\" shout
";

fn program(fns: &str, main: &str) -> String {
  let main: String = main.lines().flat_map(|l| ["    ", l, "\n"]).collect();

  format!(
    "uses\n  Std.List\n  Std.Option\n\n{EXTERNS}\nfunctions\n{fns}\n  main() {{\n{main}  }}\n\nexports\n  main\n"
  )
}

fn run(src: &str) -> String {
  run_program_with(src, "app.px", &[], None, &[("ffi.js", FFI_JS)])
    .unwrap_or_else(|e| panic!("{e}"))
}

fn js(src: &str) -> String {
  let out = compile(src, "app.px", &CompileOptions::default());

  assert!(out.diagnostics.is_empty(), "{:#?}", out.diagnostics);
  out.js
}

#[test]
fn list_argument_and_option_result_round_trip() {
  let src = program(
    "",
    "Log.info(\"#{sum([1, 2, 3])}\")\nLog.info(Option.with_default(lookup(\"known\"), \"-\"))\nLog.info(Option.with_default(lookup(\"other\"), \"-\"))",
  );

  assert_eq!(run(&src), "6\nfound\n-\n");
}

#[test]
fn option_argument_becomes_null_or_the_value() {
  let src = program(
    "",
    "Log.info(first_or_null(Some(4)))\nLog.info(first_or_null(None))",
  );

  assert_eq!(run(&src), "4\nnone\n");
}

#[test]
fn record_fields_convert_both_ways() {
  let src = program(
    "",
    "Log.info(tag_count({ title: \"t\", tags: [\"x\", \"y\", \"z\"] }))\nlet post = make_post(\"p\")\nLog.info(\"#{List.length(post.tags)} #{List.join(post.tags, \",\")}\")",
  );

  assert_eq!(run(&src), "t: 3\n2 a,b\n");
}

#[test]
fn extern_is_callable_from_a_pure_function_without_await() {
  let src = program(
    "  total(xs) {\n    sum(xs) + 1\n  }\n",
    "Log.info(\"#{total([1, 2])}\")",
  );
  let js = js(&src);

  assert!(js.contains("function total(xs) {"), "{js}");
  assert!(!js.contains("async function total"), "{js}");
  assert!(!js.contains("await"), "{js}");
  assert_eq!(run(&src), "4\n");
}

#[test]
fn extern_as_a_value_never_sees_the_async_flag() {
  let src = format!(
    "uses\n  Std.List\n  Std.Option\n\nhosts\n  Node\n\neffects\n  Clock in Node {{\n    now() -> Int\n  }}\n\n{EXTERNS}\nbinds\n  Clock in Node {{\n    now() {{\n      1\n    }}\n  }}\n\n\
functions\n  twice(f: function(String) -> String / {{| e}}, s: String) -> String / {{| e}} {{\n    f(f(s))\n  }}\n\n  \
main() {{\n    Log.info(List.join(List.map([\"a\", \"b\"], shout), \",\"))\n    \
Log.info(twice(function(s) {{ \"#{{shout(s)}}#{{Clock.now()}}\" }}, \"x\"))\n    \
Log.info(twice(shout, \"y\"))\n  }}\n\nexports\n  main\n"
  );
  let js = js(&src);

  assert!(
    js.contains("$rt.extern($js$shout, [null], null, \"shout\")"),
    "{js}"
  );
  assert_eq!(
    run_program_with(&src, "app.px", &[], Some("Node"), &[("ffi.js", FFI_JS)])
      .unwrap_or_else(|e| panic!("{e}")),
    "A,B\nX11\nY\n"
  );
}

#[test]
fn plain_extern_called_directly_is_imported_directly() {
  let src = program("", "Log.info(shout(\"q\"))");
  let js = js(&src);

  assert!(js.contains("shout as $ext$shout"), "{js}");
  assert!(!js.contains("$js$shout"), "{js}");
  assert_eq!(run(&src), "Q\n");
}

#[test]
fn extern_in_a_bind_is_still_awaited() {
  let src = "hosts\n  Node\n\neffects\n  Tick in Node {\n    next(n: Int) -> Int\n  }\n\nexterns\n  later(n: Int) -> Int = \"./ffi.js\" later\n\nbinds\n  Tick in Node {\n    next(n) {\n      later(n)\n    }\n  }\n\nfunctions\n  main() {\n    Log.info(\"#{Tick.next(1)}\")\n  }\n\nexports\n  main\n";

  assert!(js(src).contains("await $ext$later(n)"));
  assert_eq!(
    run_program_with(src, "app.px", &[], Some("Node"), &[("ffi.js", FFI_JS)])
      .unwrap_or_else(|e| panic!("{e}")),
    "2\n"
  );
}
