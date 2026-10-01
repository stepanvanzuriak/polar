use polar_compiler::{
  shared::diagnostic::{Diagnostic, DiagnosticBag},
  shared::source::SourceFile,
  syntax::lexer::{
    LexResult, lex,
    token::{TokenKind, keyword},
  },
};
use std::collections::HashSet;

fn tokens(src: &str) -> Vec<String> {
  let file = SourceFile::new("test.px", src);
  let mut bag = DiagnosticBag::default();

  lex(&file, &mut bag)
    .tokens
    .iter()
    .map(|t| {
      let nl = if t.newline_before { "↵" } else { "" };
      format!("{nl}{:?}@{}..{}", t.kind, t.span.start, t.span.end)
    })
    .collect()
}

fn comments(src: &str) -> Vec<String> {
  let file = SourceFile::new("test.px", src);
  let mut bag = DiagnosticBag::default();

  lex(&file, &mut bag)
    .comments
    .iter()
    .map(|c| format!("{:?}@{}..{}", c.kind, c.span.start, c.span.end))
    .collect()
}

fn diagnostics(src: &str) -> Vec<Diagnostic> {
  let file = SourceFile::new("test.px", src);
  let mut bag = DiagnosticBag::default();

  lex(&file, &mut bag);

  bag.into_sorted()
}

fn codes(src: &str) -> Vec<String> {
  diagnostics(src)
    .iter()
    .map(|d| {
      format!("{}@{}..{}", d.code, d.primary.span.start, d.primary.span.end)
    })
    .collect()
}

fn values(src: &str) -> Vec<String> {
  let file = SourceFile::new("test.px", src);
  let mut bag = DiagnosticBag::default();

  lex(&file, &mut bag)
    .tokens
    .iter()
    .filter(|t| t.kind == TokenKind::StringPart)
    .filter_map(|t| t.value.clone())
    .collect()
}

#[track_caller]
fn check(src: &str, expected: &[&str]) {
  let mut got = tokens(src);

  if !expected.iter().any(|e| e.contains("Eof"))
    && got.last().is_some_and(|t| t.contains("Eof"))
  {
    got.pop();
  }

  let got: Vec<&str> = got.iter().map(String::as_str).collect();

  assert_eq!(got, expected, "source: {src:?}");
}

#[track_caller]
fn check_codes(src: &str, expected: &[&str]) {
  let got = codes(src);
  let got: Vec<&str> = got.iter().map(String::as_str).collect();

  assert_eq!(got, expected, "source: {src:?}");
}

mod token {
  use super::*;

  #[test]
  fn keyword_function() {
    assert_eq!(keyword("function"), Some(TokenKind::KwFunction));
  }

  #[test]
  fn keyword_as() {
    assert_eq!(keyword("as"), Some(TokenKind::KwAs));
  }

  #[test]
  fn keyword_export_is_reserved() {
    assert_eq!(keyword("export"), Some(TokenKind::KwExport));
  }

  #[test]
  fn keyword_prefix_is_not_a_keyword() {
    assert_eq!(keyword("functionx"), None);
  }

  #[test]
  fn zone_keywords_are_distinct() {
    assert_eq!(keyword("function"), Some(TokenKind::KwFunction));
    assert_eq!(keyword("functions"), Some(TokenKind::KwFunctions));
    assert_eq!(keyword("constants"), Some(TokenKind::KwConstants));
  }

  #[test]
  fn keyword_is_case_sensitive() {
    assert_eq!(keyword("Function"), None);
  }

  #[test]
  fn underscore_is_not_a_keyword() {
    assert_eq!(keyword("_"), None);
  }

  #[test]
  fn display_operator() {
    assert_eq!(TokenKind::Arrow.display_name(), "`->`");
  }

  #[test]
  fn display_keyword() {
    assert_eq!(TokenKind::KwFunction.display_name(), "keyword `function`");
  }

  #[test]
  fn display_lower() {
    assert_eq!(TokenKind::Lower.display_name(), "identifier");
  }

  #[test]
  fn display_upper() {
    assert_eq!(TokenKind::Upper.display_name(), "type or constructor name");
  }

  #[test]
  fn display_underscore() {
    assert_eq!(TokenKind::Underscore.display_name(), "`_`");
  }

  #[test]
  fn display_eof() {
    assert_eq!(TokenKind::Eof.display_name(), "end of file");
  }

  #[test]
  fn every_kind_has_a_display_name() {
    for kind in TokenKind::ALL {
      assert!(!kind.display_name().is_empty());
    }
  }

