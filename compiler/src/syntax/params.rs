use crate::{
  shared::source::Span,
  syntax::ast::{
    Block, Decl, Else, Expr, FnDecl, LetStmt, Module, Name, Param, Stmt,
    StringPart, Var,
  },
};

pub const PREFIX: &str = "$arg";

pub fn destructure(module: &mut Module) {
  for zone in &mut module.zones {
    for decl in &mut zone.decls {
      match decl {
        Decl::Fn(f) => function(f),
        Decl::Impl(imp) => imp.methods.iter_mut().for_each(function),
        Decl::Bind(bind) => bind.ops.iter_mut().for_each(function),
        Decl::Const(c) => expr(&mut c.value),
        _ => {}
      }
    }
  }
}

#[must_use]
pub fn is_synthetic(name: &str) -> bool {
  name.starts_with(PREFIX)
}

fn function(f: &mut FnDecl) {
  block(&mut f.body);
  bind(&f.params, &mut f.body);
}

fn bind(params: &[Param], body: &mut Block) {
  let lets: Vec<Stmt> = params
    .iter()
    .filter_map(|param| {
      let pattern = param.pattern.as_ref()?;
      let span = pattern.span().clone();
      let at = Span::empty(span.file.clone(), span.start);
      let name = Name { text: param.name.text.clone(), span: at.clone() };

      Some(Stmt::Let(LetStmt {
        span,
        pattern: pattern.clone(),
        ty: None,
        value: Expr::Var(Var { span: at, name }),
      }))
    })
    .collect();

  body.stmts.splice(0..0, lets);
}

fn block(b: &mut Block) {
  for stmt in &mut b.stmts {
    match stmt {
      Stmt::Let(l) => expr(&mut l.value),
      Stmt::Expr(e) => expr(&mut e.expr),
    }
  }
  expr(&mut b.result);
}

fn expr(e: &mut Expr) {
  match e {
    Expr::Int(_)
    | Expr::Float(_)
    | Expr::Bool(_)
    | Expr::Var(_)
    | Expr::Invalid(_) => {}
    Expr::String(s) => {
      for part in &mut s.parts {
        if let StringPart::Interp(interp) = part {
          expr(&mut interp.expr);
        }
      }
    }
    Expr::Field(f) => expr(&mut f.target),
    Expr::Call(c) => {
      expr(&mut c.callee);
      c.args.iter_mut().for_each(expr);
    }
    Expr::Pipe(p) => {
      expr(&mut p.left);
      expr(&mut p.right);
    }
    Expr::Binary(b) => {
      expr(&mut b.left);
      expr(&mut b.right);
    }
    Expr::Unary(u) => expr(&mut u.operand),
    Expr::Record(r) => {
      if let Some(spread) = &mut r.spread {
        expr(spread);
      }
      r.fields.iter_mut().for_each(|f| expr(&mut f.value));
    }
    Expr::List(l) => {
      l.items.iter_mut().for_each(expr);
      if let Some(tail) = &mut l.tail {
        expr(tail);
      }
    }
    Expr::Lambda(l) => {
      block(&mut l.body);
      bind(&l.params, &mut l.body);
    }
    Expr::Block(b) => block(b),
    Expr::If(i) => if_expr(i),
    Expr::Match(m) => {
      m.subjects.iter_mut().for_each(expr);
      m.arms.iter_mut().for_each(arm);
    }
    Expr::Return(r) => expr(&mut r.value),
    Expr::Throw(t) => expr(&mut t.value),
    Expr::Try(t) => {
      block(&mut t.body);
      t.arms.iter_mut().for_each(arm);
    }
  }
}

fn arm(a: &mut crate::syntax::ast::MatchArm) {
  if let Some(guard) = &mut a.guard {
    expr(guard);
  }

  expr(&mut a.body);
}

fn if_expr(i: &mut crate::syntax::ast::If) {
  expr(&mut i.cond);
  block(&mut i.then_branch);
  match i.else_branch.as_deref_mut() {
    Some(Else::Block(b)) => block(b),
    Some(Else::If(inner)) => if_expr(inner),
    None => {}
  }
}
