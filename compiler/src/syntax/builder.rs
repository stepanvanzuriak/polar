use std::{
  collections::{HashMap, HashSet},
  sync::Arc,
};

use crate::{
  core::ir::{CExpr, CExprKind, CModule},
  shared::diagnostic::DiagnosticBag,
  shared::source::{SourceFile, Span},
  syntax::ast::{
    Binary, BinaryOp, Block, BoolLit, Call, Expr, FieldAccess, FieldInit,
    FnDecl, ListLit, Match, MatchArm, Name, PCtor, PField, PList, PLit,
    PRecord, PVar, PWildcard, Param, PatLit, Pattern, RecordLit, StringInterp,
    StringLit, StringPart, StringText, Var,
  },
};

#[derive(Debug, Default, Clone)]
pub struct Expansion {
  pub real: HashMap<usize, Span>,
  pub derived: HashMap<(usize, usize), (String, String)>,
  pub std_lists: HashSet<(usize, usize)>,
  pub std_ctors: HashSet<(usize, usize)>,
  pub std_types: std::collections::BTreeSet<String>,
  pub(crate) limit: usize,
}

impl Expansion {
  #[must_use]
  pub fn is_synthetic(&self, span: &Span) -> bool {
    self.limit > 0 && span.start > self.limit
  }

  #[must_use]
  pub fn real_span(&self, span: &Span) -> Span {
    if !self.is_synthetic(span) {
      return span.clone();
    }

    self
      .real
      .get(&span.start)
      .cloned()
      .unwrap_or_else(|| Span::empty(span.file.clone(), self.limit))
  }

  pub fn remap(&self, bag: &mut DiagnosticBag) {
    if !self.real.is_empty() {
      bag.map_spans(|span| self.real_span(span));
    }
  }

  pub fn remap_core(&self, module: &mut CModule) {
    if self.real.is_empty() {
      return;
    }

    for bind in &mut module.binds {
      if let Some(origin) = &bind.origin {
        bind.origin = Some(self.real_span(origin));
      }
    }

    let binds = module.binds.iter_mut().flat_map(|b| &mut b.ops);

    for decl in module.decls.iter_mut().chain(binds) {
      if let Some(origin) = &decl.origin {
        decl.origin = Some(self.real_span(origin));
      }

      self.remap_expr(&mut decl.body);
    }
  }

  fn remap_expr(&self, e: &mut CExpr) {
    if let Some(origin) = &e.origin {
      e.origin = Some(self.real_span(origin));
    }

    match &mut e.kind {
      CExprKind::Lit(_)
      | CExprKind::Var(_)
      | CExprKind::Builtin { .. }
      | CExprKind::Std { .. }
      | CExprKind::User { .. }
      | CExprKind::Method { .. }
      | CExprKind::CtorFn { .. }
      | CExprKind::MatchFail
      | CExprKind::Op { .. }
      | CExprKind::Extern { .. } => {}
      CExprKind::Throw { value, .. } => self.remap_expr(value),
      CExprKind::Try { body, handler, .. } => {
        self.remap_expr(body);
        self.remap_expr(handler);
      }
      CExprKind::Lam { body, .. } => self.remap_expr(body),
      CExprKind::App { func, args } => {
        self.remap_expr(func);
        for a in args {
          self.remap_expr(a);
        }
      }
      CExprKind::Prim { args, .. }
      | CExprKind::Ctor { args, .. }
      | CExprKind::Dict { args, .. }
      | CExprKind::Concat { parts: args } => {
        for a in args {
          self.remap_expr(a);
        }
      }
      CExprKind::Let { value, body, .. } => {
        self.remap_expr(value);
        self.remap_expr(body);
      }
      CExprKind::If { cond, then_branch, else_branch } => {
        self.remap_expr(cond);
        self.remap_expr(then_branch);
        self.remap_expr(else_branch);
      }
      CExprKind::Case { scrutinee, arms } => {
        self.remap_expr(scrutinee);

        for arm in arms {
          if let Some(origin) = &arm.origin {
            arm.origin = Some(self.real_span(origin));
          }

          self.remap_expr(&mut arm.body);
        }
      }
      CExprKind::Record { fields } => {
        for (_, v) in fields {
          self.remap_expr(v);
        }
      }
      CExprKind::Update { base, fields } => {
        self.remap_expr(base);
        for (_, v) in fields {
          self.remap_expr(v);
        }
      }
      CExprKind::Field { target, .. } | CExprKind::Test { target, .. } => {
        self.remap_expr(target);
      }
    }
  }
}

