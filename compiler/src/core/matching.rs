use crate::core::ir::{
  CArm, CDecl, CExpr, CExprKind, CModule, CPattern, CtorId, CtorInfo,
  PatternTest, PrimOp, Sym, SymGen,
};
use crate::shared::source::Span;

#[must_use]
pub fn compile_matches(mut module: CModule) -> CModule {
  let mut pass = Matcher {
    syms: SymGen::starting_at(next_sym_id(&module)),
    ctors: module.ctors.clone(),
  };

  for decl in &mut module.decls {
    let body = std::mem::replace(&mut decl.body, placeholder());

    decl.body = pass.expr(body);
  }

  for imp in &mut module.impls {
    for decl in &mut imp.methods {
      let body = std::mem::replace(&mut decl.body, placeholder());

      decl.body = pass.expr(body);
    }
  }

  for bind in &mut module.binds {
    for decl in &mut bind.ops {
      let body = std::mem::replace(&mut decl.body, placeholder());

      decl.body = pass.expr(body);
    }
  }

  module
}

struct Matcher {
  syms: SymGen,
  ctors: Vec<CtorInfo>,
}

type Path = Vec<String>;

#[derive(Default)]
struct Plan {
  tests: Vec<(Path, PatternTest)>,
  binds: Vec<(Path, Sym)>,
}

impl Matcher {
  fn expr(&mut self, expr: CExpr) -> CExpr {
    let CExpr { kind, origin, ty, effects } = expr;

    let kind = match kind {
      CExprKind::Case { scrutinee, arms } => {
        let scrutinee = self.expr(*scrutinee);

        return self.case(scrutinee, arms, origin);
      }
      kind @ (CExprKind::Lit(_)
      | CExprKind::Var(_)
      | CExprKind::Builtin { .. }
      | CExprKind::Std { .. }
      | CExprKind::User { .. }
      | CExprKind::Method { .. }
      | CExprKind::CtorFn { .. }
      | CExprKind::MatchFail
      | CExprKind::Op { .. }
      | CExprKind::Extern { .. }) => kind,
      CExprKind::Throw { value, tag } => {
        CExprKind::Throw { value: self.boxed(*value), tag }
      }
      CExprKind::Try { body, caught, handler, handles } => CExprKind::Try {
        body: self.boxed(*body),
        caught,
        handler: self.boxed(*handler),
        handles,
      },
      CExprKind::Dict { trait_path, target, module, args } => {
        CExprKind::Dict { trait_path, target, module, args: self.all(args) }
      }
      CExprKind::Lam { params, body } => {
        CExprKind::Lam { params, body: self.boxed(*body) }
      }
      CExprKind::App { func, args } => {
        CExprKind::App { func: self.boxed(*func), args: self.all(args) }
      }
      CExprKind::Prim { op, args } => {
        CExprKind::Prim { op, args: self.all(args) }
      }
      CExprKind::Let { sym, value, body } => CExprKind::Let {
        sym,
        value: self.boxed(*value),
        body: self.boxed(*body),
      },
      CExprKind::If { cond, then_branch, else_branch } => CExprKind::If {
        cond: self.boxed(*cond),
        then_branch: self.boxed(*then_branch),
        else_branch: self.boxed(*else_branch),
      },
      CExprKind::Record { fields } => {
        CExprKind::Record { fields: self.fields(fields) }
      }
      CExprKind::Update { base, fields } => CExprKind::Update {
        base: self.boxed(*base),
        fields: self.fields(fields),
      },
      CExprKind::Field { target, name } => {
        CExprKind::Field { target: self.boxed(*target), name }
      }
      CExprKind::Ctor { ctor, args } => {
        CExprKind::Ctor { ctor, args: self.all(args) }
      }
      CExprKind::Concat { parts } => {
        CExprKind::Concat { parts: self.all(parts) }
      }
      CExprKind::Test { test, target } => {
        CExprKind::Test { test, target: self.boxed(*target) }
      }
    };

    CExpr { kind, origin, ty, effects }
  }

  fn boxed(&mut self, expr: CExpr) -> Box<CExpr> {
    Box::new(self.expr(expr))
  }

  fn all(&mut self, exprs: Vec<CExpr>) -> Vec<CExpr> {
    exprs.into_iter().map(|e| self.expr(e)).collect()
  }

  fn fields(&mut self, fields: Vec<(String, CExpr)>) -> Vec<(String, CExpr)> {
    fields.into_iter().map(|(name, e)| (name, self.expr(e))).collect()
  }

