use super::Parser;
use crate::{
  shared::codes::DiagnosticCode::EffectVariableWithoutBar,
  shared::diagnostic::{Diagnostic, Label},
  syntax::ast::{
    EffectRow, FieldType, FnType, InvalidType, Name, RecordType, TypeExpr,
    TypeRef, TypeVar,
  },
  syntax::lexer::token::TokenKind::{
    Arrow, Bar, Colon, Comma, Eof, Gt, KwFunction, LBrace, LParen, Lower, Lt,
    RBrace, RParen, Slash, Upper,
  },
};

impl Parser<'_> {
  pub fn type_expr(&mut self) -> TypeExpr {
    match self.nested(Self::type_expr_inner) {
      Some(ty) => ty,
      None => self.invalid_type(),
    }
  }

  pub fn effect_row(&mut self) -> Option<EffectRow> {
    let start = self.eat(Slash)?.span.clone();

    self.expect(LBrace, "to open an effect row");

    let mut entries = Vec::new();
    let mut tail = None;

    loop {
      if self.at(RBrace) || self.at(Bar) || self.at(Eof) {
        break;
      }

      let before = self.pos;

      if self.at(Lower) {
        let name = self.lower_name("as an effect variable");

        self.effect_variable_without_bar(&name);

        if tail.is_none() {
          tail = Some(name);
        }
      } else {
        entries.push(self.type_ref());
      }

      if self.eat(Comma).is_none() {
        break;
      }

      self.progress(before);
    }

    if self.eat(Bar).is_some() {
      tail = Some(self.lower_name("as an effect variable"));
    }

    self.expect(RBrace, "to close the effect row");

    Some(EffectRow { span: self.span_from(&start), entries, tail })
  }

  fn type_expr_inner(&mut self) -> TypeExpr {
    match self.peek(0).kind {
      KwFunction => self.fn_type(),
      LBrace => self.record_type(),
      LParen => self.paren_type(),
      Upper => TypeExpr::Ref(self.type_ref()),
      Lower => TypeExpr::Var(self.type_var()),
      _ => {
        self.expected("a type");

        self.invalid_type()
      }
    }
  }

  pub(super) fn type_ref(&mut self) -> TypeRef {
    let name = self.upper_name("as a type name");
    let mut args = Vec::new();

    if self.eat(Lt).is_some() {
      loop {
        if self.at(Gt) || self.at(Eof) {
          break;
        }

        let before = self.pos;

        args.push(self.type_expr());

        if self.eat(Comma).is_none() {
          break;
        }

        self.progress(before);
      }

      self.expect(Gt, "to close the type arguments");
    }

    TypeRef { span: self.span_from(&name.span), name, args }
  }

  fn type_var(&mut self) -> TypeVar {
    let name = self.lower_name("as a type variable");

    TypeVar { span: name.span.clone(), name }
  }

  fn paren_type(&mut self) -> TypeExpr {
    self.bump();

    let inner = self.type_expr();

    self.expect(RParen, "to close the parenthesised type");

    inner
  }

  fn record_type(&mut self) -> TypeExpr {
    let start = self.bump().span.clone();
    let mut fields = Vec::new();

    loop {
      if self.at(RBrace) || self.at(Bar) || self.at(Eof) {
        break;
      }

      let before = self.pos;
      let name = self.lower_name("as a record field name");

      self.expect(Colon, "after a record field name");

      let ty = self.type_expr();

      fields.push(FieldType { span: self.span_from(&name.span), name, ty });

      if self.eat(Comma).is_none() {
        break;
      }

      self.progress(before);
    }

    let tail = if self.eat(Bar).is_some() {
      Some(self.lower_name("as a row variable"))
    } else {
      None
    };

    self.expect(RBrace, "to close the record type");

    TypeExpr::Record(RecordType { span: self.span_from(&start), fields, tail })
  }

  fn fn_type(&mut self) -> TypeExpr {
    let start = self.bump().span.clone();
    let mut params = Vec::new();

    self.expect(LParen, "after `function` in a function type");

    loop {
      if self.at(RParen) || self.at(Eof) {
        break;
      }

      let before = self.pos;

      params.push(self.type_expr());

      if self.eat(Comma).is_none() {
        break;
      }

      self.progress(before);
    }

    self.expect(RParen, "to close the function type's parameters");
    self.expect(Arrow, "before the function type's return type");

    let ret = Box::new(self.type_expr());
    let effects = self.effect_row();

    TypeExpr::Fn(FnType { span: self.span_from(&start), params, ret, effects })
  }

  fn effect_variable_without_bar(&mut self, name: &Name) {
    self.diagnostics.push(
      Diagnostic::error(
        EffectVariableWithoutBar,
        format!("effect variable `{}` in an effect row without `|`", name.text),
        Label::new(name.span.clone()),
      )
      .with_help("effect variables go after `|`: `{Db | e}`"),
    );
  }

  pub(super) fn invalid_type(&self) -> TypeExpr {
    TypeExpr::Invalid(InvalidType { span: self.missing_span() })
  }
}
