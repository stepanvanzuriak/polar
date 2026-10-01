use polar_compiler::{
  shared::codes::DiagnosticCode,
  shared::diagnostic::{Diagnostic, DiagnosticBag, Label, Severity},
  shared::source::Span,
};

const C: DiagnosticCode = DiagnosticCode::InternalCompilerError;

fn sp() -> Span {
  Span::new("t".into(), 0, 1)
}

mod builders {
  use super::*;

  fn l1() -> Label {
    Label::new(sp()).with_message("one")
  }

  fn l2() -> Label {
    Label::new(sp()).with_message("two")
  }

  #[test]
  fn error_defaults() {
    assert_eq!(
      Diagnostic::error(C, "boom", Label::new(sp())),
      Diagnostic {
        severity: Severity::Error,
        code: C,
        message: "boom".into(),
        primary: Label { span: sp(), message: None },
        secondary: vec![],
        notes: vec![],
        help: None,
      }
    );
  }

  #[test]
  fn warning_severity() {
    assert_eq!(
      Diagnostic::warning(C, "boom", Label::new(sp())),
      Diagnostic {
        severity: Severity::Warning,
        code: C,
        message: "boom".into(),
        primary: Label { span: sp(), message: None },
        secondary: vec![],
        notes: vec![],
        help: None,
      }
    );
  }

  #[test]
  fn info_severity() {
    assert_eq!(
      Diagnostic::info(C, "boom", Label::new(sp())),
      Diagnostic {
        severity: Severity::Info,
        code: C,
        message: "boom".into(),
        primary: Label { span: sp(), message: None },
        secondary: vec![],
        notes: vec![],
        help: None,
      }
    );
  }

  #[test]
  fn label_without_message() {
    let label = Label::new(sp());

    assert_eq!(label.message, None);
  }

  #[test]
  fn label_with_message() {
    let label = Label::new(sp()).with_message("found `Id<Post>`");

    assert_eq!(label.message, Some(String::from("found `Id<Post>`")));
  }

  #[test]
  fn secondaries_append_in_order() {
    let d = Diagnostic::error(C, "boom", Label::new(sp()))
      .with_secondary(l1())
      .with_secondary(l2());

    assert_eq!(d.secondary, [l1(), l2()]);
  }

  #[test]
  fn notes_append_in_order() {
    let d = Diagnostic::error(C, "boom", Label::new(sp()))
      .with_note("a")
      .with_note("b");

    assert_eq!(d.notes, ["a", "b"]);
  }

  #[test]
  fn help_replaces() {
    let d = Diagnostic::error(C, "boom", Label::new(sp()))
      .with_help("x")
      .with_help("y");

    assert_eq!(d.help, Some(String::from("y")));
  }
}

mod diagnostic_bag {
  use super::*;

  fn span(file: &str, start: usize, end: usize) -> Span {
    Span::new(file.into(), start, end)
  }

  fn e(span: Span, message: impl Into<String>) -> Diagnostic {
    Diagnostic::error(C, message, Label::new(span))
  }

  fn w(span: Span, message: impl Into<String>) -> Diagnostic {
    Diagnostic::warning(C, message, Label::new(span))
  }

  fn messages(bag: DiagnosticBag) -> Vec<String> {
    bag.into_sorted().into_iter().map(|d| d.message).collect()
  }

  #[test]
  fn dedups_identical() {
    let item = e(sp(), "Test");

    let mut bag = DiagnosticBag::default();
    bag.push(item.clone());
    bag.push(item.clone());

    assert_eq!(bag.into_sorted().len(), 1);
  }

  #[test]
  fn same_span_different_message() {
    let mut bag = DiagnosticBag::default();
    bag.push(e(span("t", 2, 3), "a"));
    bag.push(e(span("t", 2, 3), "b"));

    assert_eq!(bag.into_sorted().len(), 2);
  }

  #[test]
  fn same_span_different_severity() {
    let mut bag = DiagnosticBag::default();
    bag.push(e(span("t", 2, 3), "a"));
    bag.push(w(span("t", 2, 3), "a"));

    assert_eq!(bag.into_sorted().len(), 2);
  }

  #[test]
  fn sorts_by_start() {
    let mut bag = DiagnosticBag::default();
    bag.push(e(span("t", 9, 10), "b"));
    bag.push(e(span("t", 2, 3), "a"));

    assert_eq!(messages(bag), ["a", "b"]);
  }

  #[test]
  fn sorts_by_end_within_start() {
    let mut bag = DiagnosticBag::default();
    bag.push(e(span("t", 2, 9), "b"));
    bag.push(e(span("t", 2, 3), "a"));

    assert_eq!(messages(bag), ["a", "b"]);
  }

  #[test]
  fn ties_keep_insertion_order() {
    let mut bag = DiagnosticBag::default();
    bag.push(e(span("t", 4, 5), "first"));
    bag.push(e(span("t", 4, 5), "second"));

    assert_eq!(messages(bag), ["first", "second"]);
  }

  #[test]
  fn sorts_by_file_first() {
    let mut bag = DiagnosticBag::default();
    bag.push(e(span("b", 0, 1), "b"));
    bag.push(e(span("a", 0, 1), "a"));

    assert_eq!(messages(bag), ["a", "b"]);
  }

  #[test]
  fn warnings_are_not_errors() {
    let mut bag = DiagnosticBag::default();
    bag.push(w(span("t", 0, 1), "w"));

    assert!(!bag.has_errors());
    assert_eq!(bag.error_count(), 0);
    assert_eq!(bag.warning_count(), 1);
  }

  #[test]
  fn counts_ignore_duplicates() {
    let item = e(span("t", 0, 1), "a");

    let mut bag = DiagnosticBag::default();
    bag.push(item.clone());
    bag.push(item);

    assert_eq!(bag.error_count(), 1);
  }

  #[test]
  fn empty_bag() {
    let bag = DiagnosticBag::default();

    assert!(!bag.has_errors());
    assert_eq!(bag.into_sorted(), Vec::new());
  }
}
