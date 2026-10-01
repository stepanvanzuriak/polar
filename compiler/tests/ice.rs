mod common;

use std::backtrace::BacktraceStatus;

use common::expect_ice;
use polar_compiler::{
  shared::ice::{ice, invariant},
  shared::source::Span,
};

mod all {
  use super::*;

  #[test]
  fn ice_payload_type() {
    let err = expect_ice(|| {
      ice("boom", None);
    });

    assert_eq!(err.message, "boom");
    assert_eq!(err.span, None);
  }

  #[test]
  fn ice_display_without_span() {
    let err = expect_ice(|| {
      ice("boom", None);
    });
    let display = err.to_string();

    assert!(display.contains("internal compiler error: boom"));
    assert!(display.contains("this is a bug in the Polar compiler"));
  }

  #[test]
  fn ice_display_with_span() {
    let span = Span::new("t".into(), 2, 5);
    let err = expect_ice(|| {
      ice("boom", Some(&span));
    });
    let display = err.to_string();

    assert!(display.contains("t@2..5"));
  }

  #[test]
  fn ice_captures_backtrace() {
    let err = expect_ice(|| {
      ice("boom", None);
    });

    assert_eq!(err.backtrace.status(), BacktraceStatus::Captured);
  }

  #[test]
  fn invariant_true_is_silent() {
    invariant(true, || panic!("not built"));
  }

  #[test]
  fn invariant_false_ices() {
    let err = expect_ice(|| {
      invariant(false, || "bad index".into());
    });

    assert_eq!(err.message, "bad index");
  }

  fn calls_invariant() {
    invariant(false, || "boom".into());
  }

  #[test]
  fn ice_location_is_callers() {
    let err = expect_ice(calls_invariant);

    assert_eq!(err.location.file(), file!());
  }
}
