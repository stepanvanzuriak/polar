use crate::{
  shared::codes::DiagnosticCode::{
    self, MalformedNumber, UnexpectedCharacter, UnterminatedInterpolation,
    UnterminatedString,
  },
  shared::diagnostic::{Diagnostic, DiagnosticBag, Label},
  shared::source::{SourceFile, Span},
  syntax::ast::Builtin,
  syntax::lexer::token::{
    Comment, CommentKind, Token,
    TokenKind::{
      self, Amp, AndAnd, Arrow, Bang, BangEq, Bar, Caret, Colon, Comma, Dot,
      DotDot, Eq, EqEq, Float, Ge, Gt, Int, InterpEnd, InterpStart, LBrace,
      LBracket, LParen, Le, Lower, Lt, Minus, OrOr, Percent, PipeOp, Plus,
      RBrace, RBracket, RParen, Slash, Star, StringEnd, StringPart,
      StringStart, Tilde, Underscore, Unknown, Upper,
    },
    keyword,
  },
  syntax::plugins,
};
use std::{mem::take, sync::Arc};

const ESCAPE_HELP: &str =
  r#"valid escapes are `\n` `\r` `\t` `\\` `\"` `\#` and `\u{…}`"#;
const UNICODE_HELP: &str = "unicode escapes take 1–6 hex digits up to `10FFFF`";
const LINE_NOTE: &str = "strings cannot span multiple lines";

pub enum Frame {
  Str { open: usize },
  Interp { open: usize, brace_depth: u32 },
}

pub struct Cursor<'a> {
  text: &'a str,
  name: Arc<str>,
  pub pos: usize,
  newline_pending: bool,
  tokens: Vec<Token>,
  comments: Vec<Comment>,
  plugin_keywords: Vec<&'static str>,
  in_plugin_zone: bool,
}

impl<'a> Cursor<'a> {
  pub fn new(file: &'a SourceFile) -> Self {
    Self {
      text: file.text(),
      name: Arc::from(file.name()),
      pos: 0,
      newline_pending: false,
      tokens: vec![],
      comments: vec![],
      plugin_keywords: plugins::enabled()
        .into_iter()
        .map(plugins::keyword)
        .collect(),
      in_plugin_zone: false,
    }
  }

  #[must_use]
  pub fn into_parts(self) -> (Vec<Token>, Vec<Comment>) {
    (self.tokens, self.comments)
  }

  fn rest(&self) -> &str {
    &self.text[self.pos..]
  }

  fn advance(&mut self) {
    if let Some(ch) = self.peek() {
      self.pos += ch.len_utf8();
    }
  }

  fn eat(&mut self, c: char) -> bool {
    let found = self.peek() == Some(c);

    if found {
      self.advance();
    }

    found
  }

  fn eat_while(&mut self, f: impl Fn(char) -> bool) {
    while self.peek().is_some_and(&f) {
      self.advance();
    }
  }

  fn span(&self, start: usize) -> Span {
    Span::new(self.name.clone(), start, self.pos)
  }

  fn bump(&mut self) -> Option<char> {
    let ch = self.peek()?;
    self.pos += ch.len_utf8();

    Some(ch)
  }

  fn peek(&self) -> Option<char> {
    self.rest().chars().next()
  }

  fn peek_nth(&self, n: usize) -> Option<char> {
    self.rest().chars().nth(n)
  }

  fn peek2(&self) -> Option<char> {
    self.peek_nth(1)
  }

  #[must_use]
  pub fn at_eof(&self) -> bool {
    self.pos >= self.text.len()
  }

  pub fn push(&mut self, kind: TokenKind, start: usize) {
    self.tokens.push(Token {
      kind,
      span: self.span(start),
      newline_before: take(&mut self.newline_pending),
      value: None,
    });
  }
}

