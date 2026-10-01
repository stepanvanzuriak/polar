use std::collections::HashMap;

use crate::{
  backend::codegen::emit::{Colour, colour},
  core::{
    dictionaries::walk,
    ir::{
      CDecl, CExpr, CExprKind, CModule, CPattern, CtorId, CtorInfo, DeclKind,
      EffectSlot, PatternTest, Sym, SymGen,
    },
    matching::next_sym_id,
  },
  shared::{
    modules::{self, ModuleSource},
    source::Span,
  },
  stdlib,
  types::ty::{EffTail, TVar},
};

const INLINE_LIMIT: usize = 40;

#[derive(Debug, Clone, PartialEq)]
pub struct InlineBody {
  pub decl: CDecl,
  pub ctors: Vec<CtorInfo>,
}

#[must_use]
pub fn inline(mut module: CModule, modules: &[ModuleSource]) -> CModule {
  let local = module
    .decls
    .iter()
    .filter(|decl| inlinable(decl))
    .map(|decl| (decl.sym.id, decl.clone()))
    .collect();
  let mut pass = Inliner {
    local,
    external: HashMap::new(),
    modules,
    syms: SymGen::starting_at(next_sym_id(&module)),
    current: None,
    ctors: std::mem::take(&mut module.ctors),
    std_imports: std::mem::take(&mut module.std_imports),
    user_imports: &module.user_imports,
  };
  let mut decls = std::mem::take(&mut module.decls);

  for decl in &mut decls {
    pass.current = Some(decl.sym.id);
    pass.rewrite(&mut decl.body);
  }

  module.ctors = pass.ctors;
  module.std_imports = pass.std_imports;
  module.decls = decls;
  module
}

pub(crate) fn exportable(core: &CModule, path: &str) -> Vec<InlineBody> {
  let top: HashMap<u32, &CDecl> =
    core.decls.iter().map(|decl| (decl.sym.id, decl)).collect();

  core
    .decls
    .iter()
    .filter(|decl| decl.exported && inlinable(decl) && portable(decl))
    .filter_map(|decl| {
      let mut decl = decl.clone();

      globalise(&mut decl.body, &top, path)
        .then_some(InlineBody { decl, ctors: core.ctors.clone() })
    })
    .collect()
}

fn portable(decl: &CDecl) -> bool {
  let tail = tail_var(decl.effects.as_ref());
  let mut ok = true;

  walk(&decl.body, &mut |e| {
    ok &= !matches!(
      e.kind,
      CExprKind::Op { .. }
        | CExprKind::Extern { .. }
        | CExprKind::Throw { .. }
        | CExprKind::Try { .. }
        | CExprKind::Dict { .. }
        | CExprKind::Method { .. }
    );
    ok &= match e.effects.as_ref().map(|slot| slot.0.tail) {
      None | Some(EffTail::Closed) => true,
      Some(EffTail::Open(v) | EffTail::Rigid(v)) => Some(v) == tail,
      Some(EffTail::Gen(_)) => false,
    };
  });
  ok
}

fn globalise(e: &mut CExpr, top: &HashMap<u32, &CDecl>, path: &str) -> bool {
  if let CExprKind::Var(sym) = &e.kind
    && let Some(decl) = top.get(&sym.id)
  {
    if !decl.exported {
      return false;
    }

    let member = decl.sym.name.to_string();

    e.kind = match path.strip_prefix("Std.") {
      Some(module) => CExprKind::Std { module: module.to_string(), member },
      None => CExprKind::User { module: path.to_string(), member },
    };
    return true;
  }

  children(e).into_iter().all(|child| globalise(child, top, path))
}

fn inlinable(decl: &CDecl) -> bool {
  decl.kind == DeclKind::Function
    && colour(decl.effects.as_ref(), &[]) == Colour::Poly
    && tail_var(decl.effects.as_ref()).is_some()
    && !mentions(&decl.body, decl.sym.id)
    && size(&decl.body) <= INLINE_LIMIT
}

