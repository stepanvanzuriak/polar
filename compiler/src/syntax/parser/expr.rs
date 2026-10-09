use super::{
  Parser,
  precedence::{Assoc, InfixEntry, PREFIX_BINDING_POWER},
};
use crate::{
  shared::codes::DiagnosticCode::{
    self, ChainedComparison, EmptyBlock, LocalFnDeclaration, MatchPatternCount,
    RecordLiteralInCondition, SpreadNotFirst, StatementsOnSameLine,
    TrailingLet, UnexpectedToken,
  },
  shared::diagnostic::{Diagnostic, Label},
  shared::source::Span,
  syntax::ast::{
    Binary, BinaryOp, Block, BoolLit, Call, Else, Expr, ExprStmt, FieldAccess,
    FieldInit, FloatLit, If, IntLit, InvalidExpr, Lambda, LetStmt, ListLit,
    Match, MatchArm, Name, PVar, Pattern, Pipe, RecordLit, Return, Stmt,
    StringInterp, StringLit, StringPart, StringText, Throw, Try, Unary,
    UnaryOp, Var,
  },
  syntax::lexer::token::TokenKind::{
    self, Amp, AndAnd, Arrow, Bang, BangEq, Bar, Caret, Colon, Comma, Dot,
    DotDot, Eof, Eq, EqEq, Float, Ge, Gt, Int, InterpEnd, InterpStart, KwCatch,
    KwElse, KwFalse, KwFunction, KwIf, KwLet, KwMatch, KwReturn, KwThrow,
    KwTrue, KwTry, LBrace, LBracket, LParen, Le, Lower, Lt, Minus, OrOr,
    Percent, PipeOp, Plus, RBrace, RBracket, RParen, Slash, Star, StringEnd,
    StringPart as StringPartTok, StringStart, Tilde, Underscore, Upper,
  },
};

enum Item {
  Let(Box<LetStmt>),
  Expr(Expr),
}

