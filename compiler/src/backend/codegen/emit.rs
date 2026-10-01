use std::collections::{HashMap, HashSet};

use crate::{
  CompileOptions,
  backend::js::ast::{
    BinaryOp, Expr, FunctionDecl, Ident, ImportNamed, ImportNamespace, Item,
    LogicalOp, Program, Stmt, UnaryOp, VarDecl, VarKind, array, arrow,
    arrow_block, assign, await_, binary, boolean, call, conditional, const_,
    continue_, expr_stmt, ident, if_, let_, logical, member, null, number,
    object, prop, ret, spread, string, template, throw, try_, unary, var,
    while_,
  },
  backend::js::names::js_ident,
  core::{
    dictionaries::{inlined, prelude_primitive, walk},
    ir::{
      Boundary, CBridge, CDecl, CExpr, CExprKind, CModule, CtorId, CtorInfo,
      DeclKind, EffectSlot, Lit, PatternTest, PrimOp, Sym, TypeSlot,
    },
  },
  shared::ice::ice,
  shared::source::{SourceFile, Span},
  types::ty::{EffTail, TVar, Type},
};

const RT: &str = "$rt";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Prim {
  Int,
  Float,
  String,
  Bool,
  Other,
}

fn prim_of(expr: &CExpr) -> Prim {
  match &expr.ty {
    Some(TypeSlot(Type::Con { name, args })) if args.is_empty() => {
      match &**name {
        "Int" => Prim::Int,
        "Float" => Prim::Float,
        "String" => Prim::String,
        "Bool" => Prim::Bool,
        _ => Prim::Other,
      }
    }
    _ => Prim::Other,
  }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Colour {
  Sync,
  Async,
  Poly,
}

pub(crate) fn colour(
  effects: Option<&EffectSlot>,
  pure_vars: &[TVar],
) -> Colour {
  let Some(EffectSlot(row)) = effects else {
    return Colour::Async;
  };

  if row.labels.iter().any(|label| !label.is_free()) {
    return Colour::Async;
  }

  match row.tail {
    EffTail::Closed => Colour::Sync,
    EffTail::Open(var) | EffTail::Rigid(var) if pure_vars.contains(&var) => {
      Colour::Sync
    }
    EffTail::Open(_) | EffTail::Rigid(_) | EffTail::Gen(_) => Colour::Poly,
  }
}

fn tail_var(effects: Option<&EffectSlot>) -> Option<TVar> {
  match effects?.0.tail {
    EffTail::Open(var) | EffTail::Rigid(var) => Some(var),
    EffTail::Closed | EffTail::Gen(_) => None,
  }
}

fn version(name: &str, colour: Colour) -> String {
  match colour {
    Colour::Sync => format!("{name}$sync"),
    Colour::Async | Colour::Poly => format!("{name}$async"),
  }
}

#[derive(Debug, Clone)]
enum Dest {
  Return,
  Assign(Ident),
  Declare(Ident),
  Discard,
}

#[must_use]
pub fn emit(
  module: &CModule,
  file: &SourceFile,
  options: &CompileOptions,
) -> Program {
  let (module, hoisted) = &hoist(module);
  let colours: HashMap<u32, Option<Colour>> = module
    .decls
    .iter()
    .filter(|d| d.kind == DeclKind::Function)
    .map(|d| {
      let colour = d.effects.as_ref().map(|row| colour(Some(row), &[]));

      (d.sym.id, colour)
    })
    .collect();
  let top: HashMap<u32, String> = module
    .decls
    .iter()
    .map(|decl| (decl.sym.id, decl.sym.name.to_string()))
    .chain(hoisted.iter().map(|(sym, _)| (sym.id, sym.name.to_string())))
    .collect();

  let mut items = import_items(module, options);

  items.extend(extern_items(module));

  let mut roots: HashSet<String> = HashSet::new();
  let mut private: HashMap<String, HashSet<String>> = HashMap::new();

  for bind in module.binds.iter().filter(|b| b.via.is_none()) {
    let props = bind
      .ops
      .iter()
      .map(|op| {
        let mut emitter =
          Emitter::new(&module.ctors, file, &top, &module.bind_refs, &colours);
        let value = emitter
          .arrow_fn(&op.params, &op.body, (true, true))
          .at(op.origin.clone());

        roots.extend(emitter.names.refs);
        prop(&op.sym.name, value)
      })
      .collect();

    items.push(Item::Stmt(Stmt::Var(VarDecl {
      kind: VarKind::Const,
      name: ident(&bind_name(&bind.effect)).at(bind.origin.clone()),
      init: Some(object(props)),
      exported: true,
      origin: bind.origin.clone(),
    })));
  }

  let dictionaries = module
    .decls
    .iter()
    .take_while(|d| {
      d.sym.name.starts_with('$') && !d.sym.name.starts_with("$recipe$")
    })
    .count();

  for (i, decl) in module.decls.iter().enumerate() {
    if i == dictionaries {
      items.extend(hoisted_items(
        hoisted,
        &module.ctors,
        file,
        (&top, &colours),
        &mut roots,
      ));
    }

    let mut emitter =
      Emitter::new(&module.ctors, file, &top, &module.bind_refs, &colours);
    let prunable =
      !decl.exported && colours.get(&decl.sym.id) == Some(&Some(Colour::Poly));

    match decl.kind {
      DeclKind::Function => {
        for (f, refs) in emitter.decl(decl) {
          if prunable {
            private.insert(f.name.name.clone(), refs);
          } else {
            roots.extend(refs);
          }

          items.push(Item::Function(f));
        }
      }
      DeclKind::Constant => {
        items.push(Item::Stmt(emitter.constant(decl)));
        roots.extend(emitter.names.refs);
      }
    }
  }

  if dictionaries == module.decls.len() {
    items.extend(hoisted_items(
      hoisted,
      &module.ctors,
      file,
      (&top, &colours),
      &mut roots,
    ));
  }

  items.extend(bridge_items(module, file, (&top, &colours), &mut roots));

  let reached = reachable(roots, &private);

  items.retain(|item| match item {
    Item::Function(f) => {
      !private.contains_key(&f.name.name) || reached.contains(&f.name.name)
    }
    _ => true,
  });

  Program { items }
}

fn bridge_items(
  module: &CModule,
  file: &SourceFile,
  (top, colours): (&HashMap<u32, String>, &HashMap<u32, Option<Colour>>),
  roots: &mut HashSet<String>,
) -> Vec<Item> {
  let host = module.host.as_deref();
  let mut served: HashSet<&str> = HashSet::new();
  let mut out = Vec::new();

  for bridge in &module.bridges {
    let mut emitter =
      Emitter::new(&module.ctors, file, top, &module.bind_refs, colours);
    let item = if host == Some(bridge.host.as_str()) {
      emitter.bridge_client(bridge)
    } else if host == Some(bridge.via.as_str())
      && served.insert(bridge.effect.as_str())
    {
      emitter.bridge_table(bridge)
    } else {
      continue;
    };

    roots.extend(emitter.names.refs);
    out.push(item);
  }

  out
}

fn reachable(
  roots: HashSet<String>,
  private: &HashMap<String, HashSet<String>>,
) -> HashSet<String> {
  let mut reached = HashSet::new();
  let mut todo: Vec<String> = roots.into_iter().collect();

  while let Some(name) = todo.pop() {
    if let Some(refs) = private.get(&name)
      && reached.insert(name)
    {
      todo.extend(refs.iter().cloned());
    }
  }

  reached
}

fn import_items(module: &CModule, options: &CompileOptions) -> Vec<Item> {
  let mut items = vec![Item::ImportNamespace(ImportNamespace {
    local: ident(RT),
    source: options.runtime.clone(),
    origin: None,
  })];

  for name in &module.std_imports {
    items.push(Item::ImportNamespace(ImportNamespace {
      local: ident(&std_namespace(name)),
      source: crate::stdlib::specifier(&options.runtime, name),
      origin: None,
    }));
  }

  for (path, specifier) in &module.user_imports {
    items.push(Item::ImportNamespace(ImportNamespace {
      local: ident(&user_namespace(path)),
      source: specifier.clone(),
      origin: None,
    }));
  }

  items
}

fn extern_items(module: &CModule) -> Vec<Item> {
  let mut items = Vec::new();
  let mut specifiers: Vec<&str> = Vec::new();
  let values = extern_values(module);
  let wrapped: HashSet<&str> = module
    .externs
    .iter()
    .filter(|e| {
      values.contains(e.name.as_str())
        || e.ret != Boundary::Plain
        || e.params.iter().any(|p| *p != Boundary::Plain)
    })
    .map(|e| e.name.as_str())
    .collect();

  for ext in &module.externs {
    if !specifiers.contains(&ext.module.as_str()) {
      specifiers.push(&ext.module);
    }
  }

  for specifier in specifiers {
    let names = module
      .externs
      .iter()
      .filter(|e| e.module == specifier)
      .map(|e| {
        let local = if wrapped.contains(e.name.as_str()) {
          raw_extern_name(&e.name)
        } else {
          extern_name(&e.name)
        };

        (e.export.clone(), ident(&local).at(e.origin.clone()))
      })
      .collect();

    items.push(Item::ImportNamed(ImportNamed {
      names,
      source: specifier.to_string(),
      origin: None,
    }));
  }

  for ext in module.externs.iter().filter(|e| wrapped.contains(e.name.as_str()))
  {
    let init = call(
      builtin_path("extern"),
      vec![
        var(&raw_extern_name(&ext.name)),
        array(ext.params.iter().map(shape).collect()),
        shape(&ext.ret),
        string(&ext.label),
      ],
      ext.origin.clone(),
    );

    items.push(Item::Stmt(const_(ident(&extern_name(&ext.name)), init)));
  }

  items
}

fn extern_values(module: &CModule) -> HashSet<String> {
  let mut uses: HashMap<String, (usize, usize)> = HashMap::new();
  let bodies = module
    .decls
    .iter()
    .map(|d| &d.body)
    .chain(module.binds.iter().flat_map(|b| b.ops.iter().map(|op| &op.body)));

  for body in bodies {
    walk(body, &mut |e| match &e.kind {
      CExprKind::Extern { name } => {
        uses.entry(name.clone()).or_default().0 += 1;
      }
      CExprKind::App { func, .. } => {
        if let CExprKind::Extern { name } = &func.kind {
          uses.entry(name.clone()).or_default().1 += 1;
        }
      }
      _ => {}
    });
  }

  uses
    .into_iter()
    .filter(|(_, (all, called))| all > called)
    .map(|(name, _)| name)
    .collect()
}

fn shape(boundary: &Boundary) -> Expr {
  match boundary {
    Boundary::Plain => null(),
    Boundary::Unit => string("unit"),
    Boundary::Option(inner) => array(vec![string("option"), shape(inner)]),
    Boundary::List(inner) => array(vec![string("list"), shape(inner)]),
    Boundary::Result(ok, err) => {
      array(vec![string("result"), shape(ok), shape(err)])
    }
    Boundary::Record(fields) => {
      object(fields.iter().map(|(name, b)| prop(name, shape(b))).collect())
    }
  }
}

fn bridge_name(effect: &str) -> String {
  format!("$bridge${effect}")
}

fn bind_name(effect: &str) -> String {
  format!("$bind${effect}")
}

fn extern_name(name: &str) -> String {
  format!("$ext${name}")
}

fn raw_extern_name(name: &str) -> String {
  format!("$js${name}")
}

fn hoisted_items(
  hoisted: &[(Sym, CExpr)],
  ctors: &[CtorInfo],
  file: &SourceFile,
  (top, colours): (&HashMap<u32, String>, &HashMap<u32, Option<Colour>>),
  roots: &mut HashSet<String>,
) -> Vec<Item> {
  hoisted
    .iter()
    .map(|(sym, dict)| {
      let mut emitter = Emitter::new(ctors, file, top, &[], colours);
      let mut none = Vec::new();
      let init = emitter.expr(dict, &mut none);

      roots.extend(std::mem::take(&mut emitter.names.refs));

      Item::Stmt(Stmt::Var(VarDecl {
        kind: VarKind::Const,
        name: ident(&sym.name),
        init: Some(init),
        exported: false,
        origin: None,
      }))
    })
    .collect()
}

fn hoist(module: &CModule) -> (CModule, Vec<(Sym, CExpr)>) {
  let mut module = module.clone();
  let mut syms = crate::core::ir::SymGen::starting_at(
    crate::core::matching::next_sym_id(&module),
  );
  let mut hoisted: Vec<(String, Sym, CExpr)> = Vec::new();

  for decl in &mut module.decls {
    hoist_in(&mut decl.body, &mut syms, &mut hoisted);
  }

  for bind in &mut module.binds {
    for op in &mut bind.ops {
      hoist_in(&mut op.body, &mut syms, &mut hoisted);
    }
  }

  let consts = hoisted.into_iter().map(|(_, sym, e)| (sym, e)).collect();

  (module, consts)
}

fn dict_key(e: &CExpr) -> Option<String> {
  let CExprKind::Dict { trait_path, target, module, args } = &e.kind else {
    return None;
  };
  let args: Vec<String> = args.iter().map(dict_key).collect::<Option<_>>()?;

  Some(format!("{trait_path}|{target}|{module:?}({})", args.join(",")))
}

fn hoist_in(
  e: &mut CExpr,
  syms: &mut crate::core::ir::SymGen,
  hoisted: &mut Vec<(String, Sym, CExpr)>,
) {
  if let CExprKind::Dict { args, .. } = &e.kind
    && !args.is_empty()
    && let Some(key) = dict_key(e)
  {
    let sym =
      if let Some((_, sym, _)) = hoisted.iter().find(|(k, _, _)| *k == key) {
        sym.clone()
      } else {
        let sym = syms.fresh(&format!("$dict{}", hoisted.len()));

        hoisted.push((key, sym.clone(), e.clone()));
        sym
      };

    e.kind = CExprKind::Var(sym);
    return;
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
    CExprKind::Throw { value, .. } => hoist_in(value, syms, hoisted),
    CExprKind::Try { body, handler, .. } => {
      hoist_in(body, syms, hoisted);
      hoist_in(handler, syms, hoisted);
    }
    CExprKind::Lam { body, .. } => hoist_in(body, syms, hoisted),
    CExprKind::App { func, args } => {
      hoist_in(func, syms, hoisted);
      for a in args {
        hoist_in(a, syms, hoisted);
      }
    }
    CExprKind::Prim { args, .. }
    | CExprKind::Ctor { args, .. }
    | CExprKind::Dict { args, .. }
    | CExprKind::Concat { parts: args } => {
      for a in args {
        hoist_in(a, syms, hoisted);
      }
    }
    CExprKind::Let { value, body, .. } => {
      hoist_in(value, syms, hoisted);
      hoist_in(body, syms, hoisted);
    }
    CExprKind::If { cond, then_branch, else_branch } => {
      hoist_in(cond, syms, hoisted);
      hoist_in(then_branch, syms, hoisted);
      hoist_in(else_branch, syms, hoisted);
    }
    CExprKind::Case { scrutinee, arms } => {
      hoist_in(scrutinee, syms, hoisted);
      for arm in arms {
        hoist_in(&mut arm.body, syms, hoisted);
      }
    }
    CExprKind::Record { fields } => {
      for (_, v) in fields {
        hoist_in(v, syms, hoisted);
      }
    }
    CExprKind::Update { base, fields } => {
      hoist_in(base, syms, hoisted);
      for (_, v) in fields {
        hoist_in(v, syms, hoisted);
      }
    }
    CExprKind::Field { target, .. } | CExprKind::Test { target, .. } => {
      hoist_in(target, syms, hoisted);
    }
  }
}

struct Emitter<'a> {
  ctors: &'a [CtorInfo],
  file: &'a SourceFile,
  names: Names<'a>,
  binds: &'a [(String, Option<String>)],
  colours: &'a HashMap<u32, Option<Colour>>,
  pure_vars: Vec<TVar>,
  awaits: usize,
  tail: Option<Loop>,
  in_bind: bool,
}

