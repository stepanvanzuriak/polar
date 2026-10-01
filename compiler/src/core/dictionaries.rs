use std::collections::HashMap;

use crate::{
  backend::codegen::emit::dict_name,
  check::{Types, solve::Evidence},
  core::{
    ir::{
      CArm, CBridge, CBridgeOp, CDecl, CExpr, CExprKind, CModule, DeclKind,
      EffectSlot, Sym, SymGen, TypeSlot,
    },
    matching::next_sym_id,
  },
  shared::ice::{ice, invariant},
  shared::modules::ModuleSource,
  shared::source::Span,
  types::ty::Type,
};

#[must_use]
pub fn elaborate(
  mut module: CModule,
  types: &Types,
  modules: &[ModuleSource],
) -> CModule {
  let preds: HashMap<u32, usize> = module
    .decls
    .iter()
    .map(|d| {
      let count = types.decl_preds.get(&*d.sym.name).map_or(0, Vec::len);

      (d.sym.id, count)
    })
    .collect();
  let mut pass = Elaborator {
    syms: SymGen::starting_at(next_sym_id(&module)),
    types,
    preds,
    current: Vec::new(),
  };
  let mut decls = Vec::new();

  for imp in std::mem::take(&mut module.impls) {
    let Some(origin) = imp.origin.clone() else { continue };
    let Some((trait_path, target)) =
      types.impl_heads.get(&(origin.start, origin.end))
    else {
      continue;
    };
    let name = dict_name(trait_path, target, None);
    let params: Vec<Sym> =
      (0..imp.bounds).map(|i| pass.syms.fresh(&format!("$d{i}"))).collect();

    pass.current.clone_from(&params);

    let fields = imp
      .methods
      .into_iter()
      .map(|method| {
        let body = pass.expr(method.body);
        let origin = method.origin.clone();

        (
          method.sym.name.to_string(),
          CExpr {
            effects: method.effects,
            ..CExpr::new(
              CExprKind::Lam { params: method.params, body: Box::new(body) },
              origin,
            )
          },
        )
      })
      .collect();
    let body = CExpr::new(CExprKind::Record { fields }, Some(origin.clone()));

    decls.push(CDecl {
      sym: pass.syms.fresh(&name),
      kind: if imp.bounds == 0 {
        DeclKind::Constant
      } else {
        DeclKind::Function
      },
      exported: true,
      params,
      body,
      origin: Some(origin),
      effects: Some(EffectSlot::pure()),
    });
  }

  for mut decl in std::mem::take(&mut module.decls) {
    let count = pass.preds.get(&decl.sym.id).copied().unwrap_or(0);
    let dicts: Vec<Sym> =
      (0..count).map(|i| pass.syms.fresh(&format!("$d{i}"))).collect();

    pass.current.clone_from(&dicts);

    let body = std::mem::replace(&mut decl.body, placeholder());

    decl.body = pass.expr(body);
    decl.params = dicts.into_iter().chain(decl.params).collect();
    decls.push(decl);
  }

  for bind in &mut module.binds {
    for op in &mut bind.ops {
      pass.current.clear();

      let body = std::mem::replace(&mut op.body, placeholder());

      op.body = pass.expr(body);
    }
  }

  pass.current.clear();
  module.bridges = bridges(&module, &pass);
  module.decls = decls;
  module.impls = Vec::new();

  for decl in &module.decls {
    invariant(!has_method(&decl.body), || {
      format!(
        "a `Method` survived dictionary elaboration in `{}`",
        decl.sym.name
      )
    });
  }

  imports(&mut module, modules);
  module
}

fn bridges(module: &CModule, pass: &Elaborator<'_>) -> Vec<CBridge> {
  module
    .binds
    .iter()
    .filter_map(|bind| {
      let via = bind.via.clone()?;
      let ops = pass.types.bridges.get(&bind.effect)?;
      let dict = |ev: &Evidence| pass.dict(ev, None);

      Some(CBridge {
        effect: bind.effect.clone(),
        host: bind.host.clone(),
        via,
        ops: ops
          .iter()
          .map(|op| CBridgeOp {
            op: op.op.clone(),
            params: op.params.iter().map(|p| p.as_ref().map(dict)).collect(),
            ret: op.ret.as_ref().map(dict),
            throws: op
              .throws
              .iter()
              .map(|(tag, ev)| (tag.clone(), dict(ev)))
              .collect(),
          })
          .collect(),
      })
    })
    .collect()
}

