mod common;

use common::expect_ice;
use polar_compiler::shared::source::{Position, SourceFile, Span};

mod line_count {
  use super::*;

  #[test]
  fn mixed_terminators() {
    let source = "Line 1 test\nLine 2 test\rLine 3 test\r\nLine 4 test";

    assert_eq!(SourceFile::new("test", source).line_count(), 4);
  }

  #[test]
  fn with_empty_line() {
    let source = "Line 1\n";

    assert_eq!(SourceFile::new("test", source).line_count(), 2);
  }
}

mod position_at {
  use super::*;

  fn check(text: &str, offset: usize, line: usize, column: usize) {
    assert_eq!(
      SourceFile::new("t", text).position_at(offset),
      Position { line, column },
      "{text:?} @ {offset}"
    );
  }

  #[test]
  fn empty() {
    check("", 0, 0, 0);
  }

  #[test]
  fn eof_without_trailing_newline() {
    check("ab", 2, 0, 2);
  }

  #[test]
  fn on_lf_belongs_to_line() {
    check("a\n", 1, 0, 1);
  }

  #[test]
  fn eof_after_trailing_newline() {
    check("a\n", 2, 1, 0);
  }

  #[test]
  fn on_lf_of_crlf() {
    check("a\r\nb", 2, 0, 2);
  }

  #[test]
  fn after_crlf() {
    check("a\r\nb", 3, 1, 0);
  }

  #[test]
  fn after_lone_cr() {
    check("a\rb", 2, 1, 0);
  }

  #[test]
  fn after_emoji_counts_utf8_bytes() {
    check("😀x", 4, 0, 4);
  }
}

mod offset_at {
  use super::*;

  #[test]
  fn within_first_line() {
    let source = "Line 1 test\nLine 2 test";

    assert_eq!(
      SourceFile::new("test", source)
        .offset_at(Position { line: 0, column: 6 }),
      6
    );
  }
}

mod line_start {
  use super::*;

  fn check(text: &str, line: usize, expected: usize) {
    assert_eq!(
      SourceFile::new("t", text).line_start(line),
      expected,
      "{text:?} line {line}"
    );
  }

  #[test]
  fn first_line() {
    check("ab\ncd", 0, 0);
  }

  #[test]
  fn after_lf() {
    check("ab\ncd", 1, 3);
  }

  #[test]
  fn after_crlf() {
    check("ab\r\ncd", 1, 4);
  }

  #[test]
  fn after_lone_cr() {
    check("ab\rcd", 1, 3);
  }

  #[test]
  fn empty_final_line() {
    check("ab\n", 1, 3);
  }

  #[test]
  fn out_of_range() {
    let err = expect_ice(|| {
      let _ = SourceFile::new("t", "ab").line_start(1);
    });

    assert_eq!(err.message, "line 1 out of range");
  }
}

mod slice {
  use super::*;

  fn check(text: &str, start: usize, end: usize, expected: &str) {
    let span = Span::new("t".into(), start, end);

    assert_eq!(
      SourceFile::new("t", text).slice(&span),
      expected,
      "{text:?} [{start}..{end}]"
    );
  }

  #[test]
  fn within_line() {
    check("let x = 1", 4, 5, "x");
  }

  #[test]
  fn empty_span() {
    check("abc", 1, 1, "");
  }

  #[test]
  fn across_lines_keeps_terminator() {
    check("ab\r\ncd", 1, 5, "b\r\nc");
  }

  #[test]
  fn multibyte() {
    check("é😀x", 2, 6, "😀");
  }

  #[test]
  fn end_past_text() {
    let span = Span::new("t".into(), 0, 4);
    let err = expect_ice(|| {
      let _ = SourceFile::new("t", "abc").slice(&span);
    });

    assert_eq!(err.message, "span out of range");
    assert_eq!(err.span, Some(span));
  }

  #[test]
  fn inside_multibyte_char() {
    let span = Span::new("t".into(), 0, 1);
    let err = expect_ice(|| {
      let _ = SourceFile::new("t", "é").slice(&span);
    });

    assert_eq!(err.message, "span out of range");
    assert_eq!(err.span, Some(span));
  }
}

mod line_text {
  use super::*;

  fn check(text: &str, line: usize, expected: &str) {
    assert_eq!(
      SourceFile::new("t", text).line_text(line),
      expected,
      "{text:?} line {line}"
    );
  }

  #[test]
  fn last_line_without_terminator() {
    check("ab\ncd", 1, "cd");
  }

  #[test]
  fn excludes_lf() {
    check("ab\ncd", 0, "ab");
  }

  #[test]
  fn excludes_crlf() {
    check("ab\r\ncd", 0, "ab");
  }

  #[test]
  fn excludes_lone_cr() {
    check("ab\rcd", 0, "ab");
  }

  #[test]
  fn empty_middle_line() {
    check("ab\n\ncd", 1, "");
  }

  #[test]
  fn empty_final_line() {
    check("ab\n", 1, "");
  }

  #[test]
  fn multibyte() {
    check("é\n😀x", 1, "😀x");
  }

  #[test]
  fn out_of_range() {
    let err = expect_ice(|| {
      let _ = SourceFile::new("t", "ab").line_text(1);
    });

    assert_eq!(err.message, "line 1 out of range");
  }
}