#[derive(Clone)]
struct Loop {
  sym: u32,
  params: Vec<Sym>,
  targets: Vec<Ident>,
  fresh: bool,
}

impl<'a> Emitter<'a> {
  fn new(
    ctors: &'a [CtorInfo],
    file: &'a SourceFile,
    top: &'a HashMap<u32, String>,
    binds: &'a [(String, Option<String>)],
    colours: &'a HashMap<u32, Option<Colour>>,
  ) -> Self {
    Self {
      ctors,
      file,
      names: Names::new(top),
      binds,
      colours,
      pure_vars: Vec::new(),
      awaits: 0,
      tail: None,
      in_bind: false,
    }
  }

  fn decl(&mut self, decl: &CDecl) -> Vec<(FunctionDecl, HashSet<String>)> {
    let name: &str = &decl.sym.name;

    match colour(decl.effects.as_ref(), &self.pure_vars) {
      Colour::Sync => {
        let f = self.function(decl, name, false);

        vec![(f, std::mem::take(&mut self.names.refs))]
      }
      Colour::Async => {
        let f = self.function(decl, name, true);

        vec![(f, std::mem::take(&mut self.names.refs))]
      }
      Colour::Poly => {
        let saved = self.names.clone();

        self.pure_vars.extend(tail_var(decl.effects.as_ref()));

        let sync_name = version(name, Colour::Sync);
        let async_name = version(name, Colour::Async);
        let sync = self.function(decl, &sync_name, false);
        let sync_refs = std::mem::take(&mut self.names.refs);

        self.pure_vars.pop();
        self.names = saved.clone();

        let asynchronous = self.function(decl, &async_name, true);
        let async_refs = std::mem::take(&mut self.names.refs);

        self.names = saved;

        let dispatcher = self.dispatcher(decl);

        vec![
          (sync, sync_refs),
          (asynchronous, async_refs),
          (dispatcher, HashSet::from([sync_name, async_name])),
        ]
      }
    }
  }

