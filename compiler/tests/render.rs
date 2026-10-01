use std::{collections::HashMap, sync::Arc};

use polar_compiler::{
  shared::codes::DiagnosticCode,
  shared::diagnostic::{Diagnostic, Label},
  shared::render::{RenderOptions, render_diagnostic, should_use_color},
  shared::source::{SourceFile, Span},
};

const C: DiagnosticCode = DiagnosticCode::InternalCompilerError;

fn files_with(name: &str, text: &str) -> HashMap<Arc<str>, SourceFile> {
  let mut files = HashMap::new();
  files.insert(Arc::from(name), SourceFile::new(name, text));
  files
}

fn span(file: &str, start: usize, end: usize) -> Span {
  Span::new(file.into(), start, end)
}

mod worked_examples {
  use super::*;

  #[test]
  fn case_1_single_primary_label() {
    let text = "function main() {\n  x\n}\n";
    let files = files_with("a.px", text);
    let x = text.find('x').unwrap();

    let d = Diagnostic::error(
      C,
      "undefined variable `x`",
      Label::new(span("a.px", x, x + 1))
        .with_message("not found in this scope"),
    );

    let out = render_diagnostic(&d, &files, RenderOptions { color: false });

    assert_eq!(
      out,
      "error[POLAR0001]: undefined variable `x`\n --> a.px:2:3\n  |\n2 |   x\n  |   ^ not found in this scope\n"
    );
  }

  #[test]
  fn case_3_secondary_note_and_help() {
    let text = "function title(post: Post) -> String {\n  post.id\n}\n";
    let files = files_with("blog.px", text);
    let post_id = text.find("post.id").unwrap();
    let ret_ty = text.find("String").unwrap();

    let d = Diagnostic::error(
      C,
      "mismatched types",
      Label::new(span("blog.px", post_id, post_id + "post.id".len()))
        .with_message("found `Id<Post>`"),
    )
    .with_secondary(
      Label::new(span("blog.px", ret_ty, ret_ty + "String".len()))
        .with_message("expected `String` because of this return type"),
    )
    .with_note("`Id<Post>` is a distinct type, not an alias for `String`")
    .with_help("convert it with `Id.to_string(post.id)`");

    let out = render_diagnostic(&d, &files, RenderOptions { color: false });

    assert_eq!(
      out,
      "error[POLAR0001]: mismatched types\n\
       \x20--> blog.px:2:3\n\
       \x20\x20|\n\
       1 | function title(post: Post) -> String {\n\
       \x20\x20|                               ------ expected `String` because of this return type\n\
       2 |   post.id\n\
       \x20\x20|   ^^^^^^^ found `Id<Post>`\n\
       \x20\x20|\n\
       \x20\x20= note: `Id<Post>` is a distinct type, not an alias for `String`\n\
       \x20\x20= help: convert it with `Id.to_string(post.id)`\n"
    );
  }

  #[test]
  fn case_4_two_labels_one_line() {
    let text = "let x = a + b\n";
    let files = files_with("two.px", text);
    let a = text.find('a').unwrap();
    let b = text.find('b').unwrap();

    let d = Diagnostic::error(
      C,
      "operands disagree",
      Label::new(span("two.px", a, a + 1)).with_message("left"),
    )
    .with_secondary(Label::new(span("two.px", b, b + 1)).with_message("right"));

    let out = render_diagnostic(&d, &files, RenderOptions { color: false });

    assert_eq!(
      out,
      "error[POLAR0001]: operands disagree\n --> two.px:1:9\n  |\n1 | let x = a + b\n  |         ^ left\n  |             - right\n"
    );
  }

  #[test]
  fn case_8b_empty_span_at_eof() {
    let text = "a\n";
    let files = files_with("eof.px", text);

    let d = Diagnostic::error(
      C,
      "unexpected end of file",
      Label::new(span("eof.px", 2, 2)).with_message("expected `}`"),
    );

    let out = render_diagnostic(&d, &files, RenderOptions { color: false });

    assert_eq!(
      out,
      "error[POLAR0001]: unexpected end of file\n --> eof.px:2:1\n  |\n1 | a\n  |  ^ expected `}`\n"
    );
  }

  #[test]
  fn case_9_tab_and_multibyte_alignment() {
    let text = "\tlet é = 😀\n";
    let files = files_with("uni.px", text);
    let emoji = text.find('😀').unwrap();

    let d = Diagnostic::error(
      C,
      "unexpected token",
      Label::new(span("uni.px", emoji, emoji + '😀'.len_utf8()))
        .with_message("here"),
    );

    let out = render_diagnostic(&d, &files, RenderOptions { color: false });

    assert_eq!(
      out,
      "error[POLAR0001]: unexpected token\n --> uni.px:1:10\n  |\n1 |     let é = 😀\n  |             ^ here\n"
    );
  }
}

mod should_use_color_table {
  use super::*;

  fn env(vars: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
    let vars: Vec<(String, String)> =
      vars.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    move |key: &str| vars.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone())
  }

  #[test]
  fn row_84() {
    assert!(should_use_color(&env(&[("NO_COLOR", "1")]), false, Some(true)));
  }

  #[test]
  fn row_85() {
    assert!(!should_use_color(
      &env(&[("FORCE_COLOR", "1")]),
      true,
      Some(false)
    ));
  }

  #[test]
  fn row_86() {
    assert!(!should_use_color(
      &env(&[("NO_COLOR", "1"), ("FORCE_COLOR", "1")]),
      true,
      None
    ));
  }

  #[test]
  fn row_87() {
    assert!(should_use_color(&env(&[("NO_COLOR", "")]), true, None));
  }

  #[test]
  fn row_88() {
    assert!(should_use_color(&env(&[("FORCE_COLOR", "1")]), false, None));
  }

  #[test]
  fn row_89() {
    assert!(!should_use_color(&env(&[("FORCE_COLOR", "0")]), false, None));
  }

  #[test]
  fn row_90() {
    assert!(should_use_color(&env(&[]), true, None));
  }

  #[test]
  fn row_91() {
    assert!(!should_use_color(&env(&[]), false, None));
  }
}

#[test]
fn coloured_output_strips_to_plain() {
  let text = "let x = a + b\n";
  let files = files_with("two.px", text);
  let a = text.find('a').unwrap();

  let d = Diagnostic::error(
    C,
    "operands disagree",
    Label::new(span("two.px", a, a + 1)).with_message("left"),
  );

  let plain = render_diagnostic(&d, &files, RenderOptions { color: false });
  let coloured = render_diagnostic(&d, &files, RenderOptions { color: true });

  assert_ne!(plain, coloured);
  assert_eq!(strip_ansi(&coloured), plain);
}

fn strip_ansi(s: &str) -> String {
  let mut out = String::new();
  let mut chars = s.chars().peekable();

  while let Some(c) = chars.next() {
    if c == '\u{1b}' && chars.peek() == Some(&'[') {
      chars.next();
      for c in chars.by_ref() {
        if c.is_ascii_alphabetic() {
          break;
        }
      }
      continue;
    }

    out.push(c);
  }

  out
}