impl Parser<'_> {
  pub fn expr(&mut self) -> Expr {
    self.expr_bp(0)
  }

  fn expr_bp(&mut self, min_bp: u8) -> Expr {
    match self.nested(|p| p.expr_loop(min_bp)) {
      Some(expr) => expr,
      None => self.invalid_expr(),
    }
  }

  fn expr_loop(&mut self, min_bp: u8) -> Expr {
    let start = self.peek(0).span.clone();
    let before = self.pos;
    let mut left = self.prefix();
    let mut chained: Option<u8> = None;

    if self.pos == before {
      return left;
    }

    let depth = self.depth;

    loop {
      let token = self.peek(0);
      let (kind, newline) = (token.kind, token.newline_before);

      let Some(entry) = self.table.infix(kind) else { break };

      if entry.bp < min_bp {
        break;
      }

      if kind == LParen && newline {
        break;
      }

      if !self.deepen() {
        break;
      }

      if entry.assoc == Assoc::None && chained == Some(entry.bp) {
        self.chained_comparison();
      }

      left = self.infix(left, &start, kind, entry);
      chained = (entry.assoc == Assoc::None).then_some(entry.bp);
    }

    self.depth = depth;

    left
  }

  fn infix(
    &mut self,
    left: Expr,
    start: &Span,
    kind: TokenKind,
    entry: InfixEntry,
  ) -> Expr {
    match kind {
      Dot => self.field_access(left, start),
      LParen => self.call(left, start),
      PipeOp => {
        self.bump();

        let right = self.expr_bp(entry.right_bp());

        Expr::Pipe(Pipe {
          span: self.span_from(start),
          left: Box::new(left),
          right: Box::new(right),
        })
      }
      _ => {
        let op_span = self.bump().span.clone();
        let right = self.expr_bp(entry.right_bp());

        Expr::Binary(Binary {
          span: self.span_from(start),
          op: binary_op(kind),
          op_span,
          left: Box::new(left),
          right: Box::new(right),
        })
      }
    }
  }

  fn field_access(&mut self, target: Expr, start: &Span) -> Expr {
    self.bump();

    let field = if self.at(Upper) {
      let span = self.bump().span.clone();
      let text = self.file.slice(&span).to_string();

      self.error(
        UnexpectedToken,
        format!("expected a field name, found `{text}`"),
        span.clone(),
        Some("field names start with a lowercase letter"),
      );

      Name { text, span }
    } else {
      self.lower_name("as a field name")
    };

    Expr::Field(FieldAccess {
      span: self.span_from(start),
      target: Box::new(target),
      field,
    })
  }

  fn call(&mut self, callee: Expr, start: &Span) -> Expr {
    self.bump();

    let mut args = Vec::new();

    while !self.at(RParen) && !self.at(Eof) {
      let before = self.pos;

      args.push(self.delimited_expr());

      if self.eat(Comma).is_none() {
        break;
      }

      self.progress(before);
    }

    self.expect(RParen, "to close the argument list");

    Expr::Call(Call {
      span: self.span_from(start),
      callee: Box::new(callee),
      args,
    })
  }

  fn prefix(&mut self) -> Expr {
    match self.peek(0).kind {
      Int | Float | KwTrue | KwFalse | Lower | Upper | Underscore => {
        self.atom()
      }
      StringStart => Expr::String(self.string_lit()),
      LParen => self.paren(),
      LBrace => self.brace_expr(),
      LBracket => self.list(),
      KwFunction => self.lambda(),
      KwIf => match self.nested(Self::if_expr) {
        Some(node) => Expr::If(node),
        None => self.invalid_expr(),
      },
      KwMatch => self.match_expr(),
      KwThrow => self.throw_expr(),
      KwReturn => self.return_expr(),
      KwTry => self.try_expr(),
      Minus | Bang | Tilde => self.unary(),
      _ => {
        self.expected("expression");

        self.invalid_expr()
      }
    }
  }

  fn atom(&mut self) -> Expr {
    let token = self.peek(0);
    let (kind, span) = (token.kind, token.span.clone());
    let text = self.file.slice(&span).to_string();

    self.bump();

    match kind {
      Int => Expr::Int(IntLit { raw: text, span }),
      Float => Expr::Float(FloatLit { raw: text, span }),
      KwTrue | KwFalse => Expr::Bool(BoolLit { span, value: kind == KwTrue }),
      Underscore => {
        self.error(
          UnexpectedToken,
          "expected expression, found `_`",
          span.clone(),
          Some("`_` can only be used in patterns"),
        );

        Expr::Invalid(InvalidExpr { span })
      }
      _ => Expr::Var(Var { span: span.clone(), name: Name { text, span } }),
    }
  }

  fn unary(&mut self) -> Expr {
    let start = self.peek(0).span.clone();
    let op = match self.bump().kind {
      Minus => UnaryOp::Negate,
      Tilde => UnaryOp::BitNot,
      _ => UnaryOp::Not,
    };
    let operand = self.expr_bp(PREFIX_BINDING_POWER);

    Expr::Unary(Unary {
      span: self.span_from(&start),
      op,
      operand: Box::new(operand),
    })
  }

  fn throw_expr(&mut self) -> Expr {
    let start = self.bump().span.clone();
    let value = self.expr();

    Expr::Throw(Throw { span: self.span_from(&start), value: Box::new(value) })
  }

  fn try_expr(&mut self) -> Expr {
    let start = self.bump().span.clone();
    let body = self.block("after `try`");
    let catch_span = self.peek(0).span.clone();

    if self.expect(KwCatch, "after the `try` block").is_none() {
      return Expr::Try(Try {
        span: self.span_from(&start),
        body: Box::new(body),
        catch_span,
        arms: vec![],
      });
    }

    let arms = self.catch_arms(&catch_span);

    Expr::Try(Try {
      span: self.span_from(&start),
      body: Box::new(body),
      catch_span,
      arms,
    })
  }

  fn catch_arms(&mut self, catch_span: &Span) -> Vec<MatchArm> {
    if !self.at(LBrace) {
      self.expected("`{` to open the `catch` arms");

      return vec![];
    }

    if self.peek(1).kind == RBrace {
      let open = self.bump().span.clone();
      let close = self.bump().span.clone();

      self.error(
        EmptyBlock,
        "a `catch` needs at least one arm",
        catch_span.join(&close).join(&open),
        Some("add an arm for each error it handles, like `Missing(k) -> k`"),
      );

      return vec![];
    }

    self.bump();

    let arms = self.arms(1);

    self.expect(RBrace, "to close the `catch`");

    arms
  }

  pub(super) fn string_lit(&mut self) -> StringLit {
    let start = self.bump().span.clone();
    let mut parts = Vec::new();

    loop {
      let token = self.peek(0);

      match token.kind {
        StringPartTok => {
          let span = token.span.clone();
          let value = token.value.clone().unwrap_or_default();

          self.bump();
          parts.push(StringPart::Text(StringText {
            raw: self.file.slice(&span).to_string(),
            value,
            span,
          }));
        }
        InterpStart => parts.push(StringPart::Interp(self.interpolation())),
        StringEnd => {
          self.bump();
          break;
        }
        _ => break,
      }
    }

    StringLit { span: self.span_from(&start), parts }
  }

  fn interpolation(&mut self) -> StringInterp {
    let start = self.bump().span.clone();
    let expr = self.delimited_expr();

    self.expect(InterpEnd, "to close the interpolation");

    StringInterp { span: self.span_from(&start), expr: Box::new(expr) }
  }

  fn paren(&mut self) -> Expr {
    let open = self.bump().span.clone();

    if self.at(RParen) {
      let close = self.bump().span.clone();
      let span = open.join(&close);

      self.error(
        UnexpectedToken,
        "expected expression, found `)`",
        span.clone(),
        Some("use `{}` for an empty value"),
      );

      return Expr::Invalid(InvalidExpr { span });
    }

    let inner = self.delimited_expr();

    self.expect(RParen, "to close the parenthesised expression");

    inner
  }

  fn brace_expr(&mut self) -> Expr {
    let is_record = matches!(
      (self.peek(1).kind, self.peek(2).kind),
      (RBrace | DotDot, _) | (Lower, Colon)
    );

    if !is_record {
      return Expr::Block(self.block_body());
    }

    if self.no_record {
      self.error(
        RecordLiteralInCondition,
        "record literal in a condition",
        self.peek(0).span.clone(),
        Some("wrap it in parentheses"),
      );
    }

    Expr::Record(self.record_lit())
  }

  fn record_lit(&mut self) -> RecordLit {
    let start = self.bump().span.clone();
    let mut spread = None;
    let mut fields = Vec::new();

    self.with_restriction(false, |p| {
      while !p.at(RBrace) && !p.at(Eof) {
        let before = p.pos;

        if p.at(DotDot) {
          let dots = p.bump().span.clone();
          let value = p.delimited_expr();

          if spread.is_some() || !fields.is_empty() {
            p.error(
              SpreadNotFirst,
              "the spread must come first in a record literal",
              p.span_from(&dots),
              Some("move `..` before the fields: `{ ..base, field: value }`"),
            );
          }

          if spread.is_none() {
            spread = Some(Box::new(value));
          }
        } else {
          fields.push(p.field_init());
        }

        if p.eat(Comma).is_none() {
          break;
        }

        p.progress(before);
      }
    });

    self.expect(RBrace, "to close the record literal");

    RecordLit { span: self.span_from(&start), spread, fields }
  }

  fn field_init(&mut self) -> FieldInit {
    let name = self.lower_name("as a record field name");

    self.expect(Colon, "after a record field name");

    let value = self.delimited_expr();

    FieldInit { span: self.span_from(&name.span), name, value }
  }

  fn list(&mut self) -> Expr {
    let start = self.bump().span.clone();
    let mut items = Vec::new();
    let mut tail = None;

    self.with_restriction(false, |p| {
      while !p.at(RBracket) && !p.at(Eof) {
        let before = p.pos;

        if p.eat(DotDot).is_some() {
          tail = Some(Box::new(p.delimited_expr()));
          p.eat(Comma);
          break;
        }

        items.push(p.delimited_expr());

        if p.eat(Comma).is_none() {
          break;
        }

        p.progress(before);
      }
    });

    self.expect(RBracket, "to close the list");

    Expr::List(ListLit { span: self.span_from(&start), items, tail })
  }

  fn lambda(&mut self) -> Expr {
    let start = self.bump().span.clone();

    match self.nested(|p| p.lambda_rest(&start)) {
      Some(lambda) => Expr::Lambda(lambda),
      None => self.invalid_expr(),
    }
  }

  fn lambda_rest(&mut self, start: &Span) -> Box<Lambda> {
    let mut lambda = self.lambda_signature();

    *lambda.body = self.block("to open the function body");
    lambda.span = self.span_from(start);

    lambda
  }

  fn lambda_signature(&mut self) -> Box<Lambda> {
    let params = self.params();

    let (return_type, effects) = if self.eat(Arrow).is_some() {
      (Some(self.type_expr()), self.effect_row())
    } else {
      (None, None)
    };

    Box::new(Lambda {
      span: self.missing_span(),
      params,
      return_type,
      effects,
      body: Box::new(self.missing_block()),
    })
  }

  fn if_expr(&mut self) -> If {
    let start = self.bump().span.clone();
    let cond = self.with_restriction(true, Self::expr);
    let then_branch = self.block("to open the `if` branch");

    let else_branch = if self.eat(KwElse).is_some() {
      Some(Box::new(if self.at(KwIf) {
        match self.nested(Self::if_expr) {
          Some(inner) => Else::If(inner),
          None => Else::Block(self.missing_block()),
        }
      } else {
        Else::Block(self.block("to open the `else` branch"))
      }))
    } else {
      None
    };

    If {
      span: self.span_from(&start),
      cond: Box::new(cond),
      then_branch: Box::new(then_branch),
      else_branch,
    }
  }

  fn match_expr(&mut self) -> Expr {
    let start = self.bump().span.clone();
    let mut subjects = vec![self.with_restriction(true, Self::expr)];

    while self.eat(Comma).is_some() {
      subjects.push(self.with_restriction(true, Self::expr));
    }

    if self.expect(LBrace, "to open the `match` arms").is_none() {
      return Expr::Match(Match {
        span: self.span_from(&start),
        subjects,
        arms: vec![],
      });
    }

    let arms = self.arms(subjects.len());

    self.expect(RBrace, "to close the `match`");

    Expr::Match(Match { span: self.span_from(&start), subjects, arms })
  }

  fn arms(&mut self, subjects: usize) -> Vec<MatchArm> {
    let mut arms = Vec::new();

    self.with_restriction(false, |p| {
      while !p.at(RBrace) && !p.at(Eof) {
        let before = p.pos;

        arms.push(p.match_arm(subjects));

        let braced = p.pos > before && p.tokens[p.pos - 1].kind == RBrace;

        if p.eat(Comma).is_none() && !braced {
          break;
        }

        p.progress(before);
      }
    });

    arms
  }

  fn match_arm(&mut self, subjects: usize) -> MatchArm {
    let mut rows = vec![self.pattern_row(subjects)];
    let start = rows[0][0].span().clone();

    while self.eat(Bar).is_some() {
      rows.push(self.pattern_row(subjects));
    }

    let guard = if self.eat(KwIf).is_some() {
      Some(Box::new(self.with_restriction(false, Self::expr)))
    } else {
      None
    };

    self.expect(Arrow, "after a `match` pattern");

    let body = self.delimited_expr();

    MatchArm { span: self.span_from(&start), rows, guard, body }
  }

  fn pattern_row(&mut self, subjects: usize) -> Vec<Pattern> {
    let mut patterns = vec![self.single_pattern()];
    let start = patterns[0].span().clone();

    while self.eat(Comma).is_some() {
      patterns.push(self.single_pattern());
    }

    if patterns.len() != subjects {
      let span = start.join(patterns.last().map_or(&start, Pattern::span));
      let what = |n: usize| {
        if n == 1 { "1 pattern".to_string() } else { format!("{n} patterns") }
      };

      self.error(
        MatchPatternCount,
        format!(
          "this arm has {}, but the `match` has {} {}",
          what(patterns.len()),
          subjects,
          if subjects == 1 { "value" } else { "values" }
        ),
        span,
        Some("write one pattern per value, separated by commas"),
      );
    }

    patterns
  }

  fn return_expr(&mut self) -> Expr {
    let start = self.bump().span.clone();
    let next = self.peek(0);
    let value = if starts_expr(next.kind) && !next.newline_before {
      self.expr()
    } else {
      Expr::Record(RecordLit {
        span: start.clone(),
        spread: None,
        fields: Vec::new(),
      })
    };

    Expr::Return(Return {
      span: self.span_from(&start),
      value: Box::new(value),
    })
  }

  pub(super) fn block(&mut self, context: &str) -> Block {
    if !self.at(LBrace) {
      self.expected(&format!("`{{` {context}"));

      return self.missing_block();
    }

    if self.peek(1).kind == RBrace {
      let open = self.bump().span.clone();
      let span = open.join(&self.bump().span.clone());

      self.error(
        EmptyBlock,
        "empty block has no value",
        span.clone(),
        Some("a block ends with the expression it evaluates to"),
      );

      return Block {
        span: span.clone(),
        stmts: vec![],
        result: Box::new(Expr::Invalid(InvalidExpr { span })),
      };
    }

    self.block_body()
  }

  fn block_body(&mut self) -> Block {
    match self.nested(Self::block_body_inner) {
      Some(block) => block,
      None => self.skip_block(),
    }
  }

  fn block_body_inner(&mut self) -> Block {
    let start = self.bump().span.clone();
    let items = self.with_restriction(false, Self::block_items);

    if self.eat(RBrace).is_none() && !self.at_limit() {
      self.expected("`}` to close the block");
    }

    let span = self.span_from(&start);

    self.finish_block(span, items)
  }

  fn finish_block(&mut self, span: Span, items: Vec<Item>) -> Block {
    let mut stmts = Vec::with_capacity(items.len());
    let mut result = None;
    let last = items.len().saturating_sub(1);

    for (i, item) in items.into_iter().enumerate() {
      match item {
        Item::Expr(expr) if i == last => result = Some(expr),
        Item::Expr(expr) => {
          stmts.push(Stmt::Expr(ExprStmt { span: expr.span().clone(), expr }));
        }
        Item::Let(stmt) => {
          if i == last && !self.at_limit() {
            self.trailing_let(&stmt);
          }

          stmts.push(Stmt::Let(*stmt));
        }
      }
    }

    let result = result.unwrap_or_else(|| {
      Expr::Invalid(InvalidExpr {
        span: Span::empty(span.file.clone(), span.end),
      })
    });

    Block { span, stmts, result: Box::new(result) }
  }

  fn skip_block(&mut self) -> Block {
    let start = self.peek(0).span.clone();
    let mut depth = 0usize;

    while !self.at(Eof) {
      match self.bump().kind {
        LBrace | InterpStart => depth += 1,
        RBrace | InterpEnd => depth = depth.saturating_sub(1),
        _ => {}
      }

      if depth == 0 {
        break;
      }
    }

    let span = self.span_from(&start);

    Block {
      span: span.clone(),
      stmts: vec![],
      result: Box::new(Expr::Invalid(InvalidExpr { span })),
    }
  }

  fn block_items(&mut self) -> Vec<Item> {
    let mut items = Vec::new();

    while !self.at(RBrace) && !self.at(Eof) {
      let before = self.pos;
      let errors = self.diagnostics.error_count();
      let item = self.item();

      if self.pos == before {
        self.bump();
      }

      if self.diagnostics.error_count() > errors {
        self.synchronise_statement();
      } else {
        self.statement_separator(&item);
      }

      items.push(item);
    }

    items
  }

  fn item(&mut self) -> Item {
    match (self.peek(0).kind, self.peek(1).kind) {
      (KwLet, _) => Item::Let(self.let_stmt()),
      (KwFunction, Lower) => Item::Let(Box::new(self.local_fn())),
      _ => Item::Expr(self.expr()),
    }
  }

  fn let_stmt(&mut self) -> Box<LetStmt> {
    let start = self.bump().span.clone();
    let mut stmt = self.let_head();

    stmt.value = self.expr();
    stmt.span = self.span_from(&start);

    stmt
  }

  fn let_head(&mut self) -> Box<LetStmt> {
    let pattern = self.pattern();
    let ty =
      if self.eat(Colon).is_some() { Some(self.type_expr()) } else { None };

    self.expect(Eq, "after the `let` pattern");

    Box::new(LetStmt {
      span: self.missing_span(),
      pattern,
      ty,
      value: self.invalid_expr(),
    })
  }

  fn local_fn(&mut self) -> LetStmt {
    let start = self.bump().span.clone();
    let name = self.lower_name("as the function name");

    self.error(
      LocalFnDeclaration,
      "local named functions are not supported",
      self.span_from(&start),
      Some(format!("use `let {} = function(...) {{ ... }}`", name.text)),
    );

    let lambda_start = if self.at(LParen) {
      self.peek(0).span.clone()
    } else {
      self.missing_span()
    };
    let lambda = self.lambda_rest(&lambda_start);

    LetStmt {
      span: self.span_from(&start),
      pattern: Pattern::Var(PVar { span: name.span.clone(), name }),
      ty: None,
      value: Expr::Lambda(lambda),
    }
  }

  fn statement_separator(&mut self, item: &Item) {
    let token = self.peek(0);

    if token.newline_before || matches!(token.kind, RBrace | Eof) {
      return;
    }

    let span = self.error_span();
    let juxtaposed =
      matches!(item, Item::Expr(Expr::Var(_))) && starts_expr(token.kind);

    let help = if juxtaposed {
      "function calls need parentheses: `f(x)`"
    } else {
      "put each statement on its own line"
    };

    self.error(
      StatementsOnSameLine,
      "statements in a block must be separated by a newline",
      span,
      Some(help),
    );
  }

  fn synchronise_statement(&mut self) {
    let mut depth = 0usize;

    while !self.at(Eof) {
      let token = self.peek(0);

      if depth == 0 && (token.kind == RBrace || token.newline_before) {
        break;
      }

      match self.bump().kind {
        LBrace | InterpStart => depth += 1,
        RBrace | InterpEnd => depth = depth.saturating_sub(1),
        _ => {}
      }
    }
  }

  fn trailing_let(&mut self, stmt: &LetStmt) {
    self.error(
      TrailingLet,
      "`let` has no value, so it cannot end a block",
      stmt.span.clone(),
      Some("add the value to return after it"),
    );
  }

  fn chained_comparison(&mut self) {
    let span = self.peek(0).span.clone();

    self.error(
      ChainedComparison,
      "comparison operators cannot be chained",
      span,
      Some("`a < b && b < c`"),
    );
  }

  fn delimited_expr(&mut self) -> Expr {
    match self.nested(|p| p.with_restriction(false, Self::expr)) {
      Some(expr) => expr,
      None => self.invalid_expr(),
    }
  }

  fn missing_block(&self) -> Block {
    let span = self.missing_span();

    Block {
      span: span.clone(),
      stmts: vec![],
      result: Box::new(Expr::Invalid(InvalidExpr { span })),
    }
  }

  fn invalid_expr(&self) -> Expr {
    Expr::Invalid(InvalidExpr { span: self.missing_span() })
  }

  pub(super) fn error(
    &mut self,
    code: DiagnosticCode,
    message: impl Into<String>,
    span: Span,
    help: Option<impl Into<String>>,
  ) {
    let mut diagnostic = Diagnostic::error(code, message, Label::new(span));

    if let Some(help) = help {
      diagnostic = diagnostic.with_help(help);
    }

    self.diagnostics.push(diagnostic);
  }
}