  fn function(
    &mut self,
    decl: &CDecl,
    name: &str,
    is_async: bool,
  ) -> FunctionDecl {
    let params: Vec<Ident> =
      decl.params.iter().map(|p| self.names.bind(p)).collect();
    let mut body = Vec::new();
    let outer = std::mem::replace(&mut self.awaits, 0);

    if tail_calls(&decl.body, decl.sym.id, decl.params.len()) {
      let fresh = has_lambda(&decl.body);

      if fresh {
        for (p, target) in decl.params.iter().zip(&params) {
          let inner = self.names.bind(p);

          body.push(const_(inner, Expr::Ident(target.clone())));
        }
      }

      let saved = self.tail.replace(Loop {
        sym: decl.sym.id,
        params: decl.params.clone(),
        targets: params.clone(),
        fresh,
      });

      self.stmt(&decl.body, &Dest::Return, &mut body);
      self.tail = saved;
      body = vec![while_(boolean(true), body)];
    } else {
      self.stmt(&decl.body, &Dest::Return, &mut body);
    }

    let awaited = std::mem::replace(&mut self.awaits, outer) > 0;

    if awaited && !is_async {
      synchronous_awaits(&decl.sym.name, decl.origin.as_ref());
    }

    FunctionDecl {
      name: ident(name).at(decl.origin.clone()),
      params,
      body,
      is_async,
      exported: decl.exported,
      origin: decl.origin.clone(),
    }
  }

  fn dispatcher(&mut self, decl: &CDecl) -> FunctionDecl {
    let name: &str = &decl.sym.name;
    let params: Vec<Ident> =
      decl.params.iter().map(|p| self.names.bind(p)).collect();
    let flag = self.names.fresh_ident("$a");
    let args: Vec<Expr> = params.iter().cloned().map(Expr::Ident).collect();
    let chosen = conditional(
      binary(
        BinaryOp::StrictEq,
        Expr::Ident(flag.clone()),
        builtin_path("ASYNC"),
      ),
      call(var(&version(name, Colour::Async)), args.clone(), None),
      call(var(&version(name, Colour::Sync)), args, None),
    );

    FunctionDecl {
      name: ident(name).at(decl.origin.clone()),
      params: params.into_iter().chain(std::iter::once(flag)).collect(),
      body: vec![ret(chosen)],
      is_async: false,
      exported: decl.exported,
      origin: decl.origin.clone(),
    }
  }