  #[test]
  fn all_has_no_duplicates() {
    let set: HashSet<_> = TokenKind::ALL.iter().collect();
    assert_eq!(set.len(), TokenKind::ALL.len());
  }
}

mod scanner {
  use super::*;

  #[test]
  fn fn_main() {
    check(
      "function main() {}",
      &[
        "KwFunction@0..8",
        "Lower@9..13",
        "LParen@13..14",
        "RParen@14..15",
        "LBrace@16..17",
        "RBrace@17..18",
        "Eof@18..18",
      ],
    );
  }

  #[test]
  fn identifier_kinds() {
    check(
      "Post post _x _",
      &["Upper@0..4", "Lower@5..9", "Lower@10..12", "Underscore@13..14"],
    );
  }

  #[test]
  fn keyword_vs_prefix() {
    check("function functionx", &["KwFunction@0..8", "Lower@9..18"]);
  }

  #[test]
  fn empty_file() {
    check("", &["Eof@0..0"]);
    assert!(comments("").is_empty());
    assert!(codes("").is_empty());
  }

  #[test]
  fn same_line_clears_flag() {
    check("a b", &["Lower@0..1", "Lower@2..3"]);
  }

  #[test]
  fn lf_sets_flag() {
    check("a\nb", &["Lower@0..1", "↵Lower@2..3"]);
  }

  #[test]
  fn crlf_sets_flag() {
    check("a\r\nb", &["Lower@0..1", "↵Lower@3..4"]);
  }

  #[test]
  fn eof_carries_flag() {
    check("a\n", &["Lower@0..1", "↵Eof@2..2"]);
  }

  #[test]
  fn comment_then_newline_sets_flag() {
    check("a // c\nb", &["Lower@0..1", "↵Lower@7..8"]);
    assert_eq!(comments("a // c\nb"), ["Line@2..6"]);
  }

  #[test]
  fn line_comment() {
    check("// hi", &["Eof@5..5"]);
    assert_eq!(comments("// hi"), ["Line@0..5"]);
  }

  #[test]
  fn doc_comment() {
    assert_eq!(comments("/// hi"), ["Doc@0..6"]);
  }

  #[test]
  fn four_slashes_is_a_line_comment() {
    assert_eq!(comments("//// hi"), ["Line@0..7"]);
  }

  #[test]
  fn bare_slashes() {
    assert_eq!(comments("//"), ["Line@0..2"]);
    assert_eq!(comments("///"), ["Doc@0..3"]);
  }

  #[test]
  fn comments_stay_out_of_the_stream() {
    check("a // c", &["Lower@0..1", "Eof@6..6"]);
  }

  #[test]
  fn int() {
    check("42", &["Int@0..2"]);
  }

  #[test]
  fn int_with_separators() {
    check("1_000", &["Int@0..5"]);
    assert!(codes("1_000").is_empty());
  }

  #[test]
  fn float() {
    check("1.5", &["Float@0..3"]);
  }

  #[test]
  fn float_negative_exponent() {
    check("1.5e-3", &["Float@0..6"]);
  }

  #[test]
  fn float_positive_exponent() {
    check("2.0E+10", &["Float@0..7"]);
  }

  #[test]
  fn range_is_not_a_float() {
    check("1..2", &["Int@0..1", "DotDot@1..3", "Int@3..4"]);
  }

  #[test]
  fn dot_after_identifier() {
    check("x.0", &["Lower@0..1", "Dot@1..2", "Int@2..3"]);
  }

  #[test]
  fn field_access() {
    check("a.b", &["Lower@0..1", "Dot@1..2", "Lower@2..3"]);
  }

  #[test]
  fn double_separator() {
    check("1__0", &["Int@0..4"]);
    check_codes("1__0", &["POLAR0105@0..4"]);
  }

  #[test]
  fn trailing_separator() {
    check("1_", &["Int@0..2"]);
    check_codes("1_", &["POLAR0105@0..2"]);
  }

  #[test]
  fn separator_before_point() {
    check("1_.5", &["Float@0..4"]);
    check_codes("1_.5", &["POLAR0105@0..4"]);
  }

  #[test]
  fn hex_is_not_supported() {
    check("0x1F", &["Int@0..1", "Lower@1..4"]);
    assert!(codes("0x1F").is_empty());
  }

  #[test]
  fn arrow() {
    check("x->y", &["Lower@0..1", "Arrow@1..3", "Lower@3..4"]);
  }