fn mentions(e: &CExpr, id: u32) -> bool {
  let mut found = false;

  walk(e, &mut |e| {
    found |= matches!(&e.kind, CExprKind::Var(s) if s.id == id);
  });
  found
}

fn size(e: &CExpr) -> usize {
  let mut n = 0;

  walk(e, &mut |_| n += 1);
  n
}

fn tail_var(effects: Option<&EffectSlot>) -> Option<TVar> {
  match effects?.0.tail {
    EffTail::Open(var) | EffTail::Rigid(var) => Some(var),
    EffTail::Closed | EffTail::Gen(_) => None,
  }
}

struct Callee {
  decl: CDecl,
  ctors: Option<Vec<CtorInfo>>,
}

struct Inliner<'m> {
  local: HashMap<u32, CDecl>,
  external: HashMap<String, Vec<InlineBody>>,
  modules: &'m [ModuleSource],
  syms: SymGen,
  current: Option<u32>,
  ctors: Vec<CtorInfo>,
  std_imports: Vec<String>,
  user_imports: &'m [(String, String)],
}

impl Inliner<'_> {
  fn rewrite(&mut self, e: &mut CExpr) {
    for child in children(e) {
      self.rewrite(child);
    }

    if let Some(inlined) = self.call(e) {
      *e = inlined;
    }
  }

  fn callee(&mut self, func: &CExpr) -> Option<Callee> {
    match &func.kind {
      CExprKind::Var(sym) if self.current != Some(sym.id) => {
        let decl = self.local.get(&sym.id)?.clone();

        Some(Callee { decl, ctors: None })
      }
      CExprKind::Std { module, member } => {
        self.external(&format!("Std.{module}"), member)
      }
      CExprKind::User { module, member } => self.external(module, member),
      _ => None,
    }
  }

  fn external(&mut self, path: &str, member: &str) -> Option<Callee> {
    if !self.external.contains_key(path) {
      let bodies = match path.strip_prefix("Std.") {
        Some(name) => stdlib::interface(name).map(|i| i.inline),
        None => self.user_bodies(path),
      };

      self.external.insert(path.to_string(), bodies.unwrap_or_default());
    }

    let body =
      self.external[path].iter().find(|b| &*b.decl.sym.name == member)?;

    Some(Callee { decl: body.decl.clone(), ctors: Some(body.ctors.clone()) })
  }

  fn user_bodies(&self, path: &str) -> Option<Vec<InlineBody>> {
    let module = self.modules.iter().find(|m| m.path == path)?;
    let file = modules::file(path);
    let (_, interface) =
      stdlib::user_interface(path, &file, &module.source, self.modules).ok()?;

    Some(interface.inline)
  }

  fn reachable(&mut self, body: &CExpr) -> bool {
    let mut std = Vec::new();
    let mut ok = true;

    walk(body, &mut |e| match &e.kind {
      CExprKind::Std { module, .. } => std.push(module.clone()),
      CExprKind::User { module, .. } => {
        ok &= self.user_imports.iter().any(|(path, _)| path == module);
      }
      _ => {}
    });

    if ok {
      for module in std {
        if !self.std_imports.contains(&module) {
          self.std_imports.push(module);
        }
      }
    }

    ok
  }

  fn call(&mut self, e: &CExpr) -> Option<CExpr> {
    let CExprKind::App { func, args } = &e.kind else { return None };
    let Callee { decl, ctors } = self.callee(func)?;

    if args.len() != decl.params.len() {
      return None;
    }

    if ctors.is_some() && !self.reachable(&decl.body) {
      return None;
    }

    let retarget = (tail_var(decl.effects.as_ref())?, e.effects.clone()?);
    let mut body = decl.body;

    body.origin.clone_from(&e.origin);

    let foreign = ctors.as_ref().map(|ctors| Foreign {
      ctors,
      table: &mut self.ctors,
      origin: e.origin.clone(),
    });
    let copy = Copy::new(&mut self.syms, Some(retarget), foreign);
    let inlined = substitute(&decl.params, args, body, e, copy);

    Some(beta(inlined, &mut self.syms))
  }
}