  fn suspend(&mut self, value: Expr, origin: Option<Span>) -> Expr {
    self.awaits += 1;
    await_(value).at(origin)
  }

  fn constant(&mut self, decl: &CDecl) -> Stmt {
    let init = if is_plain(&decl.body) {
      let mut none = Vec::new();

      self.expr(&decl.body, &mut none)
    } else {
      let mut body = Vec::new();
      let outer = std::mem::replace(&mut self.awaits, 0);

      self.stmt(&decl.body, &Dest::Return, &mut body);

      let awaited = std::mem::replace(&mut self.awaits, outer) > 0;
      let thunk =
        call(arrow_block(Vec::new(), body, awaited), Vec::new(), None);

      if awaited { await_(thunk) } else { thunk }
    };

    Stmt::Var(VarDecl {
      kind: VarKind::Const,
      name: ident(&decl.sym.name).at(decl.origin.clone()),
      init: Some(init),
      exported: decl.exported,
      origin: decl.origin.clone(),
    })
  }

  fn stmt(&mut self, e: &CExpr, dest: &Dest, out: &mut Vec<Stmt>) {
    if matches!(dest, Dest::Return)
      && let Some(tail) = &self.tail
      && let CExprKind::App { func, args } = &e.kind
      && is_self_call(func, args, tail.sym, tail.params.len())
    {
      let tail = tail.clone();

      return self.jump(&tail, args, e, out);
    }

    match &e.kind {
      CExprKind::Let { sym, value, body } => {
        self.binding(sym.as_ref(), value, out);
        self.stmt(body, dest, out);
      }
      CExprKind::If { cond, then_branch, else_branch } => {
        if matches!(dest, Dest::Declare(_) | Dest::Assign(_))
          && is_plain(cond)
          && is_plain(then_branch)
          && is_plain(else_branch)
        {
          let value = self.expr(e, out);

          return finish(dest, value, e, out);
        }

        let test = self.expr(cond, out);

        let dest = match dest {
          Dest::Declare(name) => {
            out.push(let_(name.clone()));
            Dest::Assign(name.clone())
          }
          Dest::Return | Dest::Assign(_) | Dest::Discard => dest.clone(),
        };

        let mut consequent = Vec::new();
        let mut alternate = Vec::new();

        self.stmt(then_branch, &dest, &mut consequent);
        self.stmt(else_branch, &dest, &mut alternate);

        if matches!(dest, Dest::Return) && alternate.len() > 1 {
          out.push(if_(test, consequent, None).at(e.origin.clone()));
          out.extend(alternate);
        } else {
          out.push(if_(test, consequent, Some(alternate)).at(e.origin.clone()));
        }
      }
      CExprKind::MatchFail => {
        out.push(expr_stmt(self.match_failure(e)).at(e.origin.clone()));
      }
      CExprKind::Case { .. } => case_reached(e),
      CExprKind::Method { .. } => method_reached(e),
      CExprKind::Try { body, caught, handler, handles } => {
        self.try_stmt((body, caught, handler, handles), dest, e, out);
      }
      CExprKind::Lit(_)
      | CExprKind::Var(_)
      | CExprKind::Builtin { .. }
      | CExprKind::Std { .. }
      | CExprKind::User { .. }
      | CExprKind::Lam { .. }
      | CExprKind::CtorFn { .. } => {
        if !matches!(dest, Dest::Discard) {
          let value = self.expr(e, out);

          finish(dest, value, e, out);
        }
      }
      CExprKind::App { .. }
      | CExprKind::Dict { .. }
      | CExprKind::Prim { .. }
      | CExprKind::Record { .. }
      | CExprKind::Update { .. }
      | CExprKind::Field { .. }
      | CExprKind::Ctor { .. }
      | CExprKind::Concat { .. }
      | CExprKind::Test { .. }
      | CExprKind::Op { .. }
      | CExprKind::Extern { .. }
      | CExprKind::Throw { .. } => {
        let value = self.expr(e, out);

        finish(dest, value, e, out);
      }
    }
  }

  fn try_stmt(
    &mut self,
    (body, caught, handler, types): (&CExpr, &Sym, &CExpr, &[String]),
    dest: &Dest,
    e: &CExpr,
    out: &mut Vec<Stmt>,
  ) {
    let dest = match dest {
      Dest::Declare(name) => {
        out.push(let_(name.clone()));
        Dest::Assign(name.clone())
      }
      Dest::Return | Dest::Assign(_) | Dest::Discard => dest.clone(),
    };
    let mut block = Vec::new();
    let tail = self.tail.take();

    self.stmt(body, &dest, &mut block);
    self.tail = tail;

    let param = self.names.fresh_ident("$e");
    let tags = types.iter().map(|h| string(h)).collect();
    let known = call(
      builtin_path("handles"),
      vec![Expr::Ident(param.clone()), array(tags)],
      None,
    );
    let mut handler_stmts = vec![if_(
      unary(UnaryOp::Not, known),
      vec![throw(Expr::Ident(param.clone()))],
      None,
    )];
    let bound = self.names.bind(caught);

    handler_stmts
      .push(const_(bound, member(Expr::Ident(param.clone()), "value")));
    self.stmt(handler, &dest, &mut handler_stmts);
    out.push(try_(block, param, handler_stmts).at(e.origin.clone()));
  }

  fn jump(
    &mut self,
    tail: &Loop,
    args: &[CExpr],
    e: &CExpr,
    out: &mut Vec<Stmt>,
  ) {
    let refs: Vec<&CExpr> = args.iter().collect();
    let values = self.operands(&refs, out);
    let changed: Vec<(&Ident, &Sym, &CExpr, Expr)> = tail
      .targets
      .iter()
      .zip(&tail.params)
      .zip(args.iter().zip(values))
      .filter(|((_, param), (arg, _))| {
        !matches!(&arg.kind, CExprKind::Var(sym) if sym.id == param.id)
      })
      .map(|((target, param), (arg, value))| (target, param, arg, value))
      .collect();
    let read_later: Vec<bool> = (0..changed.len())
      .map(|i| {
        !tail.fresh
          && changed[i + 1..]
            .iter()
            .any(|(_, _, arg, _)| reads(arg, changed[i].1))
      })
      .collect();
    let mut direct = Vec::new();
    let mut delayed = Vec::new();

    for ((target, _, _, value), later) in changed.into_iter().zip(read_later) {
      if later {
        let temp = self.names.temp();

        out.push(const_(temp.clone(), value));
        delayed.push((target.clone(), Expr::Ident(temp)));
      } else {
        direct.push((target.clone(), value));
      }
    }

    for (target, value) in direct.into_iter().chain(delayed) {
      out.push(expr_stmt(assign(target, value)));
    }

    out.push(continue_().at(e.origin.clone()));
  }