pub fn scan_code_token(
  cursor: &mut Cursor,
  stack: &mut Vec<Frame>,
  diagnostics: &mut DiagnosticBag,
) {
  let start = cursor.pos;
  let Some(ch) = cursor.peek() else { return };

  match ch {
    '\n' | '\r' if !stack.is_empty() => recover(cursor, stack, diagnostics),
    ' ' | '\t' => cursor.advance(),
    '\n' => {
      cursor.advance();
      cursor.newline_pending = true;
    }
    '\r' => {
      cursor.advance();
      cursor.eat('\n');
      cursor.newline_pending = true;
    }
    '/' if cursor.peek2() == Some('/') => scan_comment(cursor, start),
    'a'..='z' | '_' => scan_lower_or_keyword(cursor, start),
    'A'..='Z' => {
      cursor.eat_while(is_ident_continue);
      cursor.push(Upper, start);
    }
    '0'..='9' => scan_number(cursor, start, diagnostics),
    '"' => {
      cursor.advance();
      cursor.push(StringStart, start);
      stack.push(Frame::Str { open: start });
    }
    '{' => {
      cursor.advance();
      cursor.push(LBrace, start);

      if let Some(Frame::Interp { brace_depth, .. }) = stack.last_mut() {
        *brace_depth += 1;
      }
    }
    '}' => {
      let close =
        matches!(stack.last(), Some(Frame::Interp { brace_depth: 0, .. }));
      cursor.advance();

      if close {
        cursor.push(InterpEnd, start);
        stack.pop();
      } else {
        if let Some(Frame::Interp { brace_depth, .. }) = stack.last_mut() {
          *brace_depth -= 1;
        }

        cursor.push(RBrace, start);
      }
    }
    _ => scan_operator_or_unexpected(cursor, start, diagnostics),
  }
}

fn scan_comment(cursor: &mut Cursor, start: usize) {
  cursor.eat_while(|ch| ch == '/');

  let slashes = cursor.pos - start;

  cursor.eat_while(|ch| ch != '\n' && ch != '\r');

  let kind = if slashes == 3 { CommentKind::Doc } else { CommentKind::Line };

  cursor.comments.push(Comment { kind, span: cursor.span(start) });
}

fn scan_lower_or_keyword(cursor: &mut Cursor, start: usize) {
  cursor.eat_while(is_ident_continue);

  let text = &cursor.text[start..cursor.pos];
  let kind =
    if text == "_" { Underscore } else { keyword(text).unwrap_or(Lower) };

  if start == 0 || cursor.text[..start].ends_with('\n') {
    if Builtin::ALL.iter().any(|b| b.as_str() == text) {
      cursor.in_plugin_zone = false;
    } else if kind == Lower && cursor.plugin_keywords.contains(&text) {
      cursor.in_plugin_zone = true;
    }
  }

  cursor.push(kind, start);
}

fn scan_number(
  cursor: &mut Cursor,
  start: usize,
  diagnostics: &mut DiagnosticBag,
) {
  let text = cursor.text;
  let mut kind = Int;
  let mut groups = vec![];

  cursor.eat_while(is_digit_or_separator);
  groups.push((start, cursor.pos));

  if cursor.peek() == Some('.')
    && cursor.peek2().is_some_and(|ch| ch.is_ascii_digit())
  {
    cursor.advance();

    let frac = cursor.pos;

    cursor.eat_while(is_digit_or_separator);
    groups.push((frac, cursor.pos));
    kind = Float;
  }

  if matches!(cursor.peek(), Some('e' | 'E')) {
    let signed = matches!(cursor.peek2(), Some('+' | '-'));
    let first = if signed { cursor.peek_nth(2) } else { cursor.peek2() };

    if first.is_some_and(|ch| ch.is_ascii_digit()) {
      cursor.advance();

      if signed {
        cursor.advance();
      }

      let exp = cursor.pos;

      cursor.eat_while(is_digit_or_separator);
      groups.push((exp, cursor.pos));
      kind = Float;
    }
  }

  if groups.iter().any(|&(a, b)| has_bad_separator(&text[a..b])) {
    diagnostics.push(
      Diagnostic::error(
        MalformedNumber,
        "malformed number literal",
        Label::new(cursor.span(start))
          .with_message("`_` may not start, end, or repeat"),
      )
      .with_help("write digit separators as `1_000`"),
    );
  }

  cursor.push(kind, start);
}