pub struct Gen {
  next: usize,
  pub(crate) file: Arc<str>,
  pub(crate) real: Span,
  pub(crate) expansion: Expansion,
}

pub enum Piece {
  Text(String),
  Expr(Expr),
}

impl Gen {
  #[must_use]
  pub fn new(file: &SourceFile) -> Self {
    let limit = file.text().len();

    Self {
      next: limit + 1,
      file: Arc::from(file.name()),
      real: Span::empty(Arc::from(file.name()), 0),
      expansion: Expansion { limit, ..Expansion::default() },
    }
  }

  #[must_use]
  pub fn finish(self) -> Expansion {
    self.expansion
  }

  #[must_use]
  pub fn resolve(&self, span: &Span) -> Span {
    if self.expansion.is_synthetic(span) {
      self
        .expansion
        .real
        .get(&span.start)
        .cloned()
        .unwrap_or_else(|| span.clone())
    } else {
      span.clone()
    }
  }

  pub fn set_real(&mut self, span: &Span) {
    self.real = self.resolve(span);
  }
  pub fn span(&mut self) -> Span {
    let span = Span::new(self.file.clone(), self.next, self.next + 1);

    self.next += 2;
    self.expansion.real.insert(span.start, self.real.clone());
    span
  }

  pub fn name(&mut self, text: &str) -> Name {
    Name { text: text.to_string(), span: self.span() }
  }

  pub fn var(&mut self, text: &str) -> Expr {
    let name = self.name(text);

    Expr::Var(Var { span: name.span.clone(), name })
  }

  pub fn field(&mut self, target: Expr, field: &str) -> Expr {
    Expr::Field(FieldAccess {
      span: self.span(),
      target: Box::new(target),
      field: self.name(field),
    })
  }

  pub fn qualified(&mut self, module: &str, member: &str) -> Expr {
    let target = self.var(module);

    self.field(target, member)
  }

  pub fn call(&mut self, callee: Expr, args: Vec<Expr>) -> Expr {
    Expr::Call(Call { span: self.span(), callee: Box::new(callee), args })
  }

  pub fn string(&mut self, pieces: Vec<Piece>) -> Expr {
    let parts = pieces
      .into_iter()
      .map(|piece| match piece {
        Piece::Text(text) => StringPart::Text(StringText {
          span: self.span(),
          raw: text.clone(),
          value: text,
        }),
        Piece::Expr(expr) => StringPart::Interp(StringInterp {
          span: self.span(),
          expr: Box::new(expr),
        }),
      })
      .collect();

    Expr::String(StringLit { span: self.span(), parts })
  }

  pub fn text(&mut self, text: &str) -> Expr {
    self.string(vec![Piece::Text(text.to_string())])
  }

  pub fn boolean(&mut self, value: bool) -> Expr {
    Expr::Bool(BoolLit { span: self.span(), value })
  }

  pub fn and(&mut self, left: Expr, right: Expr) -> Expr {
    Expr::Binary(Binary {
      span: self.span(),
      op: BinaryOp::And,
      op_span: self.span(),
      left: Box::new(left),
      right: Box::new(right),
    })
  }

  pub fn all(&mut self, mut checks: Vec<Expr>) -> Expr {
    if checks.is_empty() {
      return self.boolean(true);
    }

    let first = checks.remove(0);

    checks.into_iter().fold(first, |acc, next| self.and(acc, next))
  }

  pub fn record(&mut self, fields: Vec<(&str, Expr)>) -> Expr {
    let fields = fields
      .into_iter()
      .map(|(name, value)| FieldInit {
        span: self.span(),
        name: self.name(name),
        value,
      })
      .collect();

    Expr::Record(RecordLit { span: self.span(), spread: None, fields })
  }

  pub fn std_list(&mut self, items: Vec<Expr>) -> Expr {
    let span = self.span();

    self.expansion.std_lists.insert((span.start, span.end));
    Expr::List(ListLit { span, items, tail: None })
  }

  pub fn block(&mut self, result: Expr) -> Block {
    Block { span: self.span(), stmts: Vec::new(), result: Box::new(result) }
  }