  fn binding(&mut self, sym: Option<&Sym>, value: &CExpr, out: &mut Vec<Stmt>) {
    let dest = match sym {
      Some(sym) => Dest::Declare(self.names.bind(sym)),
      None => Dest::Discard,
    };

    self.stmt(value, &dest, out);
  }

  #[allow(clippy::too_many_lines, reason = "one arm per Core expression kind")]
  fn expr(&mut self, e: &CExpr, out: &mut Vec<Stmt>) -> Expr {
    let origin = e.origin.clone();

    match &e.kind {
      CExprKind::Lit(lit) => lit_expr(lit).at(origin),
      CExprKind::Var(sym) => Expr::Ident(self.names.reference(sym).at(origin)),
      CExprKind::Builtin { module, member } => {
        builtin(module, member).at(origin)
      }
      CExprKind::Std { module, member: name } => {
        member(var(&std_namespace(module)), &js_ident(name)).at(origin)
      }
      CExprKind::User { module, member: name } => {
        member(var(&user_namespace(module)), &js_ident(name)).at(origin)
      }
      CExprKind::Method { .. } => method_reached(e),
      CExprKind::Dict { trait_path, target, module, args } => {
        let name = dict_name(trait_path, target, module.as_deref());
        let callee = match module.as_deref() {
          None => var(&name),
          Some(path) => match path.strip_prefix("Std.") {
            Some(std) => member(var(&std_namespace(std)), &name),
            None => member(var(&user_namespace(path)), &name),
          },
        };

        if args.is_empty() {
          callee.at(origin)
        } else {
          let refs: Vec<&CExpr> = args.iter().collect();
          let values = self.operands(&refs, out);

          call(callee, values, origin)
        }
      }
      CExprKind::Lam { params, body } => {
        self.lam(params, body, e.effects.as_ref()).at(origin)
      }
      CExprKind::App { func, args } if inlined(e) => {
        let CExprKind::Field { target, name } = &func.kind else {
          ice("an inlined method call without a dictionary", origin.as_ref())
        };
        let refs: Vec<&CExpr> = args.iter().collect();
        let mut values = self.operands(&refs, out).into_iter();
        let mut next = || {
          values.next().unwrap_or_else(|| {
            ice("an inlined method call with too few arguments", None)
          })
        };

        match (name.as_str(), prelude_primitive(target, name)) {
          ("eq", _) => binary(BinaryOp::StrictEq, next(), next()).at(origin),
          (_, Some("String")) => next(),
          _ => template(vec![String::new(), String::new()], vec![next()]),
        }
      }
      CExprKind::App { func, args } => {
        let direct = matches!(
          &func.kind,
          CExprKind::Var(sym)
            if self.colours.get(&sym.id) == Some(&Some(Colour::Poly))
        );
        let operands: Vec<&CExpr> = if direct {
          args.iter().collect()
        } else {
          std::iter::once(&**func).chain(args).collect()
        };
        let mut values = self.operands(&operands, out);
        let callee = (!direct).then(|| values.remove(0));

        self.apply(func, callee, values, e.effects.as_ref(), origin)
      }
      CExprKind::Prim { op, args } => self.prim(*op, args, e, out),
      CExprKind::Let { sym, value, body } => {
        self.binding(sym.as_ref(), value, out);
        self.expr(body, out)
      }
      CExprKind::If { cond, then_branch, else_branch } => {
        if is_plain(cond) && is_plain(then_branch) && is_plain(else_branch) {
          let test = self.expr(cond, out);
          let consequent = self.expr(then_branch, out);
          let alternate = self.expr(else_branch, out);

          conditional(test, consequent, alternate).at(origin)
        } else {
          let temp = self.names.temp();

          out.push(let_(temp.clone()));
          self.stmt(e, &Dest::Assign(temp.clone()), out);

          Expr::Ident(temp)
        }
      }
      CExprKind::Case { .. } => case_reached(e),
      CExprKind::Record { fields } => {
        let values: Vec<&CExpr> = fields.iter().map(|(_, v)| v).collect();
        let values = self.operands(&values, out);

        object(
          fields.iter().zip(values).map(|((k, _), v)| prop(k, v)).collect(),
        )
        .at(origin)
      }
      CExprKind::Update { base, fields } => {
        let operands: Vec<&CExpr> = std::iter::once(&**base)
          .chain(fields.iter().map(|(_, v)| v))
          .collect();
        let mut values = self.operands(&operands, out).into_iter();
        let mut props = vec![spread(
          values
            .next()
            .unwrap_or_else(|| ice("record update without a base", None)),
        )];

        props.extend(fields.iter().zip(values).map(|((k, _), v)| prop(k, v)));

        object(props).at(origin)
      }
      CExprKind::Field { target, name } => {
        member(self.expr(target, out), name).at(origin)
      }
      CExprKind::Ctor { .. }
        if let Some((items, tail)) = self.list_chain(e) =>
      {
        let operands: Vec<&CExpr> = items.into_iter().chain(tail).collect();
        let mut values = self.operands(&operands, out);
        let tail = tail.map(|_| {
          values.pop().unwrap_or_else(|| ice("list without its tail", None))
        });
        let args = std::iter::once(array(values)).chain(tail).collect();

        call(builtin_path("list"), args, None).at(origin)
      }
      CExprKind::Ctor { ctor, args } => {
        let args: Vec<&CExpr> = args.iter().collect();
        let values = self.operands(&args, out);

        self.variant(*ctor, values).at(origin)
      }
      CExprKind::CtorFn { ctor } => {
        let params: Vec<Ident> = (0..self.ctor(*ctor).arity)
          .map(|i| ident(&format!("_{i}")))
          .collect();
        let values = params.iter().cloned().map(Expr::Ident).collect();

        arrow(params, self.variant(*ctor, values), false).at(origin)
      }
      CExprKind::Concat { parts } => self.concat(parts, out).at(origin),
      CExprKind::Test { test, target } => {
        let target = self.expr(target, out);

        match test {
          PatternTest::Tag(ctor) => binary(
            BinaryOp::StrictEq,
            member(target, "$"),
            string(&self.ctor(*ctor).name),
          ),
          PatternTest::Lit(lit) => {
            binary(BinaryOp::StrictEq, target, lit_expr(lit))
          }
        }
      }
      CExprKind::MatchFail => self.match_failure(e),
      CExprKind::Op { effect, op } => {
        let object = match self.binds.iter().find(|(e, _)| e == effect) {
          Some((_, Some(module))) => {
            member(var(&bind_owner(module)), &bind_name(effect))
          }
          Some((_, None)) | None => var(&bind_name(effect)),
        };

        member(object, op).at(origin)
      }
      CExprKind::Extern { name } => var(&extern_name(name)).at(origin),
      CExprKind::Throw { value, tag } => {
        let value = self.expr(value, out);

        call(builtin_path("raise"), vec![string(tag), value], origin)
      }
      CExprKind::Try { .. } => {
        let temp = self.names.temp();

        out.push(let_(temp.clone()));
        self.stmt(e, &Dest::Assign(temp.clone()), out);

        Expr::Ident(temp)
      }
    }
  }

