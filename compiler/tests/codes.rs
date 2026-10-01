use polar_compiler::shared::codes::DiagnosticCode;

mod display {
  use super::*;

  #[test]
  fn display_pads_to_four() {
    assert_eq!(DiagnosticCode::InternalCompilerError.to_string(), "POLAR0001");
  }
}

mod all {
  use super::*;

  #[test]
  fn codes_are_registered() {
    let parser: Vec<String> = DiagnosticCode::ALL
      .iter()
      .filter(|code| (200..300).contains(&(**code as u16)))
      .map(ToString::to_string)
      .collect();

    for n in 201..=208 {
      let code = format!("POLAR{n:04}");

      assert!(parser.contains(&code), "{code} is not registered");
    }
  }

  #[test]
  fn all_is_sorted() {
    assert!(DiagnosticCode::ALL.windows(2).all(|w| w[0] < w[1]));
  }
}
