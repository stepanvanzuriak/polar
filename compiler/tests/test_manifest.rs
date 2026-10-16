use polar_compiler::{CompileOptions, compile};

const SOURCE: &str = "module MoneyTest

uses
  Std.Assert

functions
  test_add() -> {} / {Throws<Failed>} {
    Assert.assert(true)
  }

  helper() -> {} {
    {}
  }

  test_sub() -> {} / {Throws<Failed>} {
    Assert.assert(true)
  }

exports
  test_add
  test_sub
  helper
";

#[test]
fn manifest_lists_tests_in_source_order() {
  let out = compile(SOURCE, "src/money_test.px", &CompileOptions::default());
  let tests: Vec<(&str, usize)> =
    out.tests.iter().map(|t| (t.name.as_str(), t.line)).collect();

  assert_eq!(out.module.as_deref(), Some("MoneyTest"));
  assert_eq!(tests, [("test_add", 7), ("test_sub", 15)]);
}

#[test]
fn only_test_files_have_tests() {
  let out = compile(SOURCE, "src/money.px", &CompileOptions::default());

  assert!(out.tests.is_empty());
  assert!(out.diagnostics.is_empty());
}

#[test]
fn test_file_lists_its_hosts() {
  let source = "module PageTest\n\nhosts\n  Browser\n\nfunctions\n  test_ok() {\n    {}\n  }\n";
  let file = polar_compiler::test_file(source, "src/page_test.px", &[]);

  assert_eq!(file.hosts, ["Browser"]);
  assert_eq!(file.module.as_deref(), Some("PageTest"));
  assert_eq!(file.tests.len(), 1);

  let file = polar_compiler::test_file(SOURCE, "src/money_test.px", &[]);

  assert!(file.hosts.is_empty());
  assert_eq!(file.tests.len(), 2);
}