  fn codec(&mut self, dict: Option<&CExpr>) -> Expr {
    let mut out = Vec::new();
    let value = dict.map_or_else(null, |d| self.expr(d, &mut out));

    if !out.is_empty() {
      ice(
        "a bridge dictionary needed statements",
        dict.and_then(|d| d.origin.as_ref()),
      );
    }

    value
  }

  fn codecs(&mut self, op: &crate::core::ir::CBridgeOp) -> Vec<Expr> {
    let params = op.params.iter().map(|p| self.codec(p.as_ref())).collect();
    let ret = self.codec(op.ret.as_ref());
    let throws = op
      .throws
      .iter()
      .map(|(tag, dict)| array(vec![string(tag), self.codec(Some(dict))]))
      .collect();

    vec![array(params), ret, array(throws)]
  }

  fn bridge_client(&mut self, bridge: &CBridge) -> Item {
    let props = bridge
      .ops
      .iter()
      .map(|op| {
        let mut args = vec![string(&bridge.effect), string(&op.op)];

        args.extend(self.codecs(op));
        prop(&op.op, call(member(builtin_path("bridge"), "op"), args, None))
      })
      .collect();

    Item::Stmt(Stmt::Var(VarDecl {
      kind: VarKind::Const,
      name: ident(&bind_name(&bridge.effect)),
      init: Some(object(props)),
      exported: true,
      origin: None,
    }))
  }

  fn bridge_table(&mut self, bridge: &CBridge) -> Item {
    let bound = match self.binds.iter().find(|(e, _)| *e == bridge.effect) {
      Some((_, Some(module))) => {
        member(var(&bind_owner(module)), &bind_name(&bridge.effect))
      }
      Some((_, None)) | None => var(&bind_name(&bridge.effect)),
    };
    let props = bridge
      .ops
      .iter()
      .map(|op| prop(&op.op, array(self.codecs(op))))
      .collect();
    let table = call(
      member(builtin_path("bridge"), "table"),
      vec![string(&bridge.effect), bound, object(props)],
      None,
    );

    Item::Stmt(Stmt::Var(VarDecl {
      kind: VarKind::Const,
      name: ident(&bridge_name(&bridge.effect)),
      init: Some(table),
      exported: true,
      origin: None,
    }))
  }

  fn operands(&mut self, es: &[&CExpr], out: &mut Vec<Stmt>) -> Vec<Expr> {
    let last_hoisted = es.iter().rposition(|e| !is_plain(e));
    let mut values = Vec::with_capacity(es.len());

    for (i, e) in es.iter().enumerate() {
      let value = self.expr(e, out);

      values.push(match last_hoisted {
        Some(k) if i < k && !is_pure(e) => {
          let temp = self.names.temp();

          out.push(const_(temp.clone(), value));
          Expr::Ident(temp)
        }
        Some(_) | None => value,
      });
    }

    values
  }

  fn prim(
    &mut self,
    op: PrimOp,
    args: &[CExpr],
    e: &CExpr,
    out: &mut Vec<Stmt>,
  ) -> Expr {
    let origin = e.origin.clone();

    if let (PrimOp::And | PrimOp::Or, [left, right]) = (op, args)
      && !is_plain(right)
    {
      let short = CExpr::new(CExprKind::Lit(Lit::Bool(op == PrimOp::Or)), None);
      let (then_branch, else_branch) = if op == PrimOp::And {
        (right.clone(), short)
      } else {
        (short, right.clone())
      };
      let as_if = CExpr::new(
        CExprKind::If {
          cond: Box::new(left.clone()),
          then_branch: Box::new(then_branch),
          else_branch: Box::new(else_branch),
        },
        origin,
      );

      return self.expr(&as_if, out);
    }

    if let (PrimOp::Not, [inner]) = (op, args)
      && inlined(inner)
      && let CExprKind::App { func, args: pair } = &inner.kind
      && matches!(&func.kind, CExprKind::Field { name, .. } if name == "eq")
    {
      let refs: Vec<&CExpr> = pair.iter().collect();
      let mut values = self.operands(&refs, out).into_iter();

      if let (Some(a), Some(b)) = (values.next(), values.next()) {
        return binary(BinaryOp::StrictNe, a, b).at(origin);
      }
    }

    let kind = args.first().map_or(Prim::Other, prim_of);
    let refs: Vec<&CExpr> = args.iter().collect();
    let mut values = self.operands(&refs, out).into_iter();
    let mut next = || {
      values.next().unwrap_or_else(|| {
        ice(format!("{} with too few operands", op.as_str()), None)
      })
    };

    match op {
      PrimOp::Add => binary(BinaryOp::Add, next(), next()).at(origin),
      PrimOp::Sub => binary(BinaryOp::Sub, next(), next()).at(origin),
      PrimOp::Mul => binary(BinaryOp::Mul, next(), next()).at(origin),
      PrimOp::Div if kind == Prim::Int => call(
        member(var("Math"), "trunc"),
        vec![binary(BinaryOp::Div, next(), next())],
        None,
      )
      .at(origin),
      PrimOp::Div => binary(BinaryOp::Div, next(), next()).at(origin),
      PrimOp::Mod => binary(BinaryOp::Mod, next(), next()).at(origin),
      PrimOp::Lt => binary(BinaryOp::Lt, next(), next()).at(origin),
      PrimOp::Le => binary(BinaryOp::Le, next(), next()).at(origin),
      PrimOp::Gt => binary(BinaryOp::Gt, next(), next()).at(origin),
      PrimOp::Ge => binary(BinaryOp::Ge, next(), next()).at(origin),
      PrimOp::And => logical(LogicalOp::And, next(), next()).at(origin),
      PrimOp::Or => logical(LogicalOp::Or, next(), next()).at(origin),
      PrimOp::Neg => unary(UnaryOp::Negate, next()).at(origin),
      PrimOp::Not => unary(UnaryOp::Not, next()).at(origin),
      PrimOp::Eq => binary(BinaryOp::StrictEq, next(), next()).at(origin),
      PrimOp::Ne => binary(BinaryOp::StrictNe, next(), next()).at(origin),
      PrimOp::BitAnd => binary(BinaryOp::BitAnd, next(), next()).at(origin),
      PrimOp::BitOr => binary(BinaryOp::BitOr, next(), next()).at(origin),
      PrimOp::BitXor => binary(BinaryOp::BitXor, next(), next()).at(origin),
      PrimOp::BitNot => unary(UnaryOp::BitNot, next()).at(origin),
    }
  }

