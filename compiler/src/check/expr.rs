use std::sync::Arc;

use crate::{
  backend::codegen::builtins,
  check::{
    Checker,
    convert::TypeVars,
    effects::RowOwner,
    env::{builtin_type, signature_expr},
    hosts::Blame,
    last_expr, report,
    report::Because,
    solve::{GroupCall, Source, free_vars, trait_name},
  },
  core::lower::Resolved,
  shared::source::Span,
  syntax::ast::{
    Binary, BinaryOp, Block, Else, Expr, FieldAccess, FieldInit, If, Lambda,
    LetStmt, ListLit, Match, Name, Pattern, Pipe, RecordLit, Stmt, StringPart,
    Unary, UnaryOp,
  },
  types::{
    generalise::{
      generalise, instantiate_open, instantiate_with_preds, open_row,
    },
    print::{Printer, bare},
    ty::{Constraint, Effects, Pred, Row, Scheme, Tail, Type},
    unify::{UnifyError, unify},
  },
};

impl<'a> Checker<'a> {
  pub(crate) fn infer(&mut self, expr: &'a Expr) -> Type {
    let ty = match expr {
      Expr::Int(lit) => {
        self
          .int_literals
          .insert((lit.span.start, lit.span.end), lit.raw.clone());
        Type::int()
      }
      Expr::Float(_) => Type::float(),
      Expr::Bool(_) => Type::bool(),
      Expr::String(lit) => {
        for part in &lit.parts {
          if let StringPart::Interp(interp) = part {
            let ty = self.infer(&interp.expr);

            self.want_show(&interp.expr, &ty);
          }
        }

        Type::string()
      }
      Expr::Var(var) => self.var(&var.name, &var.span),
      Expr::Field(field) => self.field(field),
      Expr::Call(call) => {
        let args: Vec<&'a Expr> = call.args.iter().collect();

        self.apply(&call.callee, &args, &call.span)
      }
      Expr::Pipe(pipe) => self.pipe(pipe),
      Expr::Binary(binary) => self.binary(binary),
      Expr::Unary(unary) => self.unary(unary),
      Expr::Record(record) => self.record(record),
      Expr::List(list) => self.list(list),
      Expr::Lambda(lambda) => self.lambda(lambda),
      Expr::Block(block) => self.block(block),
      Expr::If(node) => self.if_expr(node),
      Expr::Match(node) => self.match_expr(node),
      Expr::Throw(t) => self.throw_expr(t),
      Expr::Try(t) => self.try_expr(t),
      Expr::Invalid(_) => self.fresh(),
    };

    self.record_expr(expr.span(), &ty);
    ty
  }

  pub(crate) fn check_expr(&mut self, expr: &'a Expr, expected: &Type) -> bool {
    let found = self.infer(expr);

    self.expect(expected, &found, expr.span())
  }

  fn want_show(&mut self, expr: &Expr, ty: &Type) {
    if self.store.resolve(ty) == Type::string() {
      return;
    }

    if let Some(path) = self.prelude_trait("Show") {
      let pred = Pred { trait_path: path, ty: ty.clone() };

      self.want(vec![pred], expr.span(), expr.span(), &Source::Interpolation);
    }
  }

  fn var(&mut self, name: &Name, key: &Span) -> Type {
    match self.resolved(&name.span) {
      Some(Resolved::Local(sym)) => match self.locals.get(&sym.id).cloned() {
        Some(scheme) => self.instantiate(&scheme),
        None => self.fresh(),
      },
      Some(Resolved::Top(sym)) => {
        if let (Some(&callee), Some(owner)) =
          (self.group_members.get(&sym.id), self.owner)
        {
          self.group_calls.push(GroupCall { key: key.clone(), callee, owner });
        }

        match self.locals.get(&sym.id).cloned() {
          Some(scheme) => {
            self.instantiate_wanting(&scheme, key, &name.span, &name.text)
          }
          None => self.fresh(),
        }
      }
      Some(Resolved::Method { trait_path, method }) => {
        self.method(&trait_path, &method, key, &name.span)
      }
      Some(Resolved::Ctor(id)) => match self.ctor_scheme(id) {
        Some(scheme) => instantiate_open(&mut self.store, &scheme).0,
        None => self.fresh(),
      },
      Some(Resolved::Extern(name)) => self.extern_type(&name, key),
      Some(
        Resolved::List { .. }
        | Resolved::StdList { .. }
        | Resolved::Builtin { .. }
        | Resolved::Imported { .. }
        | Resolved::Trait(_)
        | Resolved::Operation { .. },
      )
      | None => self.fresh(),
    }
  }

