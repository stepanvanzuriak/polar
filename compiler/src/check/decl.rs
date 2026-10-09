use std::{
  collections::{HashMap, HashSet},
  sync::Arc,
};

use crate::{
  check::{
    Checker, Types,
    convert::{TypeVars, VarMode, substitute},
    effects::RowOwner,
    env::{Entry, TypeDef},
    hosts::{Blame, row_hosts},
    last_expr,
    report::Because,
  },
  core::{ir::Sym, lower::Resolved},
  shared::codes::DiagnosticCode::{
    ArgumentCount, BadRecipe, PrivateTypeExposed, UnboundTypeVariable,
  },
  shared::diagnostic::{Diagnostic, Label},
  shared::source::Span,
  stdlib,
  syntax::ast::{
    ConstDecl, Decl, FnDecl, ImplDecl, Module, TraitDecl, TypeExpr,
  },
  types::{
    generalise::{
      close_row, default_brands, default_numbers, first_gen,
      generalise_indexed, seal_closed,
    },
    print::Printer,
    ty::{Brand, Effects, Pred, Row, Scheme, Tail, Type},
    unify::unify,
  },
};

struct RecipeCase<'a> {
  decl: &'a FnDecl,
  sym: Sym,
  signature: Signature,
  name: String,
}

struct Signature {
  params: Vec<Type>,
  ret: Type,
  effects: Effects,
  vars: TypeVars,
  givens: Vec<Pred>,
}

impl Signature {
  fn ty(&self) -> Type {
    Type::func_with(self.params.clone(), self.ret.clone(), self.effects.clone())
  }
}