  fn apply(
    &mut self,
    func: &CExpr,
    callee: Option<Expr>,
    mut values: Vec<Expr>,
    effects: Option<&EffectSlot>,
    origin: Option<Span>,
  ) -> Expr {
    let callee = || {
      callee.unwrap_or_else(|| ice("a direct call without its callee", None))
    };

    match &func.kind {
      CExprKind::Builtin { .. } => return call(callee(), values, origin),
      CExprKind::Op { .. } => {
        return self.suspend(call(callee(), values, origin.clone()), origin);
      }
      CExprKind::Extern { .. } => {
        let value = call(callee(), values, origin.clone());

        return if self.in_bind
          || colour(effects, &self.pure_vars) != Colour::Sync
        {
          self.suspend(value, origin)
        } else {
          value
        };
      }
      _ => {}
    }

    let colour = colour(effects, &self.pure_vars);
    let callee_colour = match &func.kind {
      CExprKind::Var(sym) => self.colours.get(&sym.id).copied(),
      _ => None,
    };

    if colour == Colour::Sync
      && matches!(callee_colour, Some(None | Some(Colour::Async)))
    {
      ice(
        "a call can't suspend by its row, but its callee is async",
        origin.as_ref(),
      );
    }

    let direct = match &func.kind {
      CExprKind::Var(sym) if callee_colour == Some(Some(Colour::Poly)) => {
        let name = version(&sym.name, colour);

        self.names.refs.insert(name.clone());
        Some(var(&name).at(func.origin.clone()))
      }
      _ => None,
    };

    match (colour, direct) {
      (Colour::Sync, Some(direct)) => call(direct, values, origin),
      (Colour::Sync, None) => call(callee(), values, origin),
      (Colour::Async | Colour::Poly, Some(direct)) => {
        self.suspend(call(direct, values, origin.clone()), origin)
      }
      (Colour::Async | Colour::Poly, None) => {
        if callee_colour != Some(Some(Colour::Async)) {
          values.push(builtin_path("ASYNC"));
        }

        self.suspend(call(callee(), values, origin.clone()), origin)
      }
    }
  }

  fn lam(
    &mut self,
    params: &[Sym],
    body: &CExpr,
    effects: Option<&EffectSlot>,
  ) -> Expr {
    match colour(effects, &self.pure_vars) {
      Colour::Sync => self.arrow_fn(params, body, (false, false)),
      Colour::Async => self.arrow_fn(params, body, (true, false)),
      Colour::Poly => {
        let saved = self.names.clone();

        self.pure_vars.extend(tail_var(effects));

        let sync = self.arrow_fn(params, body, (false, false));

        self.pure_vars.pop();
        self.names = saved;

        let asynchronous = self.arrow_fn(params, body, (true, false));

        call(
          builtin_path("poly"),
          vec![count(params.len()), sync, asynchronous],
          None,
        )
      }
    }
  }

  fn arrow_fn(
    &mut self,
    params: &[Sym],
    body: &CExpr,
    (is_async, in_bind): (bool, bool),
  ) -> Expr {
    let params = params.iter().map(|p| self.names.bind(p)).collect();
    let outer = std::mem::replace(&mut self.awaits, 0);
    let tail = self.tail.take();
    let in_bind = std::mem::replace(&mut self.in_bind, in_bind);
    let value = if is_plain(body) {
      let mut none = Vec::new();
      let value = self.expr(body, &mut none);

      arrow(params, value, is_async)
    } else {
      let mut stmts = Vec::new();

      self.stmt(body, &Dest::Return, &mut stmts);
      arrow_block(params, stmts, is_async)
    };
    let awaited = std::mem::replace(&mut self.awaits, outer) > 0;

    self.tail = tail;
    self.in_bind = in_bind;

    if awaited && !is_async {
      synchronous_awaits("a function", body.origin.as_ref());
    }

    value
  }

  fn concat(&mut self, parts: &[CExpr], out: &mut Vec<Stmt>) -> Expr {
    let refs: Vec<&CExpr> = parts.iter().collect();
    let values = self.operands(&refs, out);
    let mut quasis = vec![String::new()];
    let mut exprs = Vec::new();

    for (part, value) in parts.iter().zip(values) {
      let value = match value {
        Expr::TemplateLit(t) if inlined(part) && t.exprs.len() == 1 => {
          t.exprs.into_iter().next().unwrap_or_else(|| ice("empty slot", None))
        }
        other => other,
      };

      if let CExprKind::Lit(Lit::String(text)) = &part.kind {
        if let Some(quasi) = quasis.last_mut() {
          quasi.push_str(text);
        }
      } else {
        exprs.push(value);
        quasis.push(String::new());
      }
    }

    template(quasis, exprs)
  }

  fn list_chain<'e>(
    &self,
    e: &'e CExpr,
  ) -> Option<(Vec<&'e CExpr>, Option<&'e CExpr>)> {
    let is = |e: &CExpr, name: &str| match &e.kind {
      CExprKind::Ctor { ctor, .. } => {
        let info = self.ctor(*ctor);

        info.owner == "List" && info.name == name
      }
      _ => false,
    };
    let mut items = Vec::new();
    let mut rest = e;

    while is(rest, "Cons") {
      let CExprKind::Ctor { args, .. } = &rest.kind else { break };
      let [head, tail] = args.as_slice() else { return None };

      items.push(head);
      rest = tail;
    }

    let tail = (!is(rest, "Nil")).then_some(rest);

    match (items.len(), tail) {
      (0, _) | (1, Some(_)) => None,
      _ => Some((items, tail)),
    }
  }

  fn variant(&self, ctor: CtorId, payload: Vec<Expr>) -> Expr {
    let mut props = vec![prop("$", string(&self.ctor(ctor).name))];

    props.extend(
      payload.into_iter().enumerate().map(|(i, v)| prop(&format!("_{i}"), v)),
    );

    object(props)
  }

  fn match_failure(&self, e: &CExpr) -> Expr {
    let Some(span) = &e.origin else {
      ice("MatchFail without the match's origin", None);
    };
    let position = self.file.position_at(span.start);
    let line = position.line + 1;
    let column = self.file.code_point_column(position);

    call(
      builtin_path("matchFailure"),
      vec![string(self.file.name()), count(line), count(column)],
      e.origin.clone(),
    )
  }

  fn ctor(&self, id: CtorId) -> &CtorInfo {
    &self.ctors[id.0 as usize]
  }
}

#[derive(Clone)]
struct Names<'a> {
  top: &'a HashMap<u32, String>,
  used: HashSet<String>,
  locals: HashMap<u32, String>,
  refs: HashSet<String>,
}

impl<'a> Names<'a> {
  fn new(top: &'a HashMap<u32, String>) -> Self {
    Self {
      top,
      used: top.values().cloned().collect(),
      locals: HashMap::new(),
      refs: HashSet::new(),
    }
  }

  fn bind(&mut self, sym: &Sym) -> Ident {
    let name = self.fresh(&sym.name);

    self.locals.insert(sym.id, name);
    self.reference(sym)
  }