  #[test]
  fn pipe_operator() {
    check("a|>b", &["Lower@0..1", "PipeOp@1..3", "Lower@3..4"]);
  }

  #[test]
  fn or_or() {
    check("a||b", &["Lower@0..1", "OrOr@1..3", "Lower@3..4"]);
  }

  #[test]
  fn bar_alone() {
    check("a|b", &["Lower@0..1", "Bar@1..2", "Lower@2..3"]);
  }

  #[test]
  fn eq_eq_vs_two_eq() {
    check("==", &["EqEq@0..2"]);
    check("= =", &["Eq@0..1", "Eq@2..3"]);
  }

  #[test]
  fn comparisons() {
    check(
      "<= >= != <>",
      &["Le@0..2", "Ge@3..5", "BangEq@6..8", "Lt@9..10", "Gt@10..11"],
    );
  }

  #[test]
  fn dot_dot() {
    check("..", &["DotDot@0..2"]);
  }

  #[test]
  fn slash_is_one_token() {
    check("a / b", &["Lower@0..1", "Slash@2..3", "Lower@4..5"]);
  }

  #[test]
  fn single_unexpected() {
    check("$", &["Eof@1..1"]);
    check_codes("$", &["POLAR0101@0..1"]);
  }

  #[test]
  fn run_is_one_diagnostic() {
    check("$$$", &["Eof@3..3"]);
    check_codes("$$$", &["POLAR0101@0..3"]);
  }

  #[test]
  fn emoji_spans_the_whole_character() {
    check_codes("😀", &["POLAR0101@0..4"]);
  }

  #[test]
  fn nbsp_is_not_whitespace() {
    check("a\u{a0}b", &["Lower@0..1", "Lower@3..4"]);
    check_codes("a\u{a0}b", &["POLAR0101@1..3"]);
  }
}

mod strings {
  use super::*;

