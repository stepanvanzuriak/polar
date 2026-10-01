mod common;

use polar_compiler::{Stage, dump_stage, shared::source::SourceFile};

fn module(body: &str) -> String {
  format!(
    "module M\n\nfunctions\n  main() {{\n{body}\n  }}\n\nexports\n  main\n"
  )
}

fn errors_of(src: &str) -> Vec<String> {
  let file = SourceFile::new("test.px", src);

  dump_stage(src, "test.px", Stage::Types)
    .diagnostics
    .iter()
    .map(|d| format!("{} at {:?}", d.code, file.slice(&d.primary.span)))
    .collect()
}

fn type_of_f(src: &str) -> String {
  let result = dump_stage(src, "test.px", Stage::Types);

  assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
  result
    .output
    .unwrap()
    .lines()
    .find_map(|l| l.strip_prefix("f : ").map(str::to_string))
    .expect("f")
}

fn run(body: &str) -> String {
  common::node::run_program(&module(body), "test.px", &[])
    .unwrap_or_else(|err| panic!("{err}"))
}

#[test]
fn operators_take_and_return_int() {
  let src = "module M\n\nfunctions\n  f(a, b) {\n    ~(a & b | a ^ b)\n  }\n";

  assert_eq!(type_of_f(src), "function(Int, Int) -> Int");
}

#[test]
fn float_operand_is_rejected() {
  let src = module("    Log.info(\"#{1.5 & 2}\")");

  assert_eq!(errors_of(&src), vec!["POLAR0501 at \"1.5\"".to_string()]);
}

#[test]
fn bool_operand_is_rejected() {
  let src = module("    Log.info(\"#{~true}\")");

  assert_eq!(errors_of(&src), vec!["POLAR0501 at \"true\"".to_string()]);
}

#[test]
fn rosetta_bitwise() {
  let out = run(
    "    let a = 10\n    let b = 2\n    Log.info(\"#{a & b} #{a | b} #{a ^ b} #{~a}\")",
  );

  assert_eq!(out.trim(), "2 10 8 -11");
}

#[test]
fn js_precedence_is_parenthesised() {
  let out = run(
    "    let a = 6\n    let b = 3\n    Log.info(\"#{(a & b) == 2} #{a | b == 7}\")",
  );

  assert_eq!(out.trim(), "true true");
}

#[test]
fn operands_wrap_to_32_bits() {
  let out = run("    Log.info(\"#{4294967296 | 0} #{2147483648 | 0}\")");

  assert_eq!(out.trim(), "0 -2147483648");
}