  fn case(
    &mut self,
    scrutinee: CExpr,
    arms: Vec<CArm>,
    origin: Option<Span>,
  ) -> CExpr {
    let (root, bound) = if let CExprKind::Var(sym) = &scrutinee.kind {
      (sym.clone(), None)
    } else {
      (self.syms.fresh("$s"), Some(scrutinee))
    };

    let mut chain = CExpr::new(CExprKind::MatchFail, origin.clone());
    let planned: Vec<(Plan, CExpr, Option<Span>)> = arms
      .into_iter()
      .map(|arm| {
        let mut plan = Plan::default();

        plan_pattern(arm.pattern, &mut Vec::new(), &mut plan);
        (plan, arm.body, arm.origin)
      })
      .collect();
    let last = planned.len().saturating_sub(1);
    let last_is_implied = self.last_is_implied(&planned);

    for (i, (mut plan, body, arm_origin)) in
      planned.into_iter().enumerate().rev()
    {
      let body = self.expr(body);
      let then_branch =
        self.bind(&root, &plan.binds, body, arm_origin.as_ref());

      if last_is_implied && i == last {
        plan.tests.clear();
      }

      chain = match conjoin(&root, plan.tests, arm_origin.as_ref()) {
        None => then_branch,
        Some(cond) => CExpr::new(
          CExprKind::If {
            cond: Box::new(cond),
            then_branch: Box::new(then_branch),
            else_branch: Box::new(chain),
          },
          arm_origin.clone(),
        ),
      };
    }

    match bound {
      None => chain,
      Some(value) => CExpr::new(
        CExprKind::Let {
          sym: Some(root),
          value: Box::new(value),
          body: Box::new(chain),
        },
        origin,
      ),
    }
  }

  fn last_is_implied(&self, planned: &[(Plan, CExpr, Option<Span>)]) -> bool {
    let root_tag = |plan: &Plan| match plan.tests.as_slice() {
      [(path, PatternTest::Tag(ctor))] if path.is_empty() => Some(*ctor),
      _ => None,
    };
    let Some(tags) = planned
      .iter()
      .map(|(plan, _, _)| root_tag(plan))
      .collect::<Option<Vec<CtorId>>>()
    else {
      return false;
    };
    let Some(last) = tags.last() else { return false };
    let owner = &self.ctors[last.0 as usize].owner;

    self
      .ctors
      .iter()
      .zip(0u32..)
      .filter(|(info, _)| info.owner == *owner)
      .all(|(_, id)| tags.contains(&CtorId(id)))
  }

  fn bind(
    &mut self,
    root: &Sym,
    binds: &[(Path, Sym)],
    body: CExpr,
    origin: Option<&Span>,
  ) -> CExpr {
    let mut prefixes: Vec<(&[String], usize)> = Vec::new();

    for (path, _) in binds {
      for len in 1..path.len() {
        let prefix = &path[..len];

        match prefixes.iter_mut().find(|(p, _)| *p == prefix) {
          Some((_, uses)) => *uses += 1,
          None => prefixes.push((prefix, 1)),
        }
      }
    }

    let mut shared: Vec<&[String]> = prefixes
      .into_iter()
      .filter(|(_, uses)| *uses > 1)
      .map(|(prefix, _)| prefix)
      .collect();

    shared.sort_by_key(|prefix| prefix.len());

    let mut temps: Vec<(&[String], Sym)> = Vec::new();
    let mut lets: Vec<(Sym, CExpr)> = Vec::new();

    for prefix in shared {
      let temp = self.syms.fresh("$t");

      lets.push((temp.clone(), project(root, &temps, prefix, origin)));
      temps.push((prefix, temp));
    }

    for (path, sym) in binds {
      lets.push((sym.clone(), project(root, &temps, path, origin)));
    }

    lets.into_iter().rev().fold(body, |body, (sym, value)| {
      CExpr::new(
        CExprKind::Let {
          sym: Some(sym),
          value: Box::new(value),
          body: Box::new(body),
        },
        origin.cloned(),
      )
    })
  }
}

fn plan_pattern(pattern: CPattern, path: &mut Path, plan: &mut Plan) {
  match pattern {
    CPattern::Wildcard => {}
    CPattern::Bind(sym) => plan.binds.push((path.clone(), sym)),
    CPattern::Lit(lit) => {
      plan.tests.push((path.clone(), PatternTest::Lit(lit)));
    }
    CPattern::Ctor { ctor, args } => {
      plan.tests.push((path.clone(), PatternTest::Tag(ctor)));

      for (i, arg) in args.into_iter().enumerate() {
        path.push(format!("_{i}"));
        plan_pattern(arg, path, plan);
        path.pop();
      }
    }
    CPattern::Record { fields } => {
      for (name, field) in fields {
        path.push(name);
        plan_pattern(field, path, plan);
        path.pop();
      }
    }
  }
}