  #[test]
  fn plain_string() {
    check(
      r#""a b""#,
      &["StringStart@0..1", "StringPart@1..4", "StringEnd@4..5"],
    );
    assert_eq!(values(r#""a b""#), ["a b"]);
  }

  #[test]
  fn empty_string() {
    check(r#""""#, &["StringStart@0..1", "StringEnd@1..2"]);
  }

  #[test]
  fn one_interpolation() {
    check(
      r#""a #{x} b""#,
      &[
        "StringStart@0..1",
        "StringPart@1..3",
        "InterpStart@3..5",
        "Lower@5..6",
        "InterpEnd@6..7",
        "StringPart@7..9",
        "StringEnd@9..10",
      ],
    );
  }

  #[test]
  fn only_interpolation() {
    check(
      r##""#{x}""##,
      &[
        "StringStart@0..1",
        "InterpStart@1..3",
        "Lower@3..4",
        "InterpEnd@4..5",
        "StringEnd@5..6",
      ],
    );
  }

  #[test]
  fn adjacent_interpolations() {
    check(
      r##""#{a}#{b}""##,
      &[
        "StringStart@0..1",
        "InterpStart@1..3",
        "Lower@3..4",
        "InterpEnd@4..5",
        "InterpStart@5..7",
        "Lower@7..8",
        "InterpEnd@8..9",
        "StringEnd@9..10",
      ],
    );
  }

  #[test]
  fn nested_string_in_interpolation() {
    check(
      r#""a #{f("b #{c}")} d""#,
      &[
        "StringStart@0..1",
        "StringPart@1..3",
        "InterpStart@3..5",
        "Lower@5..6",
        "LParen@6..7",
        "StringStart@7..8",
        "StringPart@8..10",
        "InterpStart@10..12",
        "Lower@12..13",
        "InterpEnd@13..14",
        "StringEnd@14..15",
        "RParen@15..16",
        "InterpEnd@16..17",
        "StringPart@17..19",
        "StringEnd@19..20",
      ],
    );
  }

  #[test]
  fn record_literal_in_interpolation() {
    check(
      r##""#{ { x: 1 }.x }""##,
      &[
        "StringStart@0..1",
        "InterpStart@1..3",
        "LBrace@4..5",
        "Lower@6..7",
        "Colon@7..8",
        "Int@9..10",
        "RBrace@11..12",
        "Dot@12..13",
        "Lower@13..14",
        "InterpEnd@15..16",
        "StringEnd@16..17",
      ],
    );
  }

  #[test]
  fn escape_newline() {
    check(
      r#""\n""#,
      &["StringStart@0..1", "StringPart@1..3", "StringEnd@3..4"],
    );
    assert_eq!(values(r#""\n""#), ["\n"]);
  }

  #[test]
  fn escape_quote_and_backslash() {
    assert_eq!(values("\"\\\"\\\\\""), ["\"\\"]);
  }

  #[test]
  fn escape_unicode() {
    check(
      r#""\u{e9}""#,
      &["StringStart@0..1", "StringPart@1..7", "StringEnd@7..8"],
    );
    assert_eq!(values(r#""\u{e9}""#), ["é"]);
  }

  #[test]
  fn escape_hash_is_literal() {
    check(
      r#""\#{""#,
      &["StringStart@0..1", "StringPart@1..4", "StringEnd@4..5"],
    );
    assert_eq!(values(r#""\#{""#), ["#{"]);
  }

  #[test]
  fn lone_hash_is_text() {
    check(
      r#""a # b""#,
      &["StringStart@0..1", "StringPart@1..6", "StringEnd@6..7"],
    );
    assert_eq!(values(r#""a # b""#), ["a # b"]);
  }

  #[test]
  fn unknown_escape() {
    assert_eq!(values(r#""\q""#), ["q"]);
    check_codes(r#""\q""#, &["POLAR0103@1..3"]);
  }

  #[test]
  fn empty_unicode_escape() {
    check_codes(r#""\u{}""#, &["POLAR0103@1..5"]);
  }

  #[test]
  fn too_many_hex_digits() {
    check_codes(r#""\u{1234567}""#, &["POLAR0103@1..12"]);
  }

  #[test]
  fn above_max_code_point() {
    check_codes(r#""\u{110000}""#, &["POLAR0103@1..11"]);
  }

  #[test]
  fn surrogate() {
    check_codes(r#""\u{D800}""#, &["POLAR0103@1..9"]);
  }

  #[test]
  fn non_ascii_needs_no_escape() {
    check(
      "\"héllo 😀\"",
      &["StringStart@0..1", "StringPart@1..12", "StringEnd@12..13"],
    );
    assert!(codes("\"héllo 😀\"").is_empty());
  }

  #[test]
  fn unterminated_string() {
    check(
      "\"abc\nx",
      &["StringStart@0..1", "StringPart@1..4", "StringEnd@4..4", "↵Lower@5..6"],
    );
    check_codes("\"abc\nx", &["POLAR0102@0..4"]);
  }

  #[test]
  fn next_line_still_lexes() {
    assert_eq!(codes("\"abc\nx").len(), 1);
    assert!(tokens("\"abc\nx").contains(&"↵Lower@5..6".to_string()));
  }

  #[test]
  fn unterminated_string_at_eof() {
    check(
      "\"abc",
      &["StringStart@0..1", "StringPart@1..4", "StringEnd@4..4", "Eof@4..4"],
    );
    check_codes("\"abc", &["POLAR0102@0..4"]);
  }

  #[test]
  fn unterminated_interpolation_at_eol() {
    check(
      "\"a #{ b\nx",
      &[
        "StringStart@0..1",
        "StringPart@1..3",
        "InterpStart@3..5",
        "Lower@6..7",
        "InterpEnd@7..7",
        "StringEnd@7..7",
        "↵Lower@8..9",
      ],
    );
    check_codes("\"a #{ b\nx", &["POLAR0104@3..7"]);
  }

  #[test]
  fn unterminated_interpolation_at_eof() {
    check(
      "\"a #{ b",
      &[
        "StringStart@0..1",
        "StringPart@1..3",
        "InterpStart@3..5",
        "Lower@6..7",
        "InterpEnd@7..7",
        "StringEnd@7..7",
        "Eof@7..7",
      ],
    );
    check_codes("\"a #{ b", &["POLAR0104@3..7"]);
  }

  #[test]
  fn unterminated_nested_string() {
    let src = "\"a #{f(\"b\n";

    check_codes(src, &["POLAR0102@7..9"]);
    assert_eq!(
      tokens(src).iter().filter(|t| t.contains("@9..9")).count(),
      3,
      "all three frames close with empty spans at end of line"
    );
  }

  #[test]
  fn recovery_note() {
    for src in ["\"abc\nx", "\"a #{ b\nx"] {
      let d = &diagnostics(src)[0];

      assert!(
        d.notes.iter().any(|n| n == "strings cannot span multiple lines"),
        "missing note for {src:?}"
      );
    }
  }
}

mod messages {
  use super::*;

  #[track_caller]
  fn help(src: &str) -> String {
    diagnostics(src)[0].help.clone().unwrap_or_default()
  }

  #[test]
  fn semicolon_help() {
    check_codes("a;", &["POLAR0101@1..2"]);
    assert_eq!(help("a;"), "Polar does not use semicolons; remove it");
  }

  #[test]
  fn bitwise_tokens() {
    check(
      "a & b | c ^ ~d",
      &[
        "Lower@0..1",
        "Amp@2..3",
        "Lower@4..5",
        "Bar@6..7",
        "Lower@8..9",
        "Caret@10..11",
        "Tilde@12..13",
        "Lower@13..14",
      ],
    );
    assert!(codes("a & b | c ^ ~d").is_empty());
  }

  #[test]
  fn and_and_is_a_token() {
    check("a && b", &["Lower@0..1", "AndAnd@2..4", "Lower@5..6"]);
    assert!(codes("a && b").is_empty());
  }

  #[test]
  fn single_quote_help() {
    assert_eq!(help("'a'"), "strings use double quotes");
  }

  #[test]
  fn message_shape() {
    let d = &diagnostics("$")[0];

    assert_eq!(d.message, "unexpected character `$`");
    assert_eq!(d.primary.message.as_deref(), Some("not valid in Polar source"));
  }
}

mod properties {
  use super::*;

  const CORPUS: &[&str] = &[
    "",
    "function main() {}",
    "Post post _x _",
    "a // c\nb",
    "/// doc\nfunction f() {}",
    "1_000 1.5e-3 1..2 0x1F 1__0",
    "<= >= != <> -> |> || | && ..",
    r#""a #{f("b #{c}")} d""#,
    r##""#{ { x: 1 }.x }""##,
    r#""\n\t\\\"\#\u{e9}\q""#,
    "\"abc\nx",
    "\"a #{ b\nx",
    "\"a #{f(\"b\n",
    "\"abc",
    "$$$ 😀 a\u{a0}b ; & '",
    "a\r\nb\rc\nd",
  ];

  fn each_file(f: impl Fn(&SourceFile, &LexResult)) {
    for src in CORPUS {
      let file = SourceFile::new("test.px", *src);
      let mut bag = DiagnosticBag::default();
      let result = lex(&file, &mut bag);

      f(&file, &result);
    }
  }

  #[test]
  fn tokens_are_ordered_and_non_overlapping() {
    each_file(|_, result| {
      for pair in result.tokens.windows(2) {
        assert!(
          pair[0].span.end <= pair[1].span.start,
          "overlap: {:?} then {:?}",
          pair[0],
          pair[1]
        );
      }
    });
  }

  #[test]
  fn spans_cover_exactly_their_text() {
    each_file(|file, result| {
      for token in &result.tokens {
        if token.kind == TokenKind::Eof || token.span.start == token.span.end {
          continue;
        }

        let text = file.slice(&token.span);

        assert!(!text.is_empty(), "empty span for {:?}", token.kind);

        if token.kind != TokenKind::StringPart {
          assert_eq!(text, text.trim(), "untrimmed span for {:?}", token.kind);
        }
      }
    });
  }

  #[test]
  fn value_is_present_exactly_on_string_parts() {
    each_file(|_, result| {
      for token in &result.tokens {
        assert_eq!(
          token.value.is_some(),
          token.kind == TokenKind::StringPart,
          "value/kind mismatch on {:?}",
          token.kind
        );
      }
    });
  }

  #[test]
  fn string_and_interpolation_tokens_are_balanced() {
    each_file(|_, result| {
      let mut strings = 0i32;
      let mut interps = 0i32;

      for token in &result.tokens {
        match token.kind {
          TokenKind::StringStart => strings += 1,
          TokenKind::StringEnd => strings -= 1,
          TokenKind::InterpStart => interps += 1,
          TokenKind::InterpEnd => interps -= 1,
          _ => {}
        }

        assert!(strings >= 0 && interps >= 0, "closer before opener");
      }

      assert_eq!(strings, 0, "unbalanced strings");
      assert_eq!(interps, 0, "unbalanced interpolations");
    });
  }

  #[test]
  fn lexing_never_panics() {
    for src in CORPUS {
      for (at, _) in src.char_indices().chain([(src.len(), ' ')]) {
        let file = SourceFile::new("test.px", &src[..at]);
        let mut bag = DiagnosticBag::default();
        let result = lex(&file, &mut bag);

        assert_eq!(
          result.tokens.last().map(|t| t.kind),
          Some(TokenKind::Eof),
          "stream must end with Eof: {:?}",
          &src[..at]
        );
      }
    }
  }
}