  fn temp(&mut self) -> Ident {
    ident(&self.fresh("$t"))
  }

  fn fresh_ident(&mut self, base: &str) -> Ident {
    ident(&self.fresh(base))
  }

  fn reference(&mut self, sym: &Sym) -> Ident {
    let name = match self.locals.get(&sym.id) {
      Some(name) => name,
      None => match self.top.get(&sym.id) {
        Some(name) => {
          self.refs.insert(name.clone());
          name
        }
        None => ice(format!("unbound symbol {}/{}", sym.name, sym.id), None),
      },
    };

    let mut id = ident(name);

    if **name != *sym.name && !sym.name.starts_with('$') {
      id.original_name = Some(sym.name.to_string());
    }

    id
  }

  fn fresh(&mut self, base: &str) -> String {
    let mut name = base.to_string();
    let mut i = 0;

    while self.used.contains(&name) {
      i += 1;
      name = if base.starts_with('$') {
        format!("{base}{i}")
      } else {
        format!("{base}${i}")
      };
    }

    self.used.insert(name.clone());
    name
  }
}

fn finish(dest: &Dest, value: Expr, e: &CExpr, out: &mut Vec<Stmt>) {
  let origin = e.origin.clone();

  out.push(match dest {
    Dest::Return => ret(value).at(origin),
    Dest::Declare(name) => const_(name.clone(), value).at(origin),
    Dest::Assign(temp) => expr_stmt(assign(temp.clone(), value)),
    Dest::Discard => expr_stmt(value).at(origin),
  });
}

fn is_self_call(func: &CExpr, args: &[CExpr], sym: u32, arity: usize) -> bool {
  matches!(&func.kind, CExprKind::Var(s) if s.id == sym) && args.len() == arity
}

fn tail_calls(e: &CExpr, sym: u32, arity: usize) -> bool {
  match &e.kind {
    CExprKind::App { func, args } => is_self_call(func, args, sym, arity),
    CExprKind::Let { body, .. } => tail_calls(body, sym, arity),
    CExprKind::If { then_branch, else_branch, .. } => {
      tail_calls(then_branch, sym, arity) || tail_calls(else_branch, sym, arity)
    }
    CExprKind::Try { handler, .. } => tail_calls(handler, sym, arity),
    _ => false,
  }
}

fn reads(e: &CExpr, sym: &Sym) -> bool {
  let mut found = false;

  walk(e, &mut |e| {
    found |= matches!(&e.kind, CExprKind::Var(s) if s.id == sym.id);
  });
  found
}

fn has_lambda(e: &CExpr) -> bool {
  let mut found = false;

  walk(e, &mut |e| found |= matches!(e.kind, CExprKind::Lam { .. }));
  found
}

fn is_plain(e: &CExpr) -> bool {
  match &e.kind {
    CExprKind::Let { .. }
    | CExprKind::If { .. }
    | CExprKind::MatchFail
    | CExprKind::Case { .. }
    | CExprKind::Try { .. } => false,
    CExprKind::Throw { value, .. } => is_plain(value),
    CExprKind::Lit(_)
    | CExprKind::Var(_)
    | CExprKind::Builtin { .. }
    | CExprKind::Std { .. }
    | CExprKind::User { .. }
    | CExprKind::Method { .. }
    | CExprKind::Lam { .. }
    | CExprKind::CtorFn { .. }
    | CExprKind::Op { .. }
    | CExprKind::Extern { .. } => true,
    CExprKind::App { func, args } => {
      is_plain(func) && args.iter().all(is_plain)
    }
    CExprKind::Prim { args, .. }
    | CExprKind::Dict { args, .. }
    | CExprKind::Ctor { args, .. }
    | CExprKind::Concat { parts: args } => args.iter().all(is_plain),
    CExprKind::Record { fields } => fields.iter().all(|(_, v)| is_plain(v)),
    CExprKind::Update { base, fields } => {
      is_plain(base) && fields.iter().all(|(_, v)| is_plain(v))
    }
    CExprKind::Field { target, .. } | CExprKind::Test { target, .. } => {
      is_plain(target)
    }
  }
}

fn is_pure(e: &CExpr) -> bool {
  match &e.kind {
    CExprKind::Lit(_)
    | CExprKind::Var(_)
    | CExprKind::Builtin { .. }
    | CExprKind::Std { .. }
    | CExprKind::User { .. }
    | CExprKind::Method { .. }
    | CExprKind::Lam { .. }
    | CExprKind::CtorFn { .. }
    | CExprKind::Op { .. }
    | CExprKind::Extern { .. } => true,
    CExprKind::Dict { args, .. } => args.is_empty(),
    CExprKind::App { .. }
    | CExprKind::Prim { .. }
    | CExprKind::Let { .. }
    | CExprKind::If { .. }
    | CExprKind::Case { .. }
    | CExprKind::Record { .. }
    | CExprKind::Update { .. }
    | CExprKind::Field { .. }
    | CExprKind::Ctor { .. }
    | CExprKind::Concat { .. }
    | CExprKind::Test { .. }
    | CExprKind::MatchFail
    | CExprKind::Throw { .. }
    | CExprKind::Try { .. } => false,
  }
}

fn lit_expr(lit: &Lit) -> Expr {
  match lit {
    Lit::Number(n) => number(*n),
    Lit::String(s) => string(s),
    Lit::Bool(b) => boolean(*b),
  }
}

fn std_namespace(module: &str) -> String {
  format!("${module}")
}

fn bind_owner(path: &str) -> String {
  path.strip_prefix("Std.").map_or_else(|| user_namespace(path), std_namespace)
}

fn user_namespace(path: &str) -> String {
  format!("$m${}", path.replace('.', "$"))
}

fn builtin_path(name: &str) -> Expr {
  member(var(RT), name)
}

fn builtin(module: &str, name: &str) -> Expr {
  member(builtin_path(module), name)
}

fn count(n: usize) -> Expr {
  number(f64::from(u32::try_from(n).unwrap_or(u32::MAX)))
}

fn synchronous_awaits(name: &str, origin: Option<&Span>) -> ! {
  ice(
    format!("`{name}` can't suspend by its effect row, yet its body awaits"),
    origin,
  )
}

fn method_reached(e: &CExpr) -> ! {
  ice(
    "`Method` reached emission: run dictionary elaboration first",
    e.origin.as_ref(),
  )
}

#[must_use]
pub fn dict_name(
  trait_path: &str,
  target: &str,
  module: Option<&str>,
) -> String {
  let local = |name: &str| {
    module
      .and_then(|m| {
        name.strip_prefix(m).and_then(|rest| rest.strip_prefix('.'))
      })
      .unwrap_or(name)
      .replace('.', "$")
  };

  format!("${}${}", local(trait_path), local(target))
}

fn case_reached(e: &CExpr) -> ! {
  ice("`Case` reached emission: run match compilation first", e.origin.as_ref())
}