struct Elaborator<'t> {
  syms: SymGen,
  types: &'t Types,
  preds: HashMap<u32, usize>,
  current: Vec<Sym>,
}

impl Elaborator<'_> {
  fn dict(&self, evidence: &Evidence, origin: Option<&Span>) -> CExpr {
    match evidence {
      Evidence::Param(k) => match self.current.get(*k) {
        Some(sym) => CExpr::new(CExprKind::Var(sym.clone()), origin.cloned()),
        None => {
          ice(format!("dictionary parameter {k} is out of range"), origin)
        }
      },
      Evidence::Impl { trait_path, target, module, args } => CExpr::new(
        CExprKind::Dict {
          trait_path: trait_path.to_string(),
          target: target.to_string(),
          module: module.clone(),
          args: args.iter().map(|a| self.dict(a, origin)).collect(),
        },
        origin.cloned(),
      ),
    }
  }

  fn dicts(&self, e: &CExpr) -> Option<Vec<CExpr>> {
    let origin = e.origin.as_ref()?;
    let constrained = match &e.kind {
      CExprKind::Var(sym) => self.preds.get(&sym.id).is_some_and(|&n| n > 0),
      CExprKind::User { .. } | CExprKind::Std { .. } => true,
      _ => false,
    };

    if !constrained {
      return None;
    }

    let evidence = self.types.evidence_at(origin)?;

    Some(evidence.iter().map(|ev| self.dict(ev, Some(origin))).collect())
  }

  #[allow(clippy::too_many_lines, reason = "one arm per Core expression kind")]
  fn expr(&mut self, e: CExpr) -> CExpr {
    if let Some(dicts) = self.dicts(&e) {
      return self.wrap(e, dicts);
    }

    let CExpr { kind, origin, ty, effects } = e;

    let kind = match kind {
      CExprKind::Method { trait_path, method } => {
        let Some(evidence) =
          origin.as_ref().and_then(|o| self.types.evidence_at(o))
        else {
          ice(
            format!("no evidence for the method `{trait_path}.{method}`"),
            origin.as_ref(),
          );
        };

        CExprKind::Field {
          target: Box::new(self.dict(&evidence[0], origin.as_ref())),
          name: method,
        }
      }
      CExprKind::App { func, args } => {
        let args = self.all(args);

        match self.dicts(&func) {
          Some(dicts) => CExprKind::App {
            func,
            args: dicts.into_iter().chain(args).collect(),
          },
          None => CExprKind::App { func: self.boxed(*func), args },
        }
      }
      CExprKind::Prim { op, args } => {
        let args = self.all(args);
        let evidence = origin
          .as_ref()
          .and_then(|o| self.types.evidence_at(o))
          .and_then(|e| e.first())
          .filter(|_| {
            matches!(
              op,
              crate::core::ir::PrimOp::Eq | crate::core::ir::PrimOp::Ne
            )
          });

        match evidence {
          Some(evidence) => {
            let call = CExpr {
              kind: CExprKind::App {
                func: Box::new(CExpr::new(
                  CExprKind::Field {
                    target: Box::new(self.dict(evidence, origin.as_ref())),
                    name: "eq".to_string(),
                  },
                  origin.clone(),
                )),
                args,
              },
              origin: origin.clone(),
              ty: Some(TypeSlot(Type::bool())),
              effects: Some(EffectSlot::pure()),
            };

            if op == crate::core::ir::PrimOp::Ne {
              CExprKind::Prim {
                op: crate::core::ir::PrimOp::Not,
                args: vec![call],
              }
            } else {
              return call;
            }
          }
          None => CExprKind::Prim { op, args },
        }
      }
      CExprKind::Concat { parts } => {
        let parts = parts
          .into_iter()
          .map(|part| {
            let show =
              part.origin.as_ref().and_then(|o| self.types.show_at(o)).cloned();
            let part = self.expr(part);

            match show {
              Some(evidence) => {
                let origin = part.origin.clone();
                let func = CExpr::new(
                  CExprKind::Field {
                    target: Box::new(self.dict(&evidence, origin.as_ref())),
                    name: "show".to_string(),
                  },
                  origin.clone(),
                );

                CExpr {
                  kind: CExprKind::App {
                    func: Box::new(func),
                    args: vec![part],
                  },
                  origin,
                  ty: Some(TypeSlot(Type::string())),
                  effects: Some(EffectSlot::pure()),
                }
              }
              None => part,
            }
          })
          .collect();

        CExprKind::Concat { parts }
      }
      kind @ (CExprKind::Lit(_)
      | CExprKind::Var(_)
      | CExprKind::Builtin { .. }
      | CExprKind::Std { .. }
      | CExprKind::User { .. }
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
      CExprKind::Case { scrutinee, arms } => CExprKind::Case {
        scrutinee: self.boxed(*scrutinee),
        arms: arms
          .into_iter()
          .map(|arm| CArm {
            pattern: arm.pattern,
            body: self.expr(arm.body),
            origin: arm.origin,
          })
          .collect(),
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
      CExprKind::Test { test, target } => {
        CExprKind::Test { test, target: self.boxed(*target) }
      }
    };

    CExpr { kind, origin, ty, effects }
  }

  fn wrap(&mut self, func: CExpr, dicts: Vec<CExpr>) -> CExpr {
    let arity = match &func.ty {
      Some(TypeSlot(Type::Fn { params, .. })) => params.len(),
      _ => {
        ice("a constrained value without a function type", func.origin.as_ref())
      }
    };
    let origin = func.origin.clone();
    let params: Vec<Sym> =
      (0..arity).map(|i| self.syms.fresh(&format!("$a{i}"))).collect();
    let args = dicts
      .into_iter()
      .chain(
        params
          .iter()
          .map(|p| CExpr::new(CExprKind::Var(p.clone()), origin.clone())),
      )
      .collect();
    let ty = func.ty.clone();
    let effects = ty.as_ref().and_then(TypeSlot::row);
    let body = CExpr {
      effects: effects.clone(),
      ..CExpr::new(
        CExprKind::App { func: Box::new(func), args },
        origin.clone(),
      )
    };

    CExpr {
      ty,
      effects,
      ..CExpr::new(CExprKind::Lam { params, body: Box::new(body) }, origin)
    }
  }

  fn boxed(&mut self, e: CExpr) -> Box<CExpr> {
    Box::new(self.expr(e))
  }

  fn all(&mut self, es: Vec<CExpr>) -> Vec<CExpr> {
    es.into_iter().map(|e| self.expr(e)).collect()
  }

  fn fields(&mut self, fields: Vec<(String, CExpr)>) -> Vec<(String, CExpr)> {
    fields.into_iter().map(|(n, e)| (n, self.expr(e))).collect()
  }
}

fn placeholder() -> CExpr {
  CExpr::new(CExprKind::MatchFail, None)
}

fn has_method(e: &CExpr) -> bool {
  let mut found = false;

  walk(e, &mut |e| found |= matches!(e.kind, CExprKind::Method { .. }));
  found
}

pub(crate) fn walk(e: &CExpr, f: &mut dyn FnMut(&CExpr)) {
  f(e);

  match &e.kind {
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
    CExprKind::Throw { value, .. } => walk(value, f),
    CExprKind::Try { body, handler, .. } => {
      walk(body, f);
      walk(handler, f);
    }
    CExprKind::Lam { body, .. } => walk(body, f),
    CExprKind::App { func, args } => {
      walk(func, f);
      for a in args {
        walk(a, f);
      }
    }
    CExprKind::Prim { args, .. }
    | CExprKind::Ctor { args, .. }
    | CExprKind::Dict { args, .. }
    | CExprKind::Concat { parts: args } => args.iter().for_each(|a| walk(a, f)),
    CExprKind::Let { value, body, .. } => {
      walk(value, f);
      walk(body, f);
    }
    CExprKind::If { cond, then_branch, else_branch } => {
      walk(cond, f);
      walk(then_branch, f);
      walk(else_branch, f);
    }
    CExprKind::Case { scrutinee, arms } => {
      walk(scrutinee, f);
      for arm in arms {
        walk(&arm.body, f);
      }
    }
    CExprKind::Record { fields } => fields.iter().for_each(|(_, v)| walk(v, f)),
    CExprKind::Update { base, fields } => {
      walk(base, f);
      for (_, v) in fields {
        walk(v, f);
      }
    }
    CExprKind::Field { target, .. } | CExprKind::Test { target, .. } => {
      walk(target, f);
    }
  }
}

#[must_use]
pub fn inlined(e: &CExpr) -> bool {
  let CExprKind::App { func, args } = &e.kind else { return false };
  let CExprKind::Field { target, name } = &func.kind else { return false };

  args.len() == if name == "eq" { 2 } else { 1 }
    && prelude_primitive(target, name).is_some()
}

#[must_use]
pub fn prelude_primitive<'e>(dict: &'e CExpr, method: &str) -> Option<&'e str> {
  let CExprKind::Dict { trait_path, target, module, args } = &dict.kind else {
    return None;
  };
  let wanted = match method {
    "eq" => "Eq",
    "show" => "Show",
    _ => return None,
  };
  let local = trait_path.rsplit('.').next().unwrap_or(trait_path);
  let from_prelude = match module.as_deref() {
    Some("Std.Prelude") => true,
    None => trait_path == wanted,
    Some(_) => false,
  };

  (from_prelude
    && local == wanted
    && args.is_empty()
    && matches!(target.as_str(), "Int" | "Float" | "String" | "Bool"))
  .then_some(target.as_str())
}