  fn method(
    &mut self,
    trait_path: &str,
    method: &str,
    key: &Span,
    blame: &Span,
  ) -> Type {
    let path = self.trait_path(trait_path);
    let scheme = self.env.traits.get(&path).and_then(|t| {
      t.methods.iter().find(|(name, _)| name == method).map(|(_, s)| s.clone())
    });
    let Some(scheme) = scheme else { return self.fresh() };
    let (ty, preds) = instantiate_with_preds(&mut self.store, &scheme);
    let ty = self.opened(key, ty);

    self.want(preds, key, blame, &Source::Method);
    ty
  }

  fn instantiate_wanting(
    &mut self,
    scheme: &Scheme,
    key: &Span,
    blame: &Span,
    name: &str,
  ) -> Type {
    let (ty, preds) = instantiate_with_preds(&mut self.store, scheme);
    let ty = self.opened(key, ty);

    if !preds.is_empty() {
      let mut printer = Printer::new(&[&scheme.ty], false);

      printer.print(&scheme.ty);

      let requirement = scheme
        .preds
        .iter()
        .map(|p| {
          format!("{}<{}>", trait_name(&p.trait_path), printer.print(&p.ty))
        })
        .collect::<Vec<_>>()
        .join(", ");
      let source = Source::Function { name: name.to_string(), requirement };

      self.want(preds, key, blame, &source);
    }

    ty
  }

  fn field(&mut self, field: &'a FieldAccess) -> Type {
    match self.resolved(&field.field.span) {
      Some(Resolved::Builtin { module, member }) => {
        match builtins::lookup(module, member) {
          Some(info) if builtins::std_owner(module).is_some() => {
            let scope = std::mem::replace(&mut self.vars, TypeVars::flexible());
            let ty = self.convert(&signature_expr(info));

            self.vars = scope;
            open_row(&mut self.store, ty)
          }
          Some(info) => open_row(&mut self.store, builtin_type(info)),
          None => self.fresh(),
        }
      }
      Some(Resolved::Operation { effect, op }) => {
        self.operation_type(&effect, &op)
      }
      Some(Resolved::Imported { module, member }) => {
        self.imported_wanting(&module, &member, &field.span, &field.field.span)
      }
      Some(Resolved::Method { trait_path, method }) => {
        self.method(&trait_path, &method, &field.span, &field.field.span)
      }
      _ => {
        let target = self.infer(&field.target);
        let t = self.fresh();
        let r = self.fresh_var();
        let expected = Type::Record(Row::new(
          vec![(Arc::from(field.field.text.as_str()), t.clone())],
          Tail::Open(r),
        ));

        if self.expect_because(
          &expected,
          &target,
          &field.field.span,
          Because::Access,
        ) {
          t
        } else {
          self.fresh()
        }
      }
    }
  }

  fn imported_wanting(
    &mut self,
    module: &str,
    member: &str,
    key: &Span,
    blame: &Span,
  ) -> Type {
    let scheme = self
      .interfaces
      .get(module)
      .and_then(|interface| interface.scheme(member))
      .cloned();
    let short = format!("{}.{member}", bare(module));

    match scheme {
      Some(scheme) => self.instantiate_wanting(&scheme, key, blame, &short),
      None => self.fresh(),
    }
  }

  fn pipe(&mut self, pipe: &'a Pipe) -> Type {
    let ty = match &*pipe.right {
      Expr::Call(call) => {
        let mut args: Vec<&'a Expr> = vec![&pipe.left];

        args.extend(call.args.iter());
        self.apply(&call.callee, &args, &pipe.span)
      }
      other => self.apply(other, &[&pipe.left], &pipe.span),
    };

    self.record_expr(pipe.right.span(), &ty);
    ty
  }