impl<'a> Checker<'a> {
  pub(crate) fn module(&mut self, module: &'a Module) -> Types {
    self.build_env(module);
    self.build_effects();

    let decls: Vec<&'a Decl> =
      module.zones.iter().flat_map(|zone| &zone.decls).collect();
    let syms: HashMap<(usize, usize), Sym> = self
      .core
      .decls
      .iter()
      .filter_map(|d| {
        d.origin.as_ref().map(|o| ((o.start, o.end), d.sym.clone()))
      })
      .collect();
    let sym_of = |span: &Span| syms.get(&(span.start, span.end)).cloned();
    let consts: Vec<(&'a ConstDecl, Sym)> = decls
      .iter()
      .filter_map(|d| match d {
        Decl::Const(c) => sym_of(&c.name.span).map(|s| (c, s)),
        _ => None,
      })
      .collect();
    let fns: Vec<(&'a FnDecl, Sym)> = decls
      .iter()
      .filter_map(|d| match d {
        Decl::Fn(f) => sym_of(&f.name.span).map(|s| (f, s)),
        _ => None,
      })
      .collect();
    let mut failed = HashSet::new();
    let recipes = self.recipe_signatures(&decls, &sym_of);

    self.declarations(&consts, &fns, &mut failed);

    let mut out = Vec::new();

    for recipe in &recipes {
      if let Some(scheme) = self.locals.get(&recipe.sym.id) {
        out.push((recipe.name.clone(), scheme.clone()));
      }
    }

    self.recipe_bodies(recipes);

    for decl in &decls {
      let (name, sym) = match decl {
        Decl::Const(c) => (&c.name.text, sym_of(&c.name.span)),
        Decl::Fn(f) => (&f.name.text, sym_of(&f.name.span)),
        Decl::Import(_)
        | Decl::Trait(_)
        | Decl::Type(_)
        | Decl::Impl(_)
        | Decl::Export(_)
        | Decl::Host(_)
        | Decl::Effect(_)
        | Decl::Extern(_)
        | Decl::Bind(_)
        | Decl::Plugin(_) => continue,
      };

      if let Some(scheme) = sym.and_then(|s| self.locals.get(&s.id)) {
        out.push((name.clone(), scheme.clone()));
      }
    }

    self.impl_bodies(&decls);
    self.bind_bodies(&decls);
    self.check_bridges(&decls);
    self.check_bind_hosts();
    self.check_leftovers();
    self.exhaustiveness();
    self.private_leaks(module, &out);

    let mut types = self.finish(out, failed);

    types.exported_types = module.exported_types();
    types
  }

  fn finish(
    &mut self,
    out: Vec<(String, Scheme)>,
    failed: HashSet<String>,
  ) -> Types {
    let exprs = self.finish_exprs();
    let (evidence, shows) = self.finish_evidence();
    let traits = self
      .local_traits
      .iter()
      .filter_map(|name| self.env.traits.get(&self.trait_path(name)).cloned())
      .collect();
    let impls = self.env.impls.values().cloned().collect();
    let mut hosts: Vec<(String, crate::check::hosts::HostSet)> = out
      .iter()
      .filter(|(name, _)| !name.starts_with('$'))
      .map(|(name, scheme)| {
        let row = match &scheme.ty {
          Type::Fn { effects, .. } => effects.clone(),
          _ => Effects::pure(),
        };

        (name.clone(), row_hosts(self.effects, &row))
      })
      .collect();

    hosts.sort_by(|a, b| a.0.cmp(&b.0));

    let extern_types = self.extern_types();
    let signatures = std::mem::take(&mut self.signatures)
      .into_iter()
      .map(|(k, t)| (k, self.store.zonk(&t)))
      .collect();

    Types {
      decls: out,
      exprs,
      type_defs: std::mem::take(&mut self.local_types),
      exported_types: Vec::new(),
      failed,
      evidence,
      shows,
      decl_preds: std::mem::take(&mut self.decl_preds),
      traits,
      impls,
      impl_heads: std::mem::take(&mut self.impl_heads),
      op_schemes: self
        .op_schemes
        .iter()
        .filter(|((effect, _), _)| {
          self.effects.effects.get(effect).is_some_and(|e| e.local)
        })
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect(),
      extern_types,
      bind_rows: self
        .bind_rows
        .iter()
        .map(|(k, (row, _))| (k.clone(), row.clone()))
        .collect(),
      bind_uses: std::mem::take(&mut self.bind_rows)
        .into_iter()
        .map(|(k, (_, uses))| (k, uses))
        .collect(),
      handles: std::mem::take(&mut self.handles),
      throws: std::mem::take(&mut self.throws),
      hosts,
      first_uses: std::mem::take(&mut self.decl_uses)
        .into_iter()
        .map(|(k, v)| (k, v.into_vec()))
        .collect(),
      signatures,
      closed_rows: std::mem::take(&mut self.closed_rows),
      bridges: std::mem::take(&mut self.bridges),
    }
  }

  fn declarations(
    &mut self,
    consts: &[(&'a ConstDecl, Sym)],
    fns: &[(&'a FnDecl, Sym)],
    failed: &mut HashSet<String>,
  ) {
    for (f, sym) in fns {
      self.top_fns.insert(sym.id, f);
    }

    let mut annotated: Vec<(usize, Signature)> = Vec::new();

    for (i, (f, sym)) in fns.iter().enumerate() {
      if fully_annotated(f) {
        self.store.enter_level();

        let mut signature = self.signature(f);

        self.store.leave_level();

        let (scheme, order) = generalise_indexed(
          &self.store,
          &signature.ty(),
          &signature.givens,
          true,
        );

        signature.givens =
          order.iter().map(|&k| signature.givens[k].clone()).collect();
        self.decl_preds.insert(f.name.text.clone(), scheme.preds.clone());
        self.locals.insert(sym.id, scheme);
        annotated.push((i, signature));
      }
    }

    for (c, sym) in consts {
      let before = self.bag.error_count();

      self.constant(c, sym);

      if self.bag.error_count() > before {
        failed.insert(c.name.text.clone());
      }
    }

    let open: Vec<usize> =
      (0..fns.len()).filter(|&i| !fully_annotated(fns[i].0)).collect();

    for group in groups(&self.call_graph(fns, &open)) {
      let members: Vec<(&'a FnDecl, Sym)> =
        group.iter().map(|&k| fns[open[k]].clone()).collect();
      let before = self.bag.error_count();

      self.group(&members);

      if self.bag.error_count() > before {
        failed.extend(members.iter().map(|(f, _)| f.name.text.clone()));
      }
    }

    for (i, signature) in annotated {
      let before = self.bag.error_count();
      let start = self.wanted.len();
      let f = fns[i].0;

      self.givens.clone_from(&signature.givens);
      self.signature_span = Some(signature_span(f));
      self.store.enter_level();
      self.body(f, &signature);
      self.store.leave_level();
      self.solve_closed(start);
      self.givens.clear();
      self.signature_span = None;

      let row = self.store.zonk_effects(&signature.effects);

      self.decl_hosts(f, &row);

      if self.bag.error_count() > before {
        failed.insert(f.name.text.clone());
      }
    }
  }

  fn decl_hosts(&mut self, f: &'a FnDecl, row: &Effects) {
    let uses = self.decl_uses.get(&f.name.text).cloned().unwrap_or_default();
    let blame = Blame {
      what: format!("`{}`", f.name.text),
      at: f.name.span.clone(),
      row: f.effects.as_ref().map(|r| r.span.clone()),
      uses: &uses,
    };

    self.check_hosts(row, &blame);
    self.reported_hosts.clear();
  }

  fn call_graph(
    &self,
    fns: &[(&'a FnDecl, Sym)],
    open: &[usize],
  ) -> Vec<Vec<usize>> {
    let index_of: HashMap<u32, usize> =
      open.iter().enumerate().map(|(k, &i)| (fns[i].1.id, k)).collect();
    let mut edges: Vec<Vec<usize>> = vec![Vec::new(); open.len()];

    for (&(start, end), resolved) in &self.resolutions.0 {
      let Resolved::Top(target) = resolved else { continue };
      let Some(&to) = index_of.get(&target.id) else { continue };

      for (from, &i) in open.iter().enumerate() {
        let body = &fns[i].0.body.span;

        if body.start <= start && end <= body.end && !edges[from].contains(&to)
        {
          edges[from].push(to);
        }
      }
    }

    edges
  }

  fn signature(&mut self, f: &'a FnDecl) -> Signature {
    self.vars = TypeVars::rigid();

    let params = f
      .params
      .iter()
      .map(|p| match &p.ty {
        Some(ty) => self.convert(ty),
        None => self.fresh(),
      })
      .collect();
    let ret = match &f.return_type {
      Some(ty) => self.convert(ty),
      None => self.fresh(),
    };
    let effects = match &f.return_type {
      Some(_) => self.convert_effects(f.effects.as_ref()),
      None => Effects::open(self.fresh_var()),
    };
    let givens = self.givens_of(f);
    let mut vars = std::mem::replace(&mut self.vars, TypeVars::flexible());

    vars.mode = VarMode::Flexible;
    Signature { params, ret, effects, vars, givens }
  }

  fn givens_of(&mut self, f: &'a FnDecl) -> Vec<Pred> {
    let mut givens = Vec::new();

    for bound in &f.bounds {
      let Some(Resolved::Trait(path)) =
        self.resolutions.get(&bound.trait_name.span).cloned()
      else {
        continue;
      };

      match self.vars.get(&bound.var.text).cloned() {
        Some(ty @ Type::Rigid { .. }) => {
          givens.push(Pred { trait_path: self.trait_path(&path), ty });
        }
        _ => self.push(
          Diagnostic::error(
            UnboundTypeVariable,
            format!(
              "the type variable `{}` is not in the signature",
              bound.var.text
            ),
            Label::new(bound.var.span.clone()),
          )
          .with_help(format!(
            "a `where` can only name a type variable used by a parameter or \
             the return type, like `x: {}`",
            bound.var.text
          )),
        ),
      }
    }

    givens
  }

  fn body(&mut self, f: &'a FnDecl, signature: &Signature) {
    self.record_signature(&f.name.span, &signature.ty());
    self.vars = signature.vars.clone();

    for (param, ty) in f.params.iter().zip(&signature.params) {
      self.bind_mono(&param.name.span, ty.clone());
    }

    let owner = RowOwner::Function {
      name: f.name.text.clone(),
      row: f.effects.as_ref().map(|r| r.span.clone()),
      params: f
        .params
        .iter()
        .zip(&signature.params)
        .map(|(p, ty)| (p.name.text.clone(), ty.clone()))
        .collect(),
    };
    let ret = signature.ret.clone();
    let (body, _) = self.with_row(signature.effects.clone(), owner, |c| {
      c.with_return(ret, |c| c.block(&f.body))
    });
    let uses = std::mem::take(&mut self.last_uses);

    self.decl_uses.insert(f.name.text.clone(), uses);
    let blame = last_expr(&f.body.result).span();
    let because = match &f.return_type {
      Some(annotation) => Because::Annotation(annotation.span().clone()),
      None => Because::Nothing,
    };

    self.expect_because(&signature.ret, &body, blame, because);
    self.vars = TypeVars::flexible();
  }

  fn group(&mut self, members: &[(&'a FnDecl, Sym)]) {
    let start = self.wanted.len();

    self.group_members =
      members.iter().enumerate().map(|(i, (_, sym))| (sym.id, i)).collect();
    self.store.enter_level();

    let mut signatures = Vec::with_capacity(members.len());

    for (f, sym) in members {
      let signature = self.signature(f);

      self.locals.insert(sym.id, Scheme::new(0, signature.ty()));
      signatures.push(signature);
    }

    for (i, ((f, _), signature)) in members.iter().zip(&signatures).enumerate()
    {
      self.owner = Some(i);
      self.body(f, signature);
    }

    self.owner = None;
    self.group_members.clear();
    self.store.leave_level();

    for signature in &signatures {
      default_numbers(&mut self.store, &signature.ty());
      default_brands(&mut self.store, &signature.ty());
    }

    let types: Vec<Type> = signatures.iter().map(Signature::ty).collect();
    let givens: Vec<Vec<Pred>> =
      signatures.iter().map(|s| s.givens.clone()).collect();
    let calls = std::mem::take(&mut self.group_calls);
    let schemes = self.solve_group(start, &types, &givens, &calls);

    for (((f, sym), mut scheme), signature) in
      members.iter().zip(schemes).zip(&signatures)
    {
      if f.return_type.is_none() {
        close_row(&mut scheme);
      }

      seal_closed(&mut self.store, &signature.ty(), &scheme.ty);

      if let Type::Fn { effects, .. } = &scheme.ty {
        let row = effects.clone();

        self.decl_hosts(f, &row);
      }

      self.decl_preds.insert(f.name.text.clone(), scheme.preds.clone());
      self.locals.insert(sym.id, scheme);
    }
  }

  fn recipe_signatures(
    &mut self,
    decls: &[&'a Decl],
    sym_of: &dyn Fn(&Span) -> Option<Sym>,
  ) -> Vec<RecipeCase<'a>> {
    let mut out = Vec::new();

    for decl in decls {
      let Decl::Trait(t) = decl else { continue };
      let Some(recipe) = &t.recipe else { continue };
      let path = self.trait_path(&t.name.text);
      let keyword = Span::new(
        recipe.span.file.clone(),
        recipe.span.start,
        recipe.span.start + "derive".len(),
      );
      let Some(result) = self.recipe_result(t, &path, &keyword) else {
        continue;
      };
      let mut schemes = Vec::new();

      for case in &recipe.cases {
        let name = stdlib::recipe_fn(&t.name.text, &case.name.text);
        let Some(sym) = sym_of(&case.name.span) else { continue };
        let expected = self.recipe_case_type(&case.name.text, &result);
        let help = recipe_help(case, &result);

        if !fully_annotated(case) {
          self.push(
            Diagnostic::error(
              BadRecipe,
              format!(
                "the recipe case `{}` needs its types written",
                case.name.text
              ),
              Label::new(case.name.span.clone()),
            )
            .with_help(help),
          );
          continue;
        }

        self.store.enter_level();

        let signature = self.signature(case);

        self.store.leave_level();

        if unify(&mut self.store, &expected, &signature.ty()).is_err() {
          self.push(
            Diagnostic::error(
              BadRecipe,
              format!(
                "the recipe case `{}` has the wrong type",
                case.name.text
              ),
              Label::new(case.name.span.clone()),
            )
            .with_help(help),
          );
          continue;
        }

        let scheme =
          generalise_indexed(&self.store, &signature.ty(), &[], true).0;

        self.locals.insert(sym.id, scheme.clone());
        schemes.push((case.name.text.clone(), scheme));
        out.push(RecipeCase { decl: case, sym, signature, name });
      }

      if let Some(def) = self.env.traits.get_mut(&path) {
        def.recipe = schemes;
      }
    }

    out
  }

  fn recipe_result(
    &mut self,
    t: &TraitDecl,
    path: &Arc<str>,
    keyword: &Span,
  ) -> Option<Type> {
    let method = self
      .env
      .traits
      .get(path)
      .and_then(|def| def.methods.first().cloned())
      .map(|(_, scheme)| scheme.ty);
    let problem = match (&t.methods[..], &method) {
      ([_], Some(Type::Fn { params, ret, .. })) => {
        if params.len() != 1 || params[0] != Type::Gen(0) {
          format!(
            "a recipe's method must take one parameter of type `{}`",
            t.param.text
          )
        } else if first_gen(ret).is_some() {
          format!("a recipe's method must not return `{}`", t.param.text)
        } else {
          return Some((**ret).clone());
        }
      }
      _ => "a recipe needs exactly one method".to_string(),
    };

    self.push(
      Diagnostic::error(BadRecipe, problem, Label::new(keyword.clone()))
        .with_help(
          "a recipe works for traits with one method that takes the value \
           and returns something that isn't it, like `describe(value: a) -> \
           String`",
        ),
    );
    None
  }

  fn recipe_case_type(&self, case: &str, result: &Type) -> Type {
    let record = Type::Record(Row::new(
      vec![
        (Arc::from("name"), Type::string()),
        (Arc::from("value"), result.clone()),
      ],
      Tail::anonymous(),
    ));

    match case {
      "record" => Type::func(vec![self.std_list(record)], result.clone()),
      _ => Type::func(
        vec![Type::string(), self.std_list(result.clone())],
        result.clone(),
      ),
    }
  }

  fn recipe_bodies(&mut self, recipes: Vec<RecipeCase<'a>>) {
    for recipe in recipes {
      let start = self.wanted.len();

      self.store.enter_level();
      self.body(recipe.decl, &recipe.signature);
      self.store.leave_level();
      self.solve_closed(start);
    }
  }

  fn impl_bodies(&mut self, decls: &[&'a Decl]) {
    for decl in decls {
      if let Decl::Impl(imp) = decl {
        self.impl_body(imp);
      }
    }
  }

  fn impl_body(&mut self, imp: &'a ImplDecl) {
    let header = imp.trait_name.span.join(&imp.target.span);
    let Some((trait_path, target)) =
      self.impl_heads.get(&(header.start, header.end)).cloned()
    else {
      return;
    };
    let Some(def) = self.env.impls.get(&(trait_path.clone(), target.clone()))
    else {
      return;
    };
    let def = def.clone();
    let Some(trait_def) = self.env.traits.get(&trait_path).cloned() else {
      return;
    };

    self.store.enter_level();

    let mut vars = TypeVars::rigid();
    let args: Vec<Type> = imp
      .target
      .args
      .iter()
      .filter_map(|arg| match arg {
        TypeExpr::Var(v) => {
          let ty = Type::Rigid {
            name: Arc::from(v.name.text.as_str()),
            id: self.fresh_var(),
          };

          vars.names.push((v.name.text.clone(), ty.clone()));
          Some(ty)
        }
        _ => None,
      })
      .collect();
    let target_ty =
      self.target_type(&imp.target.name.text, &target, args.clone());
    let givens: Vec<Pred> = def
      .bounds
      .iter()
      .filter_map(|(path, index)| {
        Some(Pred { trait_path: path.clone(), ty: args.get(*index)?.clone() })
      })
      .collect();

    for method in &imp.methods {
      let Some((_, scheme)) =
        trait_def.methods.iter().find(|(name, _)| *name == method.name.text)
      else {
        continue;
      };
      let mut fill = vec![target_ty.clone()];

      for i in 1..scheme.count {
        fill.push(Type::Rigid {
          name: Arc::from(format!("t{i}").as_str()),
          id: self.fresh_var(),
        });
      }

      let expected = substitute(&scheme.ty, &fill);

      self.record_signature(&method.name.span, &expected);
      let trait_span = self
        .method_spans
        .get(&(trait_path.clone(), method.name.text.clone()))
        .cloned();

      vars.mode = VarMode::Rigid;
      self.vars = vars.clone();
      self.givens.clone_from(&givens);
      self.row_owner = RowOwner::Method {
        name: method.name.text.clone(),
        trait_name: imp.trait_name.text.clone(),
        trait_span: trait_span.clone(),
      };
      self.method_body(method, &expected, trait_span.as_ref());
      self.row_owner = RowOwner::Free;
      self.givens.clear();
    }

    self.vars = TypeVars::flexible();
    self.store.leave_level();
  }

  fn target_type(
    &mut self,
    written: &str,
    target: &Arc<str>,
    args: Vec<Type>,
  ) -> Type {
    match self.env.entries.get(written) {
      Some(Entry::Def(TypeDef::Alias { name, body, .. })) => {
        let mut ty = substitute(body, &[]);

        if let Type::Record(row) = &mut ty {
          row.tail = Tail::Closed(Brand::Named(name.clone()));
        }

        ty
      }
      _ => Type::Con { name: target.clone(), args },
    }
  }

  fn method_body(
    &mut self,
    f: &'a FnDecl,
    expected: &Type,
    trait_span: Option<&Span>,
  ) {
    let Type::Fn { params, ret, effects } = expected else { return };
    let because = |span: Option<&Span>| match span {
      Some(span) => Because::Trait(span.clone(), expected.clone()),
      None => Because::Nothing,
    };

    if params.len() != f.params.len() {
      let args = if params.len() == 1 { "parameter" } else { "parameters" };

      self.push(Diagnostic::error(
        ArgumentCount,
        format!(
          "the trait says `{}` takes {} {args}, but this takes {}",
          f.name.text,
          params.len(),
          f.params.len()
        ),
        Label::new(f.name.span.clone()),
      ));
      return;
    }

    let start = self.wanted.len();

    for (param, ty) in f.params.iter().zip(params) {
      if let Some(annotation) = &param.ty {
        let written = self.convert(annotation);

        self.expect_because(
          ty,
          &written,
          annotation.span(),
          because(trait_span),
        );
      }

      self.bind_mono(&param.name.span, ty.clone());
    }

    if let Some(annotation) = &f.return_type {
      let written = self.convert(annotation);

      self.expect_because(
        ret,
        &written,
        annotation.span(),
        because(trait_span),
      );
    }

    let owner = self.row_owner.clone();
    let (body, _) = self.with_row(effects.clone(), owner, |c| {
      c.with_return(Type::clone(ret), |c| c.block(&f.body))
    });
    let blame = last_expr(&f.body.result).span();

    self.expect_because(ret, &body, blame, because(trait_span));
    self.solve_closed(start);
  }

  fn constant(&mut self, c: &'a ConstDecl, sym: &Sym) {
    let start = self.wanted.len();

    self.vars = TypeVars::rigid();
    self.store.enter_level();

    let owner = RowOwner::Constant { name: c.name.text.clone() };
    let (found, _) =
      self.with_row(Effects::pure(), owner, |ch| ch.infer(&c.value));
    let ty = match &c.ty {
      Some(annotation) => {
        let expected = self.convert(annotation);

        self.expect_because(
          &expected,
          &found,
          c.value.span(),
          Because::Annotation(annotation.span().clone()),
        );
        expected
      }
      None => found,
    };

    self.store.leave_level();
    default_numbers(&mut self.store, &ty);
    default_brands(&mut self.store, &ty);
    self.solve_closed(start);

    let mut scheme = generalise_indexed(&self.store, &ty, &[], true).0;

    if c.ty.is_none() {
      close_row(&mut scheme);
    }

    seal_closed(&mut self.store, &ty, &scheme.ty);
    self.record_signature(&c.name.span, &ty);

    self.locals.insert(sym.id, scheme);
    self.vars = TypeVars::flexible();
  }
}

fn signature_span(f: &FnDecl) -> Span {
  let end = f
    .return_type
    .as_ref()
    .map(|t| t.span().clone())
    .or_else(|| f.params.last().map(|p| p.span.clone()))
    .unwrap_or_else(|| f.name.span.clone());

  f.name.span.join(&end)
}

fn fully_annotated(f: &FnDecl) -> bool {
  f.return_type.is_some() && f.params.iter().all(|p| p.ty.is_some())
}

#[must_use]
pub fn groups(edges: &[Vec<usize>]) -> Vec<Vec<usize>> {
  struct Tarjan<'e> {
    edges: &'e [Vec<usize>],
    index: Vec<Option<usize>>,
    low: Vec<usize>,
    on_stack: Vec<bool>,
    stack: Vec<usize>,
    next: usize,
    out: Vec<Vec<usize>>,
  }

  impl Tarjan<'_> {
    fn visit(&mut self, v: usize) {
      self.index[v] = Some(self.next);
      self.low[v] = self.next;
      self.next += 1;
      self.stack.push(v);
      self.on_stack[v] = true;

      for &w in &self.edges[v] {
        match self.index[w] {
          None => {
            self.visit(w);
            self.low[v] = self.low[v].min(self.low[w]);
          }
          Some(i) if self.on_stack[w] => self.low[v] = self.low[v].min(i),
          Some(_) => {}
        }
      }

      if Some(self.low[v]) == self.index[v] {
        let mut group = Vec::new();

        while let Some(w) = self.stack.pop() {
          self.on_stack[w] = false;
          group.push(w);

          if w == v {
            break;
          }
        }

        group.sort_unstable();
        self.out.push(group);
      }
    }
  }

  let n = edges.len();
  let mut tarjan = Tarjan {
    edges,
    index: vec![None; n],
    low: vec![0; n],
    on_stack: vec![false; n],
    stack: Vec::new(),
    next: 0,
    out: Vec::new(),
  };

  for v in 0..n {
    if tarjan.index[v].is_none() {
      tarjan.visit(v);
    }
  }

  tarjan.out
}

fn recipe_help(case: &FnDecl, result: &Type) -> String {
  let r = Printer::new(&[result], true).print(result);
  let param = |i: usize, default: &str| {
    case.params.get(i).map_or(default.to_string(), |p| p.name.text.clone())
  };

  match case.name.text.as_str() {
    "record" => format!(
      "write `record({}: List<{{ name: String, value: {r} }}>) -> {r}`",
      param(0, "fields")
    ),
    _ => format!(
      "write `variant({}: String, {}: List<{r}>) -> {r}`",
      param(0, "name"),
      param(1, "args")
    ),
  }
}

impl Checker<'_> {
  fn private_leaks(&mut self, module: &Module, out: &[(String, Scheme)]) {
    let exported = module.exported_types();
    let decls: Vec<&Decl> =
      module.zones.iter().flat_map(|z| &z.decls).collect();
    let header = module.name.as_ref().map(|n| n.text.clone());
    let private: Vec<(String, String)> = decls
      .iter()
      .filter_map(|d| match d {
        Decl::Type(t) if !exported.contains(&t.name.text) => {
          Some((self.qualified(&t.name.text), t.name.text.clone()))
        }
        _ => None,
      })
      .flat_map(|(q, b)| {
        let named = header.as_ref().map(|h| (format!("{h}.{b}"), b.clone()));

        std::iter::once((q, b)).chain(named)
      })
      .collect();

    if private.is_empty() {
      return;
    }

    for decl in &decls {
      let Decl::Export(export) = decl else { continue };

      if export.methods.is_some() {
        continue;
      }

      let name = &export.name.text;
      let mut types: Vec<Type> = out
        .iter()
        .filter(|(n, _)| n == name)
        .map(|(_, scheme)| scheme.ty.clone())
        .collect();

      if let Some((_, def)) = self.local_types.iter().find(|(n, _)| n == name) {
        match def {
          TypeDef::Variant { ctors, .. } => {
            types.extend(ctors.iter().map(|(_, s)| s.ty.clone()));
          }
          TypeDef::Alias { body, .. } => types.push(body.clone()),
        }
      }

      types.extend(
        self
          .op_schemes
          .iter()
          .filter(|((effect, _), _)| effect == name)
          .map(|(_, scheme)| scheme.ty.clone()),
      );

      let mut found: Vec<String> = Vec::new();

      for ty in &types {
        mentioned(ty, &private, &mut found);
      }

      found.retain(|bare| bare != name);

      if let Some(bare) = found.first() {
        self.push(
          Diagnostic::error(
            PrivateTypeExposed,
            format!(
              "`{name}` is exported, but it uses the private type `{bare}`"
            ),
            Label::new(export.name.span.clone())
              .with_message(format!("other modules can't name `{bare}`")),
          )
          .with_help(format!("export `{bare}` too: add it to `exports`")),
        );
      }
    }
  }
}

fn mentioned(ty: &Type, private: &[(String, String)], found: &mut Vec<String>) {
  let mut note = |path: &str| {
    if let Some((_, bare)) =
      private.iter().find(|(q, b)| q == path || b == path)
      && !found.contains(bare)
    {
      found.push(bare.clone());
    }
  };

  match ty {
    Type::Con { name, args } => {
      note(name);

      for arg in args {
        mentioned(arg, private, found);
      }
    }
    Type::Fn { params, ret, .. } => {
      for param in params {
        mentioned(param, private, found);
      }

      mentioned(ret, private, found);
    }
    Type::Record(row) => {
      if let Some(brand) = row.brand() {
        note(brand);
      }

      for (_, field) in &row.fields {
        mentioned(field, private, found);
      }
    }
    Type::Var(_) | Type::Gen(_) | Type::Rigid { .. } => {}
  }
}
