use polar_compiler::{
  shared::diagnostic::DiagnosticBag,
  shared::source::SourceFile,
  syntax::lexer::{lex, token::TokenKind},
  syntax::parser::{MAX_DEPTH, Parser, parse},
};

mod common;

use common::{codes, drive, expect_ice};

mod cursor {
  use super::*;

  #[test]
  fn peek_clamps_to_eof() {
    drive("a b", |parser| {
      assert_eq!(parser.peek(0).kind, TokenKind::Lower);
      assert_eq!(parser.peek(1).kind, TokenKind::Lower);
      assert_eq!(parser.peek(2).kind, TokenKind::Eof);
      assert_eq!(parser.peek(100).kind, TokenKind::Eof);
    });
  }

  #[test]
  fn bump_stops_at_eof() {
    drive("a b", |parser| {
      for _ in 0..10 {
        parser.bump();
      }

      assert_eq!(parser.peek(0).kind, TokenKind::Eof);
      assert_eq!(parser.bump().kind, TokenKind::Eof);
    });
  }

  #[test]
  fn at_and_eat() {
    drive("functions\n  f() {}", |parser| {
      assert!(parser.at(TokenKind::KwFunctions));
      assert!(!parser.at(TokenKind::KwLet));

      assert!(parser.eat(TokenKind::KwLet).is_none());
      assert!(parser.at(TokenKind::KwFunctions));

      assert_eq!(
        parser.eat(TokenKind::KwFunctions).map(|t| t.kind),
        Some(TokenKind::KwFunctions)
      );
      assert!(parser.at(TokenKind::Lower));
    });
  }
}

mod expect {
  use super::*;

  fn at_the_equals(parser: &mut Parser) {
    while !parser.at(TokenKind::Eq) && !parser.at(TokenKind::Eof) {
      parser.bump();
    }
  }

  #[test]
  fn expect_message() {
    let diagnostics = drive("functions\n  f(a = 1) {}", |parser| {
      at_the_equals(parser);

      assert!(
        parser.expect(TokenKind::Colon, "after parameter name").is_none()
      );
    });

    assert_eq!(diagnostics.len(), 1);
    assert_eq!(
      diagnostics[0].message,
      "expected `:` after parameter name, found `=`"
    );
    assert_eq!(codes(&diagnostics), ["POLAR0201@16..17"]);
  }

  #[test]
  fn expect_does_not_consume() {
    drive("functions\n  f(a = 1) {}", |parser| {
      at_the_equals(parser);
      parser.expect(TokenKind::Colon, "after parameter name");

      assert_eq!(parser.peek(0).kind, TokenKind::Eq);
    });
  }

  #[test]
  fn expect_at_eof() {
    let src = "functions\n  f(";
    let diagnostics = drive(src, |parser| {
      while !parser.at(TokenKind::Eof) {
        parser.bump();
      }

      parser.expect(TokenKind::RParen, "to close the parameter list");
    });

    assert_eq!(diagnostics.len(), 1);
    assert_eq!(
      diagnostics[0].message,
      "expected `)` to close the parameter list, found end of file"
    );
    assert_eq!(
      (diagnostics[0].primary.span.start, diagnostics[0].primary.span.end),
      (src.len(), src.len())
    );
  }

  #[test]
  fn cascade_is_suppressed() {
    let diagnostics = drive("functions\n  f(a: , b: , c: ) {}", |parser| {
      while !parser.at(TokenKind::LParen) {
        parser.bump();
      }

      parser.bump();

      while !parser.at(TokenKind::RParen) && !parser.at(TokenKind::Eof) {
        let before = parser.peek(0).span.start;

        parser.expect(TokenKind::Lower, "as a parameter name");
        parser.expect(TokenKind::Colon, "after parameter name");
        parser.expect(TokenKind::Upper, "as a parameter type");
        parser.eat(TokenKind::Comma);

        assert!(parser.peek(0).span.start > before, "loop made no progress");
      }
    });

    assert_eq!(
      codes(&diagnostics),
      ["POLAR0201@17..18", "POLAR0201@22..23", "POLAR0201@27..28"],
      "one diagnostic per malformed parameter, not one per token"
    );
  }

  #[test]
  fn recovering_clears_on_consume() {
    let diagnostics = drive("functions\n  f(a = 1) {}", |parser| {
      at_the_equals(parser);

      parser.expect(TokenKind::Colon, "after parameter name");
      parser.expect(TokenKind::Colon, "after parameter name");

      parser.bump();

      parser.expect(TokenKind::Colon, "after parameter name");
    });

    assert_eq!(diagnostics.len(), 2, "the consume between them re-arms");
    assert_eq!(codes(&diagnostics), ["POLAR0201@16..17", "POLAR0201@18..19"]);
  }
}

mod guards {
  use super::*;

  #[test]
  fn progress_guard_ices() {
    let ice = expect_ice(|| {
      drive("functions\n  f() {}", |parser| {
        let before = 0;

        parser.progress(before);
      });
    });

    assert!(
      ice.message.contains("no progress"),
      "unexpected ICE message: {}",
      ice.message
    );
  }

  fn nest(parser: &mut Parser, depth: u32) -> bool {
    if depth == 0 {
      return true;
    }

    parser.nested(|p| nest(p, depth - 1)).unwrap_or(false)
  }

  #[test]
  fn depth_guard_rejects() {
    let diagnostics = drive("types\n  Id = Int\n", |parser| {
      assert!(!nest(parser, MAX_DEPTH + 1));
    });

    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].message, "nesting too deep");
    assert_eq!(codes(&diagnostics), ["POLAR0210@0..5"]);
  }

  #[test]
  fn depth_guard_allows_the_limit() {
    let diagnostics = drive("types\n  Id = Int\n", |parser| {
      assert!(nest(parser, MAX_DEPTH));
    });

    assert!(diagnostics.is_empty());
  }
}

mod spans {
  use super::*;

  #[test]
  fn span_from_ends_at_last_consumed() {
    drive("types\n  Id = Int\n", |parser| {
      parser.bump();

      let start = parser.peek(0).span.clone();

      parser.bump();
      parser.bump();
      parser.bump();

      let span = parser.span_from(&start);

      assert_eq!((span.start, span.end), (8, 16), "the newline is not covered");
    });
  }

  #[test]
  fn span_excludes_trailing_trivia() {
    drive("functions\n  f() {}   // c\n", |parser| {
      parser.bump();

      let start = parser.peek(0).span.clone();

      while !parser.at(TokenKind::RBrace) {
        parser.bump();
      }

      parser.bump();

      let span = parser.span_from(&start);

      assert_eq!(
        (span.start, span.end),
        (12, 18),
        "the span ends at the closing brace, not the comment"
      );
    });
  }
}

mod module {
  use super::*;

  #[test]
  fn empty_input_yields_a_module() {
    let file = SourceFile::new("test.px", "");
    let mut bag = DiagnosticBag::default();
    let lexed = lex(&file, &mut bag);
    let module = parse(&file, &lexed, &mut bag);

    assert_eq!((module.span.start, module.span.end), (0, 0));
    assert_eq!(module.name, None);
    assert!(module.zones.is_empty());
    assert!(bag.into_sorted().is_empty());
  }
}
