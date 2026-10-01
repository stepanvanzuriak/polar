use crate::{
  check::{Types, effects::error_tag},
  core::ir::{
    CArm, CDecl, CExpr, CExprKind, CModule, DeclKind, EffectSlot, TypeSlot,
  },
  shared::ice::ice,
  types::ty::Type,
};

#[must_use]
pub fn annotate(mut module: CModule, types: &Types) -> CModule {
  for decl in &mut module.decls {
    signature(decl, types);
    expr(&mut decl.body, types);
  }

  for imp in &mut module.impls {
    for method in &mut imp.methods {
      signature(method, types);
      expr(&mut method.body, types);
    }
  }

  for bind in &mut module.binds {
    for op in &mut bind.ops {
      expr(&mut op.body, types);
    }
  }

  module
}

fn signature(decl: &mut CDecl, types: &Types) {
  decl.effects = match decl.kind {
    DeclKind::Constant => Some(EffectSlot::pure()),
    DeclKind::Function => decl
      .origin
      .as_ref()
      .and_then(|o| types.signatures.get(&(o.start, o.end)))
      .and_then(|ty| TypeSlot(ty.clone()).row()),
  };
}

fn lookup(e: &CExpr, types: &Types) -> Option<Type> {
  match &e.kind {
    CExprKind::Let { body, .. } => lookup(body, types),
    CExprKind::Case { arms, .. } if arms.len() == 1 => {
      lookup(&arms[0].body, types)
    }
    _ => e.origin.as_ref().and_then(|o| types.expr(o)).cloned(),
  }
}

fn expr(e: &mut CExpr, types: &Types) {
  if let Some(ty) = e.origin.as_ref().and_then(|o| types.expr(o)) {
    e.ty = Some(TypeSlot(ty.clone()));
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
    CExprKind::Throw { value, tag } => {
      if let Some(path) =
        e.origin.as_ref().and_then(|o| types.throws.get(&(o.start, o.end)))
      {
        *tag = error_tag(path);
      }

      expr(value, types);
    }
    CExprKind::Try { body, handler, handles, .. } => {
      if let Some(paths) =
        e.origin.as_ref().and_then(|o| types.handles.get(&(o.start, o.end)))
      {
        *handles = paths.iter().map(|p| error_tag(p)).collect();
      }

      expr(body, types);
      expr(handler, types);
    }
    CExprKind::Dict { args, .. } => all(args, types),
    CExprKind::Lam { body, .. } => expr(body, types),
    CExprKind::App { func, args } => {
      expr(func, types);
      all(args, types);
    }
    CExprKind::Prim { args, .. } => {
      all(args, types);

      for arg in args.iter_mut() {
        match lookup(arg, types) {
          Some(ty) => arg.ty = Some(TypeSlot(ty)),
          None => {
            ice("an operand of a primitive has no type", arg.origin.as_ref())
          }
        }
      }
    }
    CExprKind::Let { value, body, .. } => {
      expr(value, types);
      expr(body, types);
    }
    CExprKind::If { cond, then_branch, else_branch } => {
      expr(cond, types);
      expr(then_branch, types);
      expr(else_branch, types);
    }
    CExprKind::Case { scrutinee, arms } => {
      expr(scrutinee, types);

      for CArm { body, .. } in arms {
        expr(body, types);
      }
    }
    CExprKind::Record { fields } => {
      for (_, value) in fields {
        expr(value, types);
      }
    }
    CExprKind::Update { base, fields } => {
      expr(base, types);

      for (_, value) in fields {
        expr(value, types);
      }
    }
    CExprKind::Field { target, .. } | CExprKind::Test { target, .. } => {
      expr(target, types);
    }
    CExprKind::Ctor { args, .. } | CExprKind::Concat { parts: args } => {
      all(args, types);
    }
  }

  e.effects = match &e.kind {
    CExprKind::Lam { .. } => e.ty.as_ref().and_then(TypeSlot::row),
    CExprKind::App { func, .. } => func
      .origin
      .as_ref()
      .and_then(|o| types.closed_rows.get(&(o.start, o.end)))
      .map(|row| EffectSlot(row.clone()))
      .or_else(|| func.ty.as_ref().and_then(TypeSlot::row)),
    _ => e.effects.take(),
  };
}

fn all(es: &mut [CExpr], types: &Types) {
  for e in es {
    expr(e, types);
  }
}