fn conjoin(
  root: &Sym,
  tests: Vec<(Path, PatternTest)>,
  origin: Option<&Span>,
) -> Option<CExpr> {
  tests
    .into_iter()
    .map(|(path, test)| {
      CExpr::new(
        CExprKind::Test {
          test,
          target: Box::new(project(root, &[], &path, origin)),
        },
        origin.cloned(),
      )
    })
    .reduce(|lhs, rhs| {
      CExpr::new(
        CExprKind::Prim { op: PrimOp::And, args: vec![lhs, rhs] },
        origin.cloned(),
      )
    })
}

fn project(
  root: &Sym,
  temps: &[(&[String], Sym)],
  path: &[String],
  origin: Option<&Span>,
) -> CExpr {
  let (start, sym) = temps
    .iter()
    .filter(|(prefix, _)| prefix.len() < path.len() && path.starts_with(prefix))
    .max_by_key(|(prefix, _)| prefix.len())
    .map_or((0, root), |(prefix, temp)| (prefix.len(), temp));

  path[start..].iter().fold(
    CExpr::new(CExprKind::Var(sym.clone()), origin.cloned()),
    |target, name| {
      CExpr::new(
        CExprKind::Field { target: Box::new(target), name: name.clone() },
        origin.cloned(),
      )
    },
  )
}

fn placeholder() -> CExpr {
  CExpr::new(CExprKind::MatchFail, None)
}

#[must_use]
pub fn next_sym_id(module: &CModule) -> u32 {
  let mut max = 0;

  let impls = module.impls.iter().flat_map(|imp| &imp.methods);
  let binds = module.binds.iter().flat_map(|bind| &bind.ops);

  for CDecl { sym: s, params, body, .. } in
    module.decls.iter().chain(impls).chain(binds)
  {
    sym_max(s, &mut max);
    for param in params {
      sym_max(param, &mut max);
    }
    expr_max(body, &mut max);
  }

  max
}

fn sym_max(s: &Sym, max: &mut u32) {
  *max = (*max).max(s.id + 1);
}

fn pattern_max(p: &CPattern, max: &mut u32) {
  match p {
    CPattern::Wildcard | CPattern::Lit(_) => {}
    CPattern::Bind(s) => sym_max(s, max),
    CPattern::Ctor { args, .. } => {
      for arg in args {
        pattern_max(arg, max);
      }
    }
    CPattern::Record { fields } => {
      for (_, field) in fields {
        pattern_max(field, max);
      }
    }
  }
}

fn expr_max(e: &CExpr, max: &mut u32) {
  match &e.kind {
    CExprKind::Lit(_)
    | CExprKind::Builtin { .. }
    | CExprKind::Std { .. }
    | CExprKind::User { .. }
    | CExprKind::Method { .. }
    | CExprKind::CtorFn { .. }
    | CExprKind::MatchFail
    | CExprKind::Op { .. }
    | CExprKind::Extern { .. } => {}
    CExprKind::Var(s) => sym_max(s, max),
    CExprKind::Throw { value, .. } => expr_max(value, max),
    CExprKind::Try { body, caught, handler, .. } => {
      sym_max(caught, max);
      expr_max(body, max);
      expr_max(handler, max);
    }
    CExprKind::Lam { params, body } => {
      for param in params {
        sym_max(param, max);
      }
      expr_max(body, max);
    }
    CExprKind::App { func, args } => {
      expr_max(func, max);
      for arg in args {
        expr_max(arg, max);
      }
    }
    CExprKind::Prim { args, .. }
    | CExprKind::Ctor { args, .. }
    | CExprKind::Dict { args, .. }
    | CExprKind::Concat { parts: args } => {
      for arg in args {
        expr_max(arg, max);
      }
    }
    CExprKind::Let { sym: s, value, body } => {
      if let Some(s) = s {
        sym_max(s, max);
      }
      expr_max(value, max);
      expr_max(body, max);
    }
    CExprKind::If { cond, then_branch, else_branch } => {
      expr_max(cond, max);
      expr_max(then_branch, max);
      expr_max(else_branch, max);
    }
    CExprKind::Case { scrutinee, arms } => {
      expr_max(scrutinee, max);
      for arm in arms {
        pattern_max(&arm.pattern, max);
        expr_max(&arm.body, max);
      }
    }
    CExprKind::Record { fields } => {
      for (_, value) in fields {
        expr_max(value, max);
      }
    }
    CExprKind::Update { base, fields } => {
      expr_max(base, max);
      for (_, value) in fields {
        expr_max(value, max);
      }
    }
    CExprKind::Field { target, .. } | CExprKind::Test { target, .. } => {
      expr_max(target, max);
    }
  }
}
