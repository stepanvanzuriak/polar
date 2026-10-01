use crate::{
  shared::codes::DiagnosticCode::{
    NestingTooDeep, PluginKeywordAsName, UnexpectedToken,
  },
  shared::diagnostic::{Diagnostic, DiagnosticBag, Label},
  shared::ice::invariant,
  shared::source::{SourceFile, Span},
  syntax::ast::{Module, Name, PluginId},
  syntax::lexer::{
    LexResult,
    token::{
      Comment, Token,
      TokenKind::{self, Eof, Lower, Upper},
    },
  },
  syntax::plugins,
};

pub mod decl;
pub mod expr;
pub mod pattern;
pub mod precedence;
pub mod types;

use precedence::PrecedenceTable;

pub const MAX_DEPTH: u32 = 256;

pub struct Parser<'a> {
  file: &'a SourceFile,
  tokens: &'a [Token],
  comments: &'a [Comment],
  pos: usize,
  diagnostics: &'a mut DiagnosticBag,
  recovering: bool,
  depth: u32,
  table: PrecedenceTable,
  no_record: bool,
  limit: Option<usize>,
  body_unclosed: bool,
  enabled: Vec<PluginId>,
}

#[derive(Debug, Clone, Default)]
pub struct ParseOptions {
  pub precedence: PrecedenceTable,
}

pub fn parse(
  file: &SourceFile,
  lexed: &LexResult,
  diagnostics: &mut DiagnosticBag,
) -> Module {
  parse_with(file, lexed, diagnostics, &ParseOptions::default())
}

pub fn parse_with(
  file: &SourceFile,
  lexed: &LexResult,
  diagnostics: &mut DiagnosticBag,
  options: &ParseOptions,
) -> Module {
  let mut parser =
    Parser::new(file, &lexed.tokens, &lexed.comments, diagnostics)
      .with_options(options);

  parser.module()
}

impl<'a> Parser<'a> {
  pub fn new(
    file: &'a SourceFile,
    tokens: &'a [Token],
    comments: &'a [Comment],
    diagnostics: &'a mut DiagnosticBag,
  ) -> Self {
    Self {
      file,
      tokens,
      comments,
      pos: 0,
      diagnostics,
      recovering: false,
      depth: 0,
      table: PrecedenceTable::default(),
      no_record: false,
      limit: None,
      body_unclosed: false,
      enabled: plugins::enabled(),
    }
  }

  #[must_use]
  pub fn with_options(mut self, options: &ParseOptions) -> Self {
    self.table = options.precedence.clone();
    self
  }

  #[must_use]
  pub fn peek(&self, n: usize) -> &Token {
    let eof = self.tokens.len() - 1;
    let idx = self.pos + n;

    match self.limit {
      Some(limit) if idx >= limit => &self.tokens[eof],
      _ => &self.tokens[idx.min(eof)],
    }
  }

  pub fn bump(&mut self) -> &Token {
    if self.at_limit() {
      return &self.tokens[self.tokens.len() - 1];
    }

    let idx = self.pos;

    if self.pos + 1 < self.tokens.len() {
      self.pos += 1;
      self.recovering = false;
    }

    &self.tokens[idx]
  }

  fn at_limit(&self) -> bool {
    self.limit.is_some_and(|limit| self.pos >= limit)
  }

  #[must_use]
  pub fn at(&self, kind: TokenKind) -> bool {
    self.peek(0).kind == kind
  }

  pub fn eat(&mut self, kind: TokenKind) -> Option<&Token> {
    if !self.at(kind) {
      return None;
    }

    Some(self.bump())
  }

  pub fn expect(&mut self, kind: TokenKind, context: &str) -> Option<&Token> {
    if self.at(kind) {
      return Some(self.bump());
    }

    self.expected(&format!("{} {context}", kind.display_name()));

    None
  }

  pub fn expected(&mut self, what: &str) -> Span {
    let span = self.error_span();

    if !self.recovering {
      let found = self.tokens[self.pos].kind;

      self.diagnostics.push(Diagnostic::error(
        UnexpectedToken,
        format!("expected {what}, found {}", found.display_name()),
        Label::new(span.clone()),
      ));

      self.recovering = true;
    }

    span
  }

  pub fn nested<T>(&mut self, f: impl FnOnce(&mut Self) -> T) -> Option<T> {
    if !self.deepen() {
      return None;
    }

    let result = f(self);

    self.depth -= 1;

    Some(result)
  }

  pub fn deepen(&mut self) -> bool {
    if self.depth >= MAX_DEPTH {
      let span = self.error_span();

      self.diagnostics.push(
        Diagnostic::error(NestingTooDeep, "nesting too deep", Label::new(span))
          .with_help(format!("Polar nests at most {MAX_DEPTH} levels deep")),
      );

      return false;
    }

    self.depth += 1;

    true
  }

  #[must_use]
  pub fn span_from(&self, start: &Span) -> Span {
    match self.pos.checked_sub(1) {
      Some(last) if self.tokens[last].span.start < start.start => {
        self.missing_span()
      }
      Some(last) => start.join(&self.tokens[last].span),
      None => start.clone(),
    }
  }

  #[track_caller]
  pub fn progress(&self, before: usize) {
    invariant(self.pos > before, || {
      format!("parser made no progress at token {before}")
    });
  }

  fn with_restriction<T>(
    &mut self,
    no_record: bool,
    f: impl FnOnce(&mut Self) -> T,
  ) -> T {
    let saved = std::mem::replace(&mut self.no_record, no_record);
    let result = f(self);

    self.no_record = saved;

    result
  }

  pub(crate) fn upper_name(&mut self, context: &str) -> Name {
    self.name(Upper, context)
  }

  pub(crate) fn lower_name(&mut self, context: &str) -> Name {
    self.name(Lower, context)
  }

  pub(crate) fn name(&mut self, kind: TokenKind, context: &str) -> Name {
    match self.expect(kind, context).map(|token| token.span.clone()) {
      Some(span) => {
        let text = self.file.slice(&span).to_string();

        if kind == Lower {
          self.keyword_as_name(&text, &span);
        }

        Name { text, span }
      }
      None => Name { text: String::new(), span: self.missing_span() },
    }
  }

  pub(crate) fn keyword_as_name(&mut self, text: &str, span: &Span) {
    if !self.enabled.iter().any(|id| plugins::keyword(*id) == text) {
      return;
    }

    self.diagnostics.push(
      Diagnostic::error(
        PluginKeywordAsName,
        format!("`{text}` is a zone keyword in this project, from a plugin"),
        Label::new(span.clone()),
      )
      .with_help("choose another name"),
    );
  }

  pub(crate) fn at_from(&self, n: usize) -> bool {
    let token = self.peek(n);

    token.kind == Lower && self.file.slice(&token.span) == "from"
  }

  pub(crate) fn column(&self, span: &Span) -> usize {
    self.file.position_at(span.start).column
  }

  pub(crate) fn missing_span(&self) -> Span {
    let file = self.peek(0).span.file.clone();

    match self.pos.checked_sub(1) {
      Some(last) => Span::empty(file, self.tokens[last].span.end),
      None => Span::empty(file, 0),
    }
  }

  pub(crate) fn error_span(&self) -> Span {
    let token = &self.tokens[self.pos];

    if token.kind == Eof {
      Span::empty(token.span.file.clone(), self.file.text().len())
    } else {
      token.span.clone()
    }
  }
}
