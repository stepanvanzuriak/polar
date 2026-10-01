mod common;

use polar_compiler::{
  Stage, dump_stage, fmt::format, shared::source::SourceFile,
};

const TYPES: &str = "types\n  Entry = { key: String, value: Int }\n  Wrap = Wrap(Int)\n  Shape = Circle(Int) | Square(Int)\n";

fn module(functions: &str) -> String {
  format!("module M\n\n{TYPES}\nfunctions\n{functions}\nexports\n  main\n")
}

fn errors_of(src: &str) -> Vec<String> {
  let file = SourceFile::new("test.px", src);

  dump_stage(src, "test.px", Stage::Types)
    .diagnostics
    .iter()
    .map(|d| format!("{} at {:?}", d.code, file.slice(&d.primary.span)))
    .collect()
}

fn run(functions: &str) -> String {
  common::node::run_program(&module(functions), "test.px", &[])
    .unwrap_or_else(|err| panic!("{err}"))
}

#[test]
fn record_pattern_parameter() {
  let out = run(
    "  describe({ key: k, value: v }: Entry) -> String {\n    \"#{k}=#{v}\"\n  }\n\n  main() {\n    Log.info(describe({ key: \"a\", value: 1 }))\n  }\n",
  );

  assert_eq!(out.trim(), "a=1");
}

#[test]
fn single_constructor_parameter() {
  let out = run(
    "  unwrap(Wrap(n)) -> Int {\n    n\n  }\n\n  main() {\n    Log.info(Int.to_string(unwrap(Wrap(4))))\n  }\n",
  );

  assert_eq!(out.trim(), "4");
}

#[test]
fn lambda_and_wildcard_parameters() {
  let out = run(
    "  main() {\n    let f = function({ key: k, value: _ }, _) { k }\n    Log.info(f({ key: \"z\", value: 2 }, 3))\n  }\n",
  );

  assert_eq!(out.trim(), "z");
}

#[test]
fn nested_pattern_parameter() {
  let out = run(
    "  sum({ key: _, value: v }, Wrap(w)) -> Int {\n    v + w\n  }\n\n  main() {\n    Log.info(Int.to_string(sum({ key: \"k\", value: 2 }, Wrap(3))))\n  }\n",
  );

  assert_eq!(out.trim(), "5");
}

#[test]
fn pattern_parameters_are_typed_from_the_pattern() {
  let src = module(
    "  value_of({ key: _, value: v }) {\n    v + 1\n  }\n\n  main() {\n    Log.info(Int.to_string(value_of({ key: \"k\", value: 1 })))\n  }\n",
  );

  assert!(errors_of(&src).is_empty(), "{:?}", errors_of(&src));
}

#[test]
fn refutable_parameter_is_rejected() {
  let src = module(
    "  radius(Circle(r)) -> Int {\n    r\n  }\n\n  main() {\n    Log.info(Int.to_string(radius(Circle(1))))\n  }\n",
  );

  assert_eq!(errors_of(&src), vec!["POLAR0602 at \"Circle(r)\"".to_string()]);
}

#[test]
fn refutable_parameter_message() {
  let src = "module M\n\nuses\n  Std.List\n\nfunctions\n  first([x, ..rest]: List<Int>) -> Int {\n    x\n  }\n";
  let diagnostics = dump_stage(src, "test.px", Stage::Types).diagnostics;

  assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
  assert!(diagnostics[0].message.contains("parameter pattern"));
}

#[test]
fn method_signatures_keep_plain_names() {
  let src = "module M\n\neffects\n  Db {\n    get({ id: i }: { id: Int }) -> Int\n  }\n";

  assert_eq!(errors_of(src), vec!["POLAR0201 at \"{ id: i }\"".to_string()]);
}

#[test]
fn formatter_round_trips_pattern_parameters() {
  let src =
    "module M\n\nfunctions\n  f(  { a: x }  ,Wrap(y): Wrap ,_ ) { x }\n";
  let out = format(src, "test.px").output.unwrap();

  assert_eq!(
    out,
    "module M\n\nfunctions\n  f({ a: x }, Wrap(y): Wrap, _) {\n    x\n  }\n"
  );
  assert_eq!(format(&out, "test.px").output.unwrap(), out);
}