  pub fn param(&mut self, name: &str) -> Param {
    Param { span: self.span(), name: self.name(name), pattern: None, ty: None }
  }

  pub fn method(&mut self, name: &str, params: &[&str], body: Expr) -> FnDecl {
    let params = params.iter().map(|p| self.param(p)).collect();
    let body = self.block(body);

    FnDecl {
      span: self.span(),
      docs: Vec::new(),
      name: self.name(name),
      params,
      return_type: None,
      effects: None,
      bounds: Vec::new(),
      body,
    }
  }

  pub fn match_expr(
    &mut self,
    scrutinee: Expr,
    arms: Vec<(Pattern, Expr)>,
  ) -> Expr {
    let arms = arms
      .into_iter()
      .map(|(pattern, body)| MatchArm { span: self.span(), pattern, body })
      .collect();

    Expr::Match(Match {
      span: self.span(),
      scrutinee: Box::new(scrutinee),
      arms,
    })
  }

  pub fn p_var(&mut self, name: &str) -> Pattern {
    Pattern::Var(PVar { span: self.span(), name: self.name(name) })
  }

  pub fn p_ctor(&mut self, name: &str, args: Vec<Pattern>) -> Pattern {
    Pattern::Ctor(PCtor { span: self.span(), name: self.name(name), args })
  }

  pub fn p_record(&mut self, fields: Vec<(&str, Pattern)>) -> Pattern {
    let fields = fields
      .into_iter()
      .map(|(name, pattern)| PField {
        span: self.span(),
        name: self.name(name),
        pattern,
      })
      .collect();

    Pattern::Record(PRecord { span: self.span(), fields, open: false })
  }

  pub fn std_ctor(&mut self, name: &str) -> Name {
    let name = self.name(name);

    self.expansion.std_ctors.insert((name.span.start, name.span.end));
    name
  }

  pub fn p_std_ctor(&mut self, name: &str, args: Vec<Pattern>) -> Pattern {
    Pattern::Ctor(PCtor { span: self.span(), name: self.std_ctor(name), args })
  }

  pub fn std_ctor_call(&mut self, name: &str, arg: Expr) -> Expr {
    let name = self.std_ctor(name);
    let callee = Expr::Var(Var { span: name.span.clone(), name });

    self.call(callee, vec![arg])
  }

  pub fn p_wildcard(&mut self) -> Pattern {
    Pattern::Wildcard(PWildcard { span: self.span() })
  }

  pub fn p_string(&mut self, text: &str) -> Pattern {
    let Expr::String(lit) = self.text(text) else {
      unreachable!("text builds a string")
    };

    Pattern::Lit(PLit {
      span: self.span(),
      lit: PatLit::String(lit),
      negative: false,
    })
  }

  pub fn at<T>(&mut self, real: &Span, f: impl FnOnce(&mut Self) -> T) -> T {
    let real = self.resolve(real);
    let saved = std::mem::replace(&mut self.real, real);
    let out = f(self);

    self.real = saved;
    out
  }

  pub fn reserve(&mut self, len: usize) -> usize {
    let start = self.next;

    self.next += len + 2;
    start
  }

  pub fn map(&mut self, offset: usize, real: &Span) {
    self.expansion.real.insert(offset, real.clone());
  }

  pub fn std_ctor_span(&mut self, span: &Span) {
    self.expansion.std_ctors.insert((span.start, span.end));
  }

  pub fn std_list_span(&mut self, span: &Span) {
    self.expansion.std_lists.insert((span.start, span.end));
  }

  pub fn needs_std(&mut self, module: &str) {
    self.expansion.std_types.insert(module.to_string());
  }

  pub fn int(&mut self, value: i64) -> Expr {
    Expr::Int(crate::syntax::ast::IntLit {
      span: self.span(),
      raw: value.to_string(),
    })
  }

  pub fn std_list_pattern(&mut self, items: Vec<Pattern>) -> Pattern {
    let span = self.span();

    self.expansion.std_lists.insert((span.start, span.end));
    Pattern::List(PList { span, items, tail: None })
  }

  pub fn std_ctor_expr(&mut self, name: &str, args: Vec<Expr>) -> Expr {
    let name = self.std_ctor(name);
    let callee = Expr::Var(Var { span: name.span.clone(), name });

    if args.is_empty() {
      return callee;
    }

    self.call(callee, args)
  }
}