fn substitute(
  params: &[Sym],
  args: &[CExpr],
  mut body: CExpr,
  site: &CExpr,
  mut copy: Copy<'_>,
) -> CExpr {
  let mut lets = Vec::new();

  for (param, arg) in params.iter().zip(args) {
    if matches!(arg.kind, CExprKind::Var(_) | CExprKind::Lit(_)) {
      copy.args.insert(param.id, arg.clone());
    } else {
      let fresh = copy.syms.fresh(&param.name);

      copy.renames.insert(param.id, fresh.clone());
      lets.push((fresh, arg.clone()));
    }
  }

  copy.visit(&mut body);

  lets.into_iter().rev().fold(body, |body, (sym, value)| CExpr {
    kind: CExprKind::Let {
      sym: Some(sym),
      value: Box::new(value),
      body: Box::new(body),
    },
    origin: site.origin.clone(),
    ty: site.ty.clone(),
    effects: site.effects.clone(),
  })
}

fn beta(mut e: CExpr, syms: &mut SymGen) -> CExpr {
  let CExprKind::Let { sym: Some(sym), value, body } = &mut e.kind else {
    return e;
  };

  if let CExprKind::Lam { params, .. } = &value.kind
    && uses(sym.id, params.len(), body) == (1, true)
  {
    let CExprKind::Lam { params, body: lambda } =
      std::mem::replace(&mut value.kind, CExprKind::MatchFail)
    else {
      return e;
    };
    let mut rest = std::mem::replace(&mut **body, hole());

    paste(&mut rest, sym.id, &params, *lambda, syms);

    return beta(rest, syms);
  }

  let rest = std::mem::replace(&mut **body, hole());

  **body = beta(rest, syms);
  e
}

fn uses(id: u32, arity: usize, e: &CExpr) -> (usize, bool) {
  let mut all = 0;
  let mut called = 0;

  walk(e, &mut |e| match &e.kind {
    CExprKind::Var(s) if s.id == id => all += 1,
    CExprKind::App { func, args }
      if is_var(func, id) && args.len() == arity =>
    {
      called += 1;
    }
    _ => {}
  });

  (all, all == called)
}

fn is_var(e: &CExpr, id: u32) -> bool {
  matches!(&e.kind, CExprKind::Var(s) if s.id == id)
}

fn paste(
  e: &mut CExpr,
  id: u32,
  params: &[Sym],
  lambda: CExpr,
  syms: &mut SymGen,
) -> Option<CExpr> {
  if let CExprKind::App { func, args } = &e.kind
    && is_var(func, id)
  {
    let pasted =
      substitute(params, args, lambda, e, Copy::new(syms, None, None));

    *e = pasted;
    return None;
  }

  let mut lambda = Some(lambda);

  for child in children(e) {
    lambda = paste(child, id, params, lambda?, syms);
  }

  lambda
}

fn hole() -> CExpr {
  CExpr::new(CExprKind::MatchFail, None)
}

struct Foreign<'a> {
  ctors: &'a [CtorInfo],
  table: &'a mut Vec<CtorInfo>,
  origin: Option<Span>,
}

impl Foreign<'_> {
  fn ctor(&mut self, id: &mut CtorId) {
    let info = &self.ctors[id.0 as usize];
    let found = self
      .table
      .iter()
      .position(|c| c.name == info.name && c.owner == info.owner);
    let index = found.unwrap_or_else(|| {
      self.table.push(info.clone());
      self.table.len() - 1
    });

    *id = CtorId(u32::try_from(index).unwrap_or(u32::MAX));
  }
}

struct Copy<'a> {
  renames: HashMap<u32, Sym>,
  args: HashMap<u32, CExpr>,
  retarget: Option<(TVar, EffectSlot)>,
  foreign: Option<Foreign<'a>>,
  syms: &'a mut SymGen,
}