fn scan_operator_or_unexpected(
  cursor: &mut Cursor,
  start: usize,
  diagnostics: &mut DiagnosticBag,
) {
  let kind = match cursor.bump() {
    Some('-') => {
      if cursor.eat('>') {
        Arrow
      } else {
        Minus
      }
    }
    Some('|') => {
      if cursor.eat('>') {
        PipeOp
      } else if cursor.eat('|') {
        OrOr
      } else {
        Bar
      }
    }
    Some('=') => {
      if cursor.eat('=') {
        EqEq
      } else {
        Eq
      }
    }
    Some('!') => {
      if cursor.eat('=') {
        BangEq
      } else {
        Bang
      }
    }
    Some('<') => {
      if cursor.eat('=') {
        Le
      } else {
        Lt
      }
    }
    Some('>') => {
      if cursor.eat('=') {
        Ge
      } else {
        Gt
      }
    }
    Some('.') => {
      if cursor.eat('.') {
        DotDot
      } else {
        Dot
      }
    }
    Some('&') => {
      if cursor.eat('&') {
        AndAnd
      } else {
        Amp
      }
    }
    Some('^') => Caret,
    Some('~') => Tilde,
    Some('(') => LParen,
    Some(')') => RParen,
    Some('[') => LBracket,
    Some(']') => RBracket,
    Some(',') => Comma,
    Some(':') => Colon,
    Some('+') => Plus,
    Some('*') => Star,
    Some('/') => Slash,
    Some('%') => Percent,
    _ => return unexpected(cursor, start, diagnostics),
  };

  cursor.push(kind, start);
}

fn unexpected(
  cursor: &mut Cursor,
  start: usize,
  diagnostics: &mut DiagnosticBag,
) {
  cursor.pos = start;
  cursor.advance();

  while cursor
    .peek()
    .is_some_and(|ch| !is_known_start(ch) && !is_skipped_whitespace(ch))
  {
    cursor.advance();
  }

  if cursor.in_plugin_zone {
    cursor.push(Unknown, start);

    return;
  }

  let span = cursor.span(start);
  let Some(first) = cursor.text[start..].chars().next() else { return };
  let mut d = Diagnostic::error(
    UnexpectedCharacter,
    format!("unexpected character `{first}`"),
    Label::new(span).with_message("not valid in Polar source"),
  );

  if let Some(h) = help_for(first) {
    d = d.with_help(h);
  }

  diagnostics.push(d);
}

pub fn scan_string_piece(
  cursor: &mut Cursor,
  stack: &mut Vec<Frame>,
  diagnostics: &mut DiagnosticBag,
) {
  let start = cursor.pos;
  let mut value = String::new();

  loop {
    match cursor.peek() {
      None | Some('\n' | '\r') => {
        flush(cursor, start, value);
        recover(cursor, stack, diagnostics);

        return;
      }
      Some('"') => {
        flush(cursor, start, value);

        let quote = cursor.pos;

        cursor.advance();
        cursor.push(StringEnd, quote);
        stack.pop();

        return;
      }
      Some('#') if cursor.peek2() == Some('{') => {
        flush(cursor, start, value);

        let at = cursor.pos;

        cursor.advance();
        cursor.advance();
        cursor.push(InterpStart, at);
        stack.push(Frame::Interp { open: at, brace_depth: 0 });

        return;
      }
      Some('\\') => scan_escape(cursor, &mut value, diagnostics),
      Some(ch) => {
        cursor.advance();
        value.push(ch);
      }
    }
  }
}

