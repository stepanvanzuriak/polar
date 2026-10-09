use super::Parser;
use crate::{
  shared::codes::DiagnosticCode::InterpolationInPattern,
  shared::source::Span,
  syntax::ast::{
    BoolLit, FloatLit, IntLit, InvalidPattern, PCtor, PField, PList, PLit, POr,
    PRecord, PVar, PWildcard, PatLit, Pattern, StringPart,
  },
  syntax::lexer::token::TokenKind::{
    Bar, Colon, Comma, DotDot, Eof, Float, Int, KwFalse, KwTrue, LBrace,
    LBracket, LParen, Lower, Minus, RBrace, RBracket, RParen, StringStart,
    Underscore, Upper,
  },
};

impl Parser<'_> {
  pub fn pattern(&mut self) -> Pattern {
    let first = self.single_pattern();

    if !self.at(Bar) {
      return first;
    }

    let start = first.span().clone();
    let mut alternatives = vec![first];

    while self.eat(Bar).is_some() {
      alternatives.push(self.single_pattern());
    }

    Pattern::Or(POr { span: self.span_from(&start), alternatives })
  }

  pub(super) fn single_pattern(&mut self) -> Pattern {
    match self.nested(Self::pattern_inner) {
      Some(pattern) => pattern,
      None => self.invalid_pattern(),
    }
  }

  fn pattern_inner(&mut self) -> Pattern {
    let token = self.peek(0);
    let span = token.span.clone();

    match token.kind {
      Underscore => {
        self.bump();
        Pattern::Wildcard(PWildcard { span })
      }
      Lower => {
        let name = self.lower_name("as a pattern variable");

        Pattern::Var(PVar { span: name.span.clone(), name })
      }
      Int | Float | KwTrue | KwFalse | StringStart => {
        let lit = self.pat_lit();

        Pattern::Lit(PLit { span: self.span_from(&span), lit, negative: false })
      }
      Minus if matches!(self.peek(1).kind, Int | Float) => {
        self.bump();

        let lit = self.pat_lit();

        Pattern::Lit(PLit { span: self.span_from(&span), lit, negative: true })
      }
      Upper => self.ctor_pattern(),
      LBrace => self.record_pattern(),
      LBracket => self.list_pattern(),
      _ => {
        self.expected("a pattern");

        self.invalid_pattern()
      }
    }
  }

  fn pat_lit(&mut self) -> PatLit {
    let token = self.peek(0);
    let span = token.span.clone();

    match token.kind {
      Int => {
        self.bump();
        PatLit::Int(IntLit { raw: self.file.slice(&span).to_string(), span })
      }
      Float => {
        self.bump();
        PatLit::Float(FloatLit {
          raw: self.file.slice(&span).to_string(),
          span,
        })
      }
      KwTrue | KwFalse => {
        let value = token.kind == KwTrue;

        self.bump();
        PatLit::Bool(BoolLit { span, value })
      }
      _ => {
        let lit = self.string_lit();

        if let Some(StringPart::Interp(interp)) =
          lit.parts.iter().find(|part| matches!(part, StringPart::Interp(_)))
        {
          self.error(
            InterpolationInPattern,
            "a string pattern cannot contain an interpolation",
            interp.span.clone(),
            Some("match the plain string, or bind it and compare in the arm"),
          );
        }

        PatLit::String(lit)
      }
    }
  }

  fn ctor_pattern(&mut self) -> Pattern {
    let name = self.upper_name("as a constructor name");
    let mut args = Vec::new();

    if self.eat(LParen).is_some() {
      while !self.at(RParen) && !self.at(Eof) {
        let before = self.pos;

        args.push(self.pattern());

        if self.eat(Comma).is_none() {
          break;
        }

        self.progress(before);
      }

      self.expect(RParen, "to close the constructor pattern");
    }

    Pattern::Ctor(PCtor { span: self.span_from(&name.span), name, args })
  }

  fn record_pattern(&mut self) -> Pattern {
    let start = self.bump().span.clone();
    let mut fields = Vec::new();
    let mut open = false;

    while !self.at(RBrace) && !self.at(Eof) {
      let before = self.pos;

      if self.eat(DotDot).is_some() {
        open = true;
        self.eat(Comma);
        break;
      }

      let name = self.lower_name("as a record field name");

      self.expect(Colon, "after a record field name in a pattern");

      let pattern = self.pattern();

      fields.push(PField { span: self.span_from(&name.span), name, pattern });

      if self.eat(Comma).is_none() {
        break;
      }

      self.progress(before);
    }

    self.expect(RBrace, "to close the record pattern");

    Pattern::Record(PRecord { span: self.span_from(&start), fields, open })
  }

  fn list_pattern(&mut self) -> Pattern {
    let start = self.bump().span.clone();
    let mut items = Vec::new();
    let mut tail = None;

    while !self.at(RBracket) && !self.at(Eof) {
      let before = self.pos;

      if self.eat(DotDot).is_some() {
        tail = Some(Box::new(self.pattern()));
        self.eat(Comma);
        break;
      }

      items.push(self.pattern());

      if self.eat(Comma).is_none() {
        break;
      }

      self.progress(before);
    }

    self.expect(RBracket, "to close the list pattern");

    Pattern::List(PList { span: self.span_from(&start), items, tail })
  }

  fn invalid_pattern(&self) -> Pattern {
    let span: Span = self.missing_span();

    Pattern::Invalid(InvalidPattern { span })
  }
}