  pub(crate) fn apply(
    &mut self,
    callee: &'a Expr,
    args: &[&'a Expr],
    span: &Span,
  ) -> Type {
    let func = self.infer(callee);

    match self.store.resolve(&func) {
      Type::Fn { params, ret, effects } => {
        if params.len() != args.len() {
          for arg in args {
            self.infer(arg);
          }

          let diagnostic = report::argument_count(
            params.len(),
            args.len(),
            &func,
            span,
            &self.store,
          );

          self.push(diagnostic);
          return self.fresh();
        }

        let mut ok = true;

        for (i, (arg, param)) in args.iter().zip(&params).enumerate() {
          let found = self.infer(arg);

          if ok {
            let because = self.parameter_because(callee, i);
            let saved = self.callee.replace(callee_name(callee));

            ok = self.expect_because(param, &found, arg.span(), because);
            self.callee = saved;
          }
        }

        self.perform_at(&effects, span, callee.span());
        *ret
      }
      Type::Var(_) => {
        let params: Vec<Type> = args.iter().map(|a| self.infer(a)).collect();
        let ret = self.fresh();
        let row = Effects::open(self.fresh_var());
        let wanted = Type::func_with(params, ret.clone(), row.clone());

        if self.expect(&func, &wanted, span) {
          self.perform_at(&row, span, callee.span());
          ret
        } else {
          self.fresh()
        }
      }
      other => {
        for arg in args {
          self.infer(arg);
        }

        let diagnostic =
          report::not_a_function(&other, callee.span(), &self.store);

        self.push(diagnostic);
        self.fresh()
      }
    }
  }

  fn parameter_because(&self, callee: &Expr, index: usize) -> Because {
    let Expr::Var(var) = callee else { return Because::Nothing };
    let Some(Resolved::Top(sym)) = self.resolutions.get(&var.name.span) else {
      return Because::Nothing;
    };

    self
      .top_fns
      .get(&sym.id)
      .and_then(|f| f.params.get(index))
      .filter(|p| p.ty.is_some())
      .map_or(Because::Nothing, |p| Because::Parameter(p.span.clone()))
  }

  fn binary(&mut self, binary: &'a Binary) -> Type {
    match binary.op {
      BinaryOp::Add
      | BinaryOp::Sub
      | BinaryOp::Mul
      | BinaryOp::Div
      | BinaryOp::Rem => {
        let n = self.fresh_constrained(Constraint::Num);

        if self.operands(binary, &n) { n } else { self.fresh() }
      }
      BinaryOp::Lt | BinaryOp::LtEq | BinaryOp::Gt | BinaryOp::GtEq => {
        let o = self.fresh_constrained(Constraint::Ord);

        self.operands(binary, &o);
        Type::bool()
      }
      BinaryOp::Eq | BinaryOp::NotEq => {
        let t = self.fresh();

        if self.operands(binary, &t)
          && let Some(path) = self.prelude_trait("Eq")
        {
          let pred = Pred { trait_path: path, ty: t };

          self.want(
            vec![pred],
            &binary.span,
            &binary.op_span,
            &Source::Operator,
          );
        }

        Type::bool()
      }
      BinaryOp::And | BinaryOp::Or => {
        self.operands(binary, &Type::bool());
        Type::bool()
      }
      BinaryOp::BitAnd | BinaryOp::BitOr | BinaryOp::BitXor => {
        self.operands(binary, &Type::int());
        Type::int()
      }
    }
  }

  fn operands(&mut self, binary: &'a Binary, expected: &Type) -> bool {
    if self.check_expr(&binary.left, expected) {
      self.check_expr(&binary.right, expected)
    } else {
      self.infer(&binary.right);
      false
    }
  }

  fn unary(&mut self, unary: &'a Unary) -> Type {
    match unary.op {
      UnaryOp::Negate => {
        let n = self.fresh_constrained(Constraint::Num);

        if self.check_expr(&unary.operand, &n) { n } else { self.fresh() }
      }
      UnaryOp::Not => {
        self.check_expr(&unary.operand, &Type::bool());
        Type::bool()
      }
      UnaryOp::BitNot => {
        self.check_expr(&unary.operand, &Type::int());
        Type::int()
      }
    }
  }

  fn record(&mut self, record: &'a RecordLit) -> Type {
    let names: Vec<&Name> = record.fields.iter().map(|f| &f.name).collect();

    self.duplicate_fields(&names);

    let fields = self.field_values(&record.fields);

    match &record.spread {
      None => {
        Type::Record(Row::new(fields, Tail::Closed(self.store.fresh_brand())))
      }
      Some(base) => self.update(base, fields),
    }
  }

  fn field_values(&mut self, inits: &'a [FieldInit]) -> Vec<(Arc<str>, Type)> {
    let mut fields: Vec<(Arc<str>, Type)> = Vec::new();

    for init in inits {
      let ty = self.infer(&init.value);

      if !fields.iter().any(|(n, _)| **n == *init.name.text) {
        fields.push((Arc::from(init.name.text.as_str()), ty));
      }
    }

    fields
  }

  fn update(&mut self, base: &'a Expr, updates: Vec<(Arc<str>, Type)>) -> Type {
    let base_ty = self.infer(base);

    match self.store.zonk(&base_ty) {
      Type::Record(row) => {
        let mut fields = row.fields.clone();
        let mut tail = row.tail;
        let mut keeps_brand = true;

        for (name, ty) in updates {
          if let Some(slot) = fields.iter_mut().find(|(n, _)| *n == name) {
            keeps_brand &= self.store.zonk(&slot.1) == self.store.zonk(&ty);
            slot.1 = ty;
            continue;
          }

          keeps_brand = false;

          if let Tail::Open(r) = tail {
            let field = self.fresh();
            let r2 = self.fresh_var();
            let rest = Type::Record(Row::new(Vec::new(), Tail::Open(r)));
            let wanted = Type::Record(Row::new(
              vec![(name.clone(), field)],
              Tail::Open(r2),
            ));

            if unify(&mut self.store, &rest, &wanted).is_ok() {
              tail =
                self.store.zonk_row(&Row::new(Vec::new(), Tail::Open(r))).tail;
            }
          }

          fields.push((name, ty));
        }

        if !keeps_brand && tail.is_closed() {
          tail = Tail::anonymous();
        }

        Type::Record(Row::new(fields, tail))
      }
      Type::Var(_) => {
        let r = self.fresh_var();
        let wanted_fields: Vec<(Arc<str>, Type)> =
          updates.iter().map(|(n, _)| (n.clone(), self.fresh())).collect();
        let wanted = Type::Record(Row::new(wanted_fields, Tail::Open(r)));

        if self.expect(&wanted, &base_ty, base.span()) {
          Type::Record(Row::new(updates, Tail::Open(r)))
        } else {
          self.fresh()
        }
      }
      other => {
        let r = self.fresh_var();
        let wanted = Type::Record(Row::new(Vec::new(), Tail::Open(r)));
        let error = UnifyError::Mismatch {
          expected: wanted.clone(),
          found: other.clone(),
        };
        let diagnostic = report::Report {
          error: &error,
          expected: &wanted,
          found: &other,
          span: base.span().clone(),
          because: Because::Nothing,
          int_literal: None,
        }
        .build(&self.store);

        self.push(diagnostic);
        self.fresh()
      }
    }
  }

  fn list(&mut self, list: &'a ListLit) -> Type {
    let (elem, list_ty) = self.list_types(&list.span);

    for item in &list.items {
      self.check_expr(item, &elem);
    }

    if let Some(tail) = &list.tail {
      self.check_expr(tail, &list_ty);
    }

    list_ty
  }

  pub(crate) fn std_list(&self, elem: Type) -> Type {
    let name = if self.qualifier.is_none()
      && self.module_name.as_deref() == Some("List")
    {
      "List"
    } else {
      "Std.List.List"
    };

    Type::Con { name: Arc::from(name), args: vec![elem] }
  }

  pub(crate) fn list_types(&mut self, span: &Span) -> (Type, Type) {
    if let Some(Resolved::StdList { .. }) = self.resolved(span) {
      let elem = self.fresh();
      let list = self.std_list(elem.clone());

      return (elem, list);
    }

    let cons = match self.resolved(span) {
      Some(Resolved::List { cons, .. }) => self.ctor_scheme(cons),
      _ => None,
    };

    if let Some(scheme) = cons
      && let Type::Fn { params, ret, .. } = self.instantiate(&scheme)
      && let [elem, _] = params.as_slice()
    {
      return (elem.clone(), *ret);
    }

    (self.fresh(), self.fresh())
  }

  fn lambda(&mut self, lambda: &'a Lambda) -> Type {
    let mut params = Vec::with_capacity(lambda.params.len());

    for param in &lambda.params {
      let ty = match &param.ty {
        Some(ty) => self.convert(ty),
        None => self.fresh(),
      };

      self.bind_mono(&param.name.span, ty.clone());
      params.push(ty);
    }

    let row = match &lambda.return_type {
      Some(_) => self.convert_effects(lambda.effects.as_ref()),
      None => Effects::open(self.fresh_var()),
    };
    let owner = match &lambda.return_type {
      Some(_) => RowOwner::Lambda {
        row: lambda.effects.as_ref().map(|r| r.span.clone()),
      },
      None => RowOwner::Free,
    };
    let (body, row) = self.with_row(row, owner, |c| c.block(&lambda.body));
    let uses = std::mem::take(&mut self.last_uses);
    let ret = match &lambda.return_type {
      Some(annotation) => {
        let ret = self.convert(annotation);
        let blame = last_expr(&lambda.body.result).span();

        self.expect_because(
          &ret,
          &body,
          blame,
          Because::Annotation(annotation.span().clone()),
        );
        ret
      }
      None => body,
    };
    let row = self.store.zonk_effects(&row);
    let at = lambda.span.start..lambda.span.start + "function".len();
    let blame = Blame {
      what: "this function".to_string(),
      at: crate::shared::source::Span::new(
        lambda.span.file.clone(),
        at.start,
        at.end,
      ),
      row: lambda.effects.as_ref().map(|r| r.span.clone()),
      uses: &uses,
    };

    self.check_hosts(&row, &blame);

    Type::func_with(params, ret, row)
  }

  pub(crate) fn block(&mut self, block: &'a Block) -> Type {
    for stmt in &block.stmts {
      match stmt {
        Stmt::Let(stmt) => self.let_stmt(stmt),
        Stmt::Expr(stmt) => {
          self.infer(&stmt.expr);
        }
      }
    }

    self.infer(&block.result)
  }

  fn let_stmt(&mut self, stmt: &'a LetStmt) {
    match &stmt.pattern {
      Pattern::Var(var) => {
        let start = self.wanted.len();

        self.store.enter_level();

        let ty = self.let_value(stmt);

        self.store.leave_level();

        let level = self.store.current_level();

        for i in start..self.wanted.len() {
          let pred_ty = self.store.zonk(&self.wanted[i].pred.ty);

          for v in free_vars(&pred_ty) {
            self.store.lower(v, level);
          }
        }

        let scheme = generalise(&self.store, &ty);

        if let Some(sym) = self.local_sym(&var.name.span) {
          self.locals.insert(sym.id, scheme);
        }
      }
      pattern => {
        let ty = self.let_value(stmt);

        self.check_pattern(pattern, &ty);
        self.matches.push(super::exhaustive::Site::Let(stmt));
      }
    }
  }

  fn let_value(&mut self, stmt: &'a LetStmt) -> Type {
    let found = self.infer(&stmt.value);

    match &stmt.ty {
      Some(annotation) => {
        let expected = self.convert(annotation);

        self.expect_because(
          &expected,
          &found,
          stmt.value.span(),
          Because::Annotation(annotation.span().clone()),
        );
        expected
      }
      None => found,
    }
  }

  fn if_expr(&mut self, node: &'a If) -> Type {
    self.check_expr(&node.cond, &Type::bool());

    let then_ty = self.block(&node.then_branch);
    let first = last_expr(&node.then_branch.result).span().clone();

    let (else_ty, blame) = match &*node.else_branch {
      Else::Block(block) => {
        (self.block(block), last_expr(&block.result).span().clone())
      }
      Else::If(inner) => {
        let ty = self.if_expr(inner);

        self.record_expr(&inner.span, &ty);
        (ty, last_expr(&inner.then_branch.result).span().clone())
      }
    };

    let because = Because::Branch(first, then_ty.clone());

    self.expect_because(&then_ty, &else_ty, &blame, because);
    then_ty
  }

  fn match_expr(&mut self, node: &'a Match) -> Type {
    let scrutinee = self.infer(&node.scrutinee);
    let mut result: Option<(Type, Span)> = None;

    for arm in &node.arms {
      self.check_pattern(&arm.pattern, &scrutinee);

      let body = self.infer(&arm.body);
      let blame = last_expr(&arm.body).span().clone();

      match &result {
        None => result = Some((body, blame)),
        Some((expected, first)) => {
          let because = Because::Branch(first.clone(), expected.clone());
          let expected = expected.clone();

          self.expect_because(&expected, &body, &blame, because);
        }
      }
    }

    self.matches.push(super::exhaustive::Site::Match(node));

    match result {
      Some((ty, _)) => ty,
      None => self.fresh(),
    }
  }
}

fn callee_name(callee: &Expr) -> String {
  match callee {
    Expr::Var(var) => var.name.text.clone(),
    Expr::Field(field) => match &*field.target {
      Expr::Var(var) => format!("{}.{}", var.name.text, field.field.text),
      _ => field.field.text.clone(),
    },
    _ => "the function".to_string(),
  }
}
