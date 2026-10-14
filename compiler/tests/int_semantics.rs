mod common;

use common::node::run_program;

fn program(body: &str) -> String {
  format!(
    "functions\n  zero() -> Int {{\n    0\n  }}\n\n  big() -> Int {{\n    9007199254740991\n  }}\n\n  main() {{\n    {body}\n  }}\n\nexports\n  main\n"
  )
}

fn failure(body: &str) -> String {
  match run_program(&program(body), "t.px", &[]) {
    Ok(out) => panic!("expected a failure, got {out:?}"),
    Err(e) => e,
  }
}

#[test]
fn division_by_zero_names_its_location() {
  let e = failure("Log.info(Int.to_string(1 / zero()))");

  assert!(e.contains("division by zero (t.px:"), "{e}");
}

#[test]
fn remainder_by_zero_is_an_error_too() {
  let e = failure("Log.info(Int.to_string(1 % zero()))");

  assert!(e.contains("division by zero"), "{e}");
}

#[test]
fn overflow_is_an_error_not_a_rounding() {
  let e = failure("Log.info(Int.to_string(big() + 1))");

  assert!(e.contains("integer overflow"), "{e}");

  let e = failure("Log.info(Int.to_string(big() * 2))");

  assert!(e.contains("integer overflow"), "{e}");
}

#[test]
fn the_largest_int_is_exact() {
  let out =
    run_program(&program("Log.info(Int.to_string(big() - 1))"), "t.px", &[])
      .unwrap();

  assert_eq!(out, "9007199254740990\n");
}

#[test]
fn float_division_by_zero_is_still_infinity() {
  let out =
    run_program(&program("Log.info(\"#{1.0 / 0.0}\")"), "t.px", &[]).unwrap();

  assert_eq!(out, "Infinity\n");
}