fn scan_escape(
  cursor: &mut Cursor,
  value: &mut String,
  diagnostics: &mut DiagnosticBag,
) {
  let text = cursor.text;
  let start = cursor.pos;

  cursor.advance();

  match cursor.bump() {
    Some('n') => value.push('\n'),
    Some('r') => value.push('\r'),
    Some('t') => value.push('\t'),
    Some('\\') => value.push('\\'),
    Some('"') => value.push('"'),
    Some('#') => value.push('#'),
    Some('u') if cursor.peek() == Some('{') => {
      cursor.advance();

      let from = cursor.pos;

      cursor.eat_while(|ch| ch.is_ascii_hexdigit());

      let to = cursor.pos;
      let closed = cursor.eat('}');
      let digits = &text[from..to];
      let decoded =
        u32::from_str_radix(digits, 16).ok().and_then(char::from_u32);

      match (closed, digits.len(), decoded) {
        (true, 1..=6, Some(ch)) => value.push(ch),
        _ => diagnostics.push(invalid_escape(
          text,
          cursor.span(start),
          UNICODE_HELP,
        )),
      }
    }
    Some(ch) => {
      value.push(ch);
      diagnostics.push(invalid_escape(text, cursor.span(start), ESCAPE_HELP));
    }
    None => {}
  }
}

fn flush(cursor: &mut Cursor, start: usize, value: String) {
  if cursor.pos > start {
    cursor.tokens.push(Token {
      kind: StringPart,
      span: cursor.span(start),
      newline_before: take(&mut cursor.newline_pending),
      value: Some(value),
    });
  }
}

pub fn recover(
  cursor: &mut Cursor,
  stack: &mut Vec<Frame>,
  diagnostics: &mut DiagnosticBag,
) {
  let at = cursor.pos;
  let name = cursor.name.clone();

  match stack.last() {
    Some(Frame::Str { open }) => diagnostics.push(
      Diagnostic::error(
        UnterminatedString,
        "unterminated string",
        Label::new(Span::new(name.clone(), *open, at))
          .with_message("string starts here"),
      )
      .with_note(LINE_NOTE),
    ),
    Some(Frame::Interp { open, .. }) => diagnostics.push(
      Diagnostic::error(
        UnterminatedInterpolation,
        "unterminated interpolation",
        Label::new(Span::new(name.clone(), *open, at))
          .with_message("interpolation starts here"),
      )
      .with_note(LINE_NOTE),
    ),
    None => return,
  }

  while let Some(frame) = stack.pop() {
    let kind = match frame {
      Frame::Str { .. } => StringEnd,
      Frame::Interp { .. } => InterpEnd,
    };

    cursor.tokens.push(Token {
      kind,
      span: Span::empty(name.clone(), at),
      newline_before: false,
      value: None,
    });
  }
}

fn invalid_escape(text: &str, span: Span, help: &'static str) -> Diagnostic {
  let raw = &text[span.start..span.end];

  Diagnostic::error(
    DiagnosticCode::InvalidEscape,
    format!("invalid escape `{raw}`"),
    Label::new(span).with_message("not a valid escape sequence"),
  )
  .with_help(help)
}

fn is_ident_continue(ch: char) -> bool {
  ch.is_ascii_alphanumeric() || ch == '_'
}

fn is_digit_or_separator(ch: char) -> bool {
  ch.is_ascii_digit() || ch == '_'
}

fn has_bad_separator(s: &str) -> bool {
  s.starts_with('_') || s.ends_with('_') || s.contains("__")
}

fn is_known_start(ch: char) -> bool {
  ch.is_ascii_alphanumeric()
    || matches!(
      ch,
      '_'
        | '"'
        | '('
        | ')'
        | '['
        | ']'
        | '{'
        | '}'
        | ','
        | ':'
        | '.'
        | '+'
        | '-'
        | '*'
        | '/'
        | '%'
        | '='
        | '!'
        | '<'
        | '>'
        | '|'
        | '&'
        | '^'
        | '~'
    )
}

fn is_skipped_whitespace(ch: char) -> bool {
  matches!(ch, ' ' | '\t' | '\n' | '\r')
}

fn help_for(ch: char) -> Option<&'static str> {
  match ch {
    ';' => Some("Polar does not use semicolons; remove it"),
    '\'' => Some("strings use double quotes"),
    _ => None,
  }
}