fn imports(module: &mut CModule, modules: &[ModuleSource]) {
  let mut needed: Vec<String> = Vec::new();

  let binds = module.binds.iter().flat_map(|b| &b.ops);

  for decl in module.decls.iter().chain(binds) {
    collect(&decl.body, &mut needed);
  }

  for dict in module.bridges.iter().flat_map(bridge_dicts) {
    collect(dict, &mut needed);
  }

  for path in needed {
    if let Some(std) = path.strip_prefix("Std.") {
      if !module.std_imports.iter().any(|m| m == std) {
        module.std_imports.push(std.to_string());
      }
    } else if !module.user_imports.iter().any(|(p, _)| *p == path)
      && let Some(source) = modules.iter().find(|m| m.path == path)
    {
      module.user_imports.push((path, source.specifier.clone()));
    }
  }
}

fn collect(e: &CExpr, out: &mut Vec<String>) {
  if inlined(e) {
    let CExprKind::App { args, .. } = &e.kind else { return };

    for arg in args {
      collect(arg, out);
    }

    return;
  }

  if let CExprKind::Dict { module: Some(module), args, .. } = &e.kind {
    if !out.contains(module) {
      out.push(module.clone());
    }

    for arg in args {
      collect(arg, out);
    }

    return;
  }

  match &e.kind {
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
    CExprKind::Throw { value, .. } => collect(value, out),
    CExprKind::Try { body, handler, .. } => {
      collect(body, out);
      collect(handler, out);
    }
    CExprKind::Lam { body, .. } => collect(body, out),
    CExprKind::App { func, args } => {
      collect(func, out);
      for a in args {
        collect(a, out);
      }
    }
    CExprKind::Prim { args, .. }
    | CExprKind::Ctor { args, .. }
    | CExprKind::Dict { args, .. }
    | CExprKind::Concat { parts: args } => {
      for a in args {
        collect(a, out);
      }
    }
    CExprKind::Let { value, body, .. } => {
      collect(value, out);
      collect(body, out);
    }
    CExprKind::If { cond, then_branch, else_branch } => {
      collect(cond, out);
      collect(then_branch, out);
      collect(else_branch, out);
    }
    CExprKind::Case { scrutinee, arms } => {
      collect(scrutinee, out);
      for arm in arms {
        collect(&arm.body, out);
      }
    }
    CExprKind::Record { fields } => {
      for (_, v) in fields {
        collect(v, out);
      }
    }
    CExprKind::Update { base, fields } => {
      collect(base, out);
      for (_, v) in fields {
        collect(v, out);
      }
    }
    CExprKind::Field { target, .. } | CExprKind::Test { target, .. } => {
      collect(target, out);
    }
  }
}

#[must_use]
pub fn bridge_dicts(bridge: &CBridge) -> Vec<&CExpr> {
  bridge
    .ops
    .iter()
    .flat_map(|op| {
      op.params
        .iter()
        .flatten()
        .chain(&op.ret)
        .chain(op.throws.iter().map(|(_, d)| d))
    })
    .collect()
}