impl<'a> Copy<'a> {
  fn new(
    syms: &'a mut SymGen,
    retarget: Option<(TVar, EffectSlot)>,
    foreign: Option<Foreign<'a>>,
  ) -> Self {
    Self {
      renames: HashMap::new(),
      args: HashMap::new(),
      retarget,
      foreign,
      syms,
    }
  }

  fn fresh(&mut self, sym: &mut Sym) {
    let fresh = self.syms.fresh(&sym.name);

    self.renames.insert(sym.id, fresh.clone());
    *sym = fresh;
  }

  fn pattern(&mut self, pattern: &mut CPattern) {
    match pattern {
      CPattern::Wildcard | CPattern::Lit(_) => {}
      CPattern::Bind(sym) => self.fresh(sym),
      CPattern::Ctor { ctor, args } => {
        if let Some(foreign) = &mut self.foreign {
          foreign.ctor(ctor);
        }

        for arg in args {
          self.pattern(arg);
        }
      }
      CPattern::Record { fields } => {
        for (_, field) in fields {
          self.pattern(field);
        }
      }
    }
  }

  fn visit(&mut self, e: &mut CExpr) {
    if let CExprKind::Var(sym) = &mut e.kind {
      if let Some(arg) = self.args.get(&sym.id) {
        *e = arg.clone();
        return;
      }

      if let Some(fresh) = self.renames.get(&sym.id) {
        *sym = fresh.clone();
      }
    }

    if let Some((tail, site)) = &self.retarget
      && let Some(EffectSlot(row)) = &e.effects
      && matches!(row.tail, EffTail::Open(v) | EffTail::Rigid(v) if v == *tail)
    {
      e.effects = Some(site.clone());
    }

    if let Some(foreign) = &mut self.foreign {
      e.origin.clone_from(&foreign.origin);

      match &mut e.kind {
        CExprKind::Ctor { ctor, .. }
        | CExprKind::CtorFn { ctor }
        | CExprKind::Test { test: PatternTest::Tag(ctor), .. } => {
          foreign.ctor(ctor);
        }
        CExprKind::Case { arms, .. } => {
          for arm in arms {
            arm.origin.clone_from(&foreign.origin);
          }
        }
        _ => {}
      }
    }

    match &mut e.kind {
      CExprKind::Let { sym: Some(sym), .. }
      | CExprKind::Try { caught: sym, .. } => self.fresh(sym),
      CExprKind::Lam { params, .. } => {
        for param in params {
          self.fresh(param);
        }
      }
      CExprKind::Case { arms, .. } => {
        for arm in arms {
          self.pattern(&mut arm.pattern);
        }
      }
      _ => {}
    }

    for child in children(e) {
      self.visit(child);
    }
  }
}

fn children(e: &mut CExpr) -> Vec<&mut CExpr> {
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
    | CExprKind::Extern { .. } => Vec::new(),
    CExprKind::Throw { value: target, .. }
    | CExprKind::Lam { body: target, .. }
    | CExprKind::Field { target, .. }
    | CExprKind::Test { target, .. } => vec![&mut **target],
    CExprKind::Try { body: first, handler: second, .. }
    | CExprKind::Let { value: first, body: second, .. } => {
      vec![&mut **first, &mut **second]
    }
    CExprKind::App { func, args } => {
      std::iter::once(&mut **func).chain(args.iter_mut()).collect()
    }
    CExprKind::Prim { args, .. }
    | CExprKind::Ctor { args, .. }
    | CExprKind::Dict { args, .. }
    | CExprKind::Concat { parts: args } => args.iter_mut().collect(),
    CExprKind::If { cond, then_branch, else_branch } => {
      vec![&mut **cond, &mut **then_branch, &mut **else_branch]
    }
    CExprKind::Case { scrutinee, arms } => std::iter::once(&mut **scrutinee)
      .chain(arms.iter_mut().map(|arm| &mut arm.body))
      .collect(),
    CExprKind::Record { fields } => {
      fields.iter_mut().map(|(_, value)| value).collect()
    }
    CExprKind::Update { base, fields } => std::iter::once(&mut **base)
      .chain(fields.iter_mut().map(|(_, value)| value))
      .collect(),
  }
}
