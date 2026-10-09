use crate::core::ir::{
  CArm, CExpr, CExprKind, CModule, CPattern, CtorId, CtorInfo, DeclKind, Lit,
  PatternTest, Sym,
};
use std::fmt::Write as _;

#[must_use]
pub fn dump_module(module: &CModule) -> String {
  Dump { ctors: &module.ctors, ids: true }.module(module)
}

#[must_use]
pub fn dump_module_shape(module: &CModule) -> String {
  Dump { ctors: &module.ctors, ids: false }.module(module)
}

#[must_use]
pub fn dump_expr(expr: &CExpr, ctors: &[CtorInfo]) -> String {
  let mut out = String::new();

  Dump { ctors, ids: true }.expr(&mut out, expr);

  out
}

#[must_use]
pub fn dump_expr_shape(expr: &CExpr, ctors: &[CtorInfo]) -> String {
  let mut out = String::new();

  Dump { ctors, ids: false }.expr(&mut out, expr);

  out
}

struct Dump<'a> {
  ctors: &'a [CtorInfo],
  ids: bool,
}

impl Dump<'_> {
  fn module(&self, module: &CModule) -> String {
    let mut out = String::from("(module");

    if !module.ctors.is_empty() {
      out.push_str("\n  (ctors");

      for ctor in &module.ctors {
        let _ = write!(out, " ({} {} {})", ctor.name, ctor.arity, ctor.owner);
      }

      out.push(')');
    }

    for imp in &module.impls {
      let _ = write!(out, "\n  (impl {} {}", imp.trait_path, imp.target);

      if imp.bounds > 0 {
        let _ = write!(out, " (bounds {})", imp.bounds);
      }

      for method in &imp.methods {
        out.push_str("\n    (method ");
        self.sym(&mut out, &method.sym);
        out.push_str(" (");
        self.syms(&mut out, &method.params);
        out.push_str(") ");
        self.expr(&mut out, &method.body);
        out.push(')');
      }

      out.push(')');
    }

    for ext in &module.externs {
      let _ = write!(
        out,
        "\n  (extern {} {:?} {})",
        ext.name, ext.module, ext.export
      );
    }

    for bind in &module.binds {
      let _ = write!(out, "\n  (bind {} {}", bind.effect, bind.host);

      for op in &bind.ops {
        out.push_str("\n    (op ");
        self.sym(&mut out, &op.sym);
        out.push_str(" (");
        self.syms(&mut out, &op.params);
        out.push_str(") ");
        self.expr(&mut out, &op.body);
        out.push(')');
      }

      out.push(')');
    }

    for decl in &module.decls {
      let constant = decl.kind == DeclKind::Constant;

      out.push_str(if constant { "\n  (const " } else { "\n  (fn " });
      self.sym(&mut out, &decl.sym);

      if decl.exported {
        out.push_str(" export");
      }

      if constant {
        out.push(' ');
      } else {
        out.push_str(" (");
        self.syms(&mut out, &decl.params);
        out.push_str(") ");
      }

      self.expr(&mut out, &decl.body);
      out.push(')');
    }

    out.push_str(")\n");
    out
  }

  #[allow(clippy::too_many_lines, reason = "one arm per Core expression kind")]
  fn expr(&self, out: &mut String, expr: &CExpr) {
    match &expr.kind {
      CExprKind::Lit(lit) => {
        out.push_str("(lit ");
        lit_text(out, lit);
        out.push(')');
      }
      CExprKind::Var(sym) => {
        out.push_str("(var ");
        self.sym(out, sym);
        out.push(')');
      }
      CExprKind::Builtin { module, member } => {
        let _ = write!(out, "(builtin {module} {member})");
      }
      CExprKind::Std { module, member } => {
        let _ = write!(out, "(std {module} {member})");
      }
      CExprKind::User { module, member } => {
        let _ = write!(out, "(user {module} {member})");
      }
      CExprKind::Method { trait_path, method } => {
        let _ = write!(out, "(method {trait_path} {method})");
      }
      CExprKind::Dict { trait_path, target, module, args } => {
        let _ = write!(out, "(dict {trait_path} {target}");

        if let Some(module) = module {
          let _ = write!(out, " from {module}");
        }

        self.list(out, args);
        out.push(')');
      }
      CExprKind::Lam { params, body } => {
        out.push_str("(lam (");
        self.syms(out, params);
        out.push_str(") ");
        self.expr(out, body);
        out.push(')');
      }
      CExprKind::App { func, args } => {
        out.push_str("(app ");
        self.expr(out, func);
        self.list(out, args);
        out.push(')');
      }
      CExprKind::Prim { op, args } => {
        let _ = write!(out, "(prim {}", op.as_str());
        self.list(out, args);
        out.push(')');
      }
      CExprKind::Let { sym, value, body } => {
        out.push_str("(let ");

        match sym {
          Some(sym) => self.sym(out, sym),
          None => out.push('_'),
        }

        out.push(' ');
        self.expr(out, value);
        out.push(' ');
        self.expr(out, body);
        out.push(')');
      }
      CExprKind::If { cond, then_branch, else_branch } => {
        out.push_str("(if ");
        self.expr(out, cond);
        out.push(' ');
        self.expr(out, then_branch);
        out.push(' ');
        self.expr(out, else_branch);
        out.push(')');
      }
      CExprKind::Case { scrutinee, arms } => {
        out.push_str("(case ");
        self.expr(out, scrutinee);

        for arm in arms {
          out.push(' ');
          self.arm(out, arm);
        }

        out.push(')');
      }
      CExprKind::Record { fields } => {
        out.push_str("(record");
        self.fields(out, fields);
        out.push(')');
      }
      CExprKind::Update { base, fields } => {
        out.push_str("(update ");
        self.expr(out, base);
        self.fields(out, fields);
        out.push(')');
      }
      CExprKind::Field { target, name } => {
        out.push_str("(field ");
        self.expr(out, target);
        let _ = write!(out, " {name})");
      }
      CExprKind::Ctor { ctor, args } => {
        let _ = write!(out, "(ctor {}", self.ctor_name(*ctor));
        self.list(out, args);
        out.push(')');
      }
      CExprKind::CtorFn { ctor } => {
        let _ = write!(out, "(ctorfn {})", self.ctor_name(*ctor));
      }
      CExprKind::Concat { parts } => {
        out.push_str("(concat");
        self.list(out, parts);
        out.push(')');
      }
      CExprKind::Test { test, target } => self.test(out, test, target),
      CExprKind::MatchFail => out.push_str("(match-fail)"),
      CExprKind::Op { effect, op } => {
        let _ = write!(out, "(op {effect} {op})");
      }
      CExprKind::Extern { name } => {
        let _ = write!(out, "(extern {name})");
      }
      CExprKind::Throw { value, .. } => {
        out.push_str("(throw ");
        self.expr(out, value);
        out.push(')');
      }
      CExprKind::Return { value } => {
        out.push_str("(return ");
        self.expr(out, value);
        out.push(')');
      }
      CExprKind::Try { body, caught, handler, .. } => {
        out.push_str("(try ");
        self.expr(out, body);
        out.push_str(" (catch ");
        self.sym(out, caught);
        out.push(' ');
        self.expr(out, handler);
        out.push_str("))");
      }
    }
  }

  fn test(&self, out: &mut String, test: &PatternTest, target: &CExpr) {
    out.push_str("(test ");

    match test {
      PatternTest::Tag(ctor) => {
        let _ = write!(out, "(tag {})", self.ctor_name(*ctor));
      }
      PatternTest::Lit(lit) => {
        out.push_str("(lit ");
        lit_text(out, lit);
        out.push(')');
      }
    }

    out.push(' ');
    self.expr(out, target);
    out.push(')');
  }

  fn arm(&self, out: &mut String, arm: &CArm) {
    out.push_str("(arm ");
    self.pattern(out, &arm.pattern);
    out.push(' ');

    if let Some(guard) = &arm.guard {
      out.push_str("(if ");
      self.expr(out, guard);
      out.push_str(") ");
    }

    self.expr(out, &arm.body);
    out.push(')');
  }

  fn pattern(&self, out: &mut String, pattern: &CPattern) {
    match pattern {
      CPattern::Wildcard => out.push('_'),
      CPattern::Bind(sym) => self.sym(out, sym),
      CPattern::Lit(lit) => lit_text(out, lit),
      CPattern::Ctor { ctor, args } => {
        let _ = write!(out, "({}", self.ctor_name(*ctor));

        for arg in args {
          out.push(' ');
          self.pattern(out, arg);
        }

        out.push(')');
      }
      CPattern::Record { fields } => {
        out.push_str("(record");

        for (name, pattern) in fields {
          let _ = write!(out, " ({name} ");
          self.pattern(out, pattern);
          out.push(')');
        }

        out.push(')');
      }
      CPattern::Or(alternatives) => {
        out.push_str("(or");

        for alternative in alternatives {
          out.push(' ');
          self.pattern(out, alternative);
        }

        out.push(')');
      }
    }
  }

  fn list(&self, out: &mut String, exprs: &[CExpr]) {
    for expr in exprs {
      out.push(' ');
      self.expr(out, expr);
    }
  }

  fn fields(&self, out: &mut String, fields: &[(String, CExpr)]) {
    for (name, value) in fields {
      let _ = write!(out, " ({name} ");
      self.expr(out, value);
      out.push(')');
    }
  }

  fn syms(&self, out: &mut String, syms: &[Sym]) {
    for (i, sym) in syms.iter().enumerate() {
      if i > 0 {
        out.push(' ');
      }

      self.sym(out, sym);
    }
  }

  fn sym(&self, out: &mut String, sym: &Sym) {
    out.push_str(&sym.name);

    if self.ids {
      let _ = write!(out, "/{}", sym.id);
    }
  }

  fn ctor_name(&self, id: CtorId) -> &str {
    self.ctors.get(id.0 as usize).map_or("?", |ctor| ctor.name.as_str())
  }
}

fn lit_text(out: &mut String, lit: &Lit) {
  let _ = match lit {
    Lit::Number(n) => write!(out, "{n}"),
    Lit::String(s) => write!(out, "{s:?}"),
    Lit::Bool(b) => write!(out, "{b}"),
  };
}