fn binary_op(kind: TokenKind) -> BinaryOp {
  match kind {
    OrOr => BinaryOp::Or,
    AndAnd => BinaryOp::And,
    EqEq => BinaryOp::Eq,
    BangEq => BinaryOp::NotEq,
    Lt => BinaryOp::Lt,
    Le => BinaryOp::LtEq,
    Gt => BinaryOp::Gt,
    Ge => BinaryOp::GtEq,
    Plus => BinaryOp::Add,
    Minus => BinaryOp::Sub,
    Star => BinaryOp::Mul,
    Slash => BinaryOp::Div,
    Percent => BinaryOp::Rem,
    Amp => BinaryOp::BitAnd,
    Bar => BinaryOp::BitOr,
    Caret => BinaryOp::BitXor,
    other => crate::shared::ice::ice(
      format!(
        "{} is in the precedence table but is not a binary operator",
        other.display_name()
      ),
      None,
    ),
  }
}

fn starts_expr(kind: TokenKind) -> bool {
  matches!(
    kind,
    Int
      | Float
      | StringStart
      | Lower
      | Upper
      | LParen
      | LBrace
      | LBracket
      | KwTrue
      | KwFalse
      | KwFunction
      | KwIf
      | KwMatch
      | KwThrow
      | KwReturn
      | KwTry
      | Bang
  )
}
