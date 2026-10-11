use polar_cli::expect::{mismatches, parse};

#[test]
fn expectation_applies_to_next_code_line() {
  let text = "module M\n// expect-error: POLAR0504 \"no action `show`\"\n// expect-error: POLAR0310\n\nroutes\n";
  let got = parse(text).unwrap();

  assert_eq!(got.len(), 2);
  assert_eq!(got[0].code, "POLAR0504");
  assert_eq!(got[0].message.as_deref(), Some("no action `show`"));
  assert_eq!(got[0].line, 5);
  assert_eq!(got[1].line, 5);
}

#[test]
fn malformed_expectations_are_reported() {
  assert!(parse("// expect-error: nope\nx").is_err());
  assert!(parse("// expect-error: POLAR0001\n").is_err());
  assert!(parse("// expect-error: POLAR0001 oops\nx").is_err());
}

#[test]
fn mismatches_name_each_problem() {
  let expected = parse("// expect-error: POLAR0001 \"a\"\nx\n").unwrap();
  let ok = [("POLAR0001".to_string(), 2, "has a here".to_string())];

  assert!(mismatches(&expected, &ok).is_empty());

  let moved = [("POLAR0001".to_string(), 3, "a".to_string())];

  assert_eq!(mismatches(&expected, &moved).len(), 2);
  assert_eq!(mismatches(&expected, &[]).len(), 1);
}
