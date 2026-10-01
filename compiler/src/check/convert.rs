use std::sync::Arc;

use crate::{
  check::{
    Checker,
    env::{Entry, ImplDef, PRIMITIVES, TraitDef, TypeDef},
  },
  core::{lower::Resolved, scope::suggest},
  shared::codes::DiagnosticCode::{
    BadThrowsType, DuplicateDefinition, DuplicateField, DuplicateImpl,
    NotDerivable, RecursiveAlias, TypeArgumentCount, TypeMismatch,
    UnboundTypeVariable, UnknownEffect, UnknownType,
  },
  shared::diagnostic::{Diagnostic, Label},
  shared::ice::ice,
  shared::source::Span,
  syntax::ast::{
    Decl, EffectRow, FieldType, ImplDecl, Module, Name, TraitDecl, TypeBody,
    TypeDecl, TypeExpr, TypeRef,
  },
  types::ty::{
    Brand, EffTail, Effects, Label as EffectLabel, MUT, Pred, Row, Scheme,
    Tail, Type,
  },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VarMode {
  Params,
  Method,
  Rigid,
  Flexible,
}

#[derive(Debug, Clone)]
pub struct TypeVars {
  pub names: Vec<(String, Type)>,
  pub mode: VarMode,
  pub rows: Vec<String>,
}

impl TypeVars {
  #[must_use]
  pub fn flexible() -> Self {
    Self { names: Vec::new(), mode: VarMode::Flexible, rows: Vec::new() }
  }

  #[must_use]
  pub fn rigid() -> Self {
    Self { names: Vec::new(), mode: VarMode::Rigid, rows: Vec::new() }
  }

  #[must_use]
  pub fn params(params: &[Name]) -> Self {
    let names = params
      .iter()
      .zip(0u32..)
      .map(|(p, i)| (p.text.clone(), Type::Gen(i)))
      .collect();

    Self { names, mode: VarMode::Params, rows: Vec::new() }
  }

  #[must_use]
  pub fn method(param: &Name) -> Self {
    Self {
      names: vec![(param.text.clone(), Type::Gen(0))],
      mode: VarMode::Method,
      rows: Vec::new(),
    }
  }

  #[must_use]
  pub fn row_name(&self, var: crate::types::ty::TVar) -> Option<&str> {
    self.names.iter().find_map(|(name, ty)| match ty {
      Type::Rigid { id, .. } | Type::Var(id)
        if *id == var && self.rows.contains(name) =>
      {
        Some(name.as_str())
      }
      _ => None,
    })
  }

  pub(crate) fn get(&self, name: &str) -> Option<&Type> {
    self.names.iter().find(|(n, _)| n == name).map(|(_, t)| t)
  }
}

impl<'a> Checker<'a> {
  pub(crate) fn build_env(&mut self, module: &'a Module) {
    let interfaces = self.interfaces;

    for interface in interfaces.values() {
      for (bare, def) in &interface.types {
        self.env.add_def(bare, def.clone());
      }
    }

    let decls: Vec<&'a TypeDecl> = module
      .zones
      .iter()
      .flat_map(|zone| &zone.decls)
      .filter_map(|decl| match decl {
        Decl::Type(ty) => Some(ty),
        _ => None,
      })
      .collect();
    let mut declared: Vec<&'a TypeDecl> = Vec::new();

    for decl in &decls {
      let name = &decl.name;

      if PRIMITIVES.contains(&name.text.as_str()) {
        self.push(
          Diagnostic::error(
            DuplicateDefinition,
            format!("the type `{}` is built in", name.text),
            Label::new(name.span.clone()),
          )
          .with_help("choose another name"),
        );
        continue;
      }

      if let Some(first) = declared.iter().find(|d| d.name.text == name.text) {
        self.push(
          Diagnostic::error(
            DuplicateDefinition,
            format!("the type `{}` is defined more than once", name.text),
            Label::new(name.span.clone()),
          )
          .with_secondary(
            Label::new(first.name.span.clone())
              .with_message("first defined here"),
          ),
        );
        continue;
      }

      declared.push(decl);
      self.env.entries.insert(name.text.clone(), Entry::Pending(decl));

      if let TypeBody::Alias(TypeExpr::Record(record)) = &decl.body {
        let owner = self.qualified(&name.text);

        for field in &record.fields {
          self.field_spans.insert(
            (owner.clone(), field.name.text.clone()),
            field.span.clone(),
          );
        }
      }
    }

    for decl in declared {
      let def = self.resolve_local(&decl.name.text);

      if let Some(def) = def {
        self.local_types.push((decl.name.text.clone(), def));
      }
    }

    self.derive_shapes(&decls);
    self.build_traits(module);
  }

  fn derive_shapes(&mut self, decls: &[&'a TypeDecl]) {
    for decl in decls {
      let Some(derive) = decl.derive.as_ref().filter(|d| !d.names.is_empty())
      else {
        continue;
      };
      let message = match &decl.body {
        TypeBody::Variants(_) => continue,
        TypeBody::Alias(TypeExpr::Record(_)) if decl.params.is_empty() => {
          continue;
        }
        TypeBody::Alias(TypeExpr::Record(_)) => format!(
          "derive on a generic record alias isn't supported yet; make it a \
           variant: `{name}<{params}> = {name}({{ … }})`",
          name = decl.name.text,
          params = decl
            .params
            .iter()
            .map(|p| p.text.as_str())
            .collect::<Vec<_>>()
            .join(", "),
        ),
        TypeBody::Alias(_) => format!(
          "`{}` is an alias of another type, so it can't derive",
          decl.name.text
        ),
      };

      self.push(
        Diagnostic::error(
          NotDerivable,
          message,
          Label::new(derive.span.clone()),
        )
        .with_help("derive works on variants and on record aliases"),
      );
    }
  }

  fn build_traits(&mut self, module: &'a Module) {
    let interfaces = self.interfaces;

    for (path, interface) in interfaces {
      for sig in &interface.traits {
        let trait_path: Arc<str> = Arc::from(format!("{path}.{}", sig.name));

        self.env.traits.insert(
          trait_path.clone(),
          TraitDef {
            path: trait_path,
            methods: sig.schemes.clone(),
            recipe: sig.recipe.clone(),
          },
        );
      }

      for def in &interface.impls {
        self
          .env
          .impls
          .entry((def.trait_path.clone(), def.target.clone()))
          .or_insert_with(|| def.clone());
      }
    }

    let decls: Vec<&'a Decl> =
      module.zones.iter().flat_map(|zone| &zone.decls).collect();

    for decl in &decls {
      if let Decl::Trait(t) = decl {
        self.local_traits.insert(t.name.text.clone());
      }
    }

    for decl in &decls {
      if let Decl::Trait(t) = decl {
        self.trait_def(t);
      }
    }

    for decl in &decls {
      if let Decl::Impl(imp) = decl {
        self.impl_def(imp);
      }
    }
  }

  fn trait_def(&mut self, t: &'a TraitDecl) {
    let path = self.trait_path(&t.name.text);

    if self.env.traits.contains_key(&path) {
      return;
    }

    let mut methods = Vec::new();

    for method in &t.methods {
      let saved = std::mem::replace(&mut self.vars, TypeVars::method(&t.param));
      let params: Vec<Type> = method
        .params
        .iter()
        .map(|p| match &p.ty {
          Some(ty) => self.convert(ty),
          None => self.fresh(),
        })
        .collect();
      let ret = self.convert(&method.return_type);
      let effects = self.convert_effects(method.effects.as_ref());
      let count = u32::try_from(self.vars.names.len()).unwrap_or(u32::MAX);

      self.vars = saved;
      self
        .method_spans
        .insert((path.clone(), method.name.text.clone()), method.span.clone());
      methods.push((
        method.name.text.clone(),
        Scheme {
          count,
          ty: Type::func_with(params, ret, effects),
          preds: vec![Pred { trait_path: path.clone(), ty: Type::Gen(0) }],
        },
      ));
    }

    self
      .env
      .traits
      .insert(path.clone(), TraitDef { path, methods, recipe: Vec::new() });
  }

  fn resolved_trait(&self, name: &Name) -> Option<Arc<str>> {
    match self.resolutions.get(&name.span) {
      Some(Resolved::Trait(path)) => Some(self.trait_path(path)),
      _ => None,
    }
  }

  fn impl_def(&mut self, imp: &'a ImplDecl) {
    let Some(trait_path) = self.resolved_trait(&imp.trait_name) else {
      return;
    };
    let header = imp.trait_name.span.join(&imp.target.span);
    let mut vars: Vec<&Name> = Vec::new();

    for arg in &imp.target.args {
      match arg {
        TypeExpr::Var(v) if !vars.iter().any(|n| n.text == v.name.text) => {
          vars.push(&v.name);
        }
        _ => {
          let written = imp.target.name.text.clone();
          let params: Vec<&str> = (0..imp.target.args.len())
            .map(|i| ["a", "b", "c", "d", "e"].get(i).copied().unwrap_or("x"))
            .collect();

          self.push(
            Diagnostic::error(
              TypeArgumentCount,
              "an impl covers the whole type, so its type arguments must be \
               distinct type variables",
              Label::new(imp.target.span.clone()),
            )
            .with_help(format!(
              "impls cover the whole type: write `{written}<{}>`",
              params.join(", ")
            )),
          );
          return;
        }
      }
    }

    let Some((target, arity)) = self.impl_target(&imp.target) else {
      return;
    };

    if arity != vars.len() {
      self.type_argument_count(
        &imp.target.name.text,
        arity,
        vars.len(),
        &imp.target.span,
      );
      return;
    }

    let mut bounds = Vec::new();

    for bound in &imp.bounds {
      let Some(index) = vars.iter().position(|v| v.text == bound.var.text)
      else {
        self.unbound_in_impl(&bound.var, &imp.target.name.text);
        continue;
      };
      let Some(bound_path) = self.resolved_trait(&bound.trait_name) else {
        continue;
      };

      bounds.push((bound_path, index));
    }

    let key = (trait_path.clone(), target.clone());

    if self.env.impls.contains_key(&key) {
      self.push(
        Diagnostic::error(
          DuplicateImpl,
          format!(
            "`{}` already has a `{}` impl",
            imp.target.name.text, imp.trait_name.text
          ),
          Label::new(header),
        )
        .with_note("an imported module already provides it"),
      );
      return;
    }

    let def = ImplDef {
      trait_path: trait_path.clone(),
      target: target.clone(),
      arity,
      bounds,
      module: self.qualifier.clone(),
    };

    self.env.impls.insert(key, def);
    self.impl_heads.insert((header.start, header.end), (trait_path, target));
  }

  fn unbound_in_impl(&mut self, var: &Name, target: &str) {
    self.push(
      Diagnostic::error(
        UnboundTypeVariable,
        format!("the type variable `{}` is not defined", var.text),
        Label::new(var.span.clone()),
      )
      .with_help(format!(
        "a bound can only name a variable of the impl's type, like `{target}<{}>`",
        var.text
      )),
    );
  }

  pub(crate) fn impl_target(
    &mut self,
    r: &TypeRef,
  ) -> Option<(Arc<str>, usize)> {
    let name = &r.name.text;

    match self.env.entries.get(name).cloned() {
      Some(Entry::Prim) => Some((Arc::from(name.as_str()), 0)),
      Some(Entry::Def(TypeDef::Variant { name, arity, .. })) => {
        Some((name, arity))
      }
      Some(Entry::Def(TypeDef::Alias { name, arity, body })) => match body {
        Type::Record(_) if arity == 0 => Some((name, 0)),
        _ => {
          self.push(
            Diagnostic::error(
              UnknownType,
              format!(
                "`{}` is an alias, so it can't have an impl",
                r.name.text
              ),
              Label::new(r.name.span.clone()),
            )
            .with_help("impls go on variants and on record aliases"),
          );
          None
        }
      },
      Some(Entry::Pending(_) | Entry::Converting(_)) | None => {
        let help = suggest(name, self.env.names())
          .map(|s| format!("did you mean `{s}`?"));
        let mut diagnostic = Diagnostic::error(
          UnknownType,
          format!("cannot find type `{name}`"),
          Label::new(r.name.span.clone()),
        );

        if let Some(help) = help {
          diagnostic = diagnostic.with_help(help);
        }

        self.push(diagnostic);
        None
      }
    }
  }

  fn resolve_local(&mut self, name: &str) -> Option<TypeDef> {
    match self.env.entries.get(name).cloned() {
      Some(Entry::Def(def)) => Some(def),
      Some(Entry::Pending(decl)) => {
        self.env.entries.insert(name.to_string(), Entry::Converting(decl));

        let def = self.convert_decl(decl);

        self.env.add_def(name, def.clone());
        Some(def)
      }
      Some(Entry::Converting(decl)) => {
        self.push(
          Diagnostic::error(
            RecursiveAlias,
            format!("the type alias `{}` refers to itself", decl.name.text),
            Label::new(decl.name.span.clone()),
          )
          .with_help(
            "make it a variant, like `Node = Node({ … })`, so it has a name \
             to stop at",
          ),
        );
        None
      }
      Some(Entry::Prim) | None => None,
    }
  }

  fn convert_decl(&mut self, decl: &TypeDecl) -> TypeDef {
    let saved =
      std::mem::replace(&mut self.vars, TypeVars::params(&decl.params));
    let declaring = self.declaring.replace(decl.name.text.clone());
    let arity = decl.params.len();

    let def = match &decl.body {
      TypeBody::Alias(expr) => TypeDef::Alias {
        name: Arc::from(self.qualified(&decl.name.text)),
        arity,
        body: self.convert(expr),
      },
      TypeBody::Variants(variants) => {
        let name: Arc<str> = Arc::from(self.qualified(&decl.name.text));
        let result = Type::Con {
          name: name.clone(),
          args: (0..arity)
            .map(|i| Type::Gen(u32::try_from(i).unwrap_or(u32::MAX)))
            .collect(),
        };
        let count = u32::try_from(arity).unwrap_or(u32::MAX);
        let ctors = variants
          .ctors
          .iter()
          .map(|ctor| {
            let params: Vec<Type> =
              ctor.args.iter().map(|a| self.convert(a)).collect();
            let ty = if params.is_empty() {
              result.clone()
            } else {
              Type::func(params, result.clone())
            };

            (ctor.name.text.clone(), Scheme::new(count, ty))
          })
          .collect();

        TypeDef::Variant { name, arity, ctors }
      }
    };

    self.vars = saved;
    self.declaring = declaring;
    def
  }

  pub(crate) fn convert(&mut self, expr: &TypeExpr) -> Type {
    match expr {
      TypeExpr::Ref(r) => self.convert_ref(r),
      TypeExpr::Var(v) => self.type_var(&v.name),
      TypeExpr::Fn(f) => {
        let params = f.params.iter().map(|p| self.convert(p)).collect();
        let ret = self.convert(&f.ret);
        let effects = self.convert_effects(f.effects.as_ref());

        Type::func_with(params, ret, effects)
      }
      TypeExpr::Record(r) => {
        let names: Vec<&Name> = r.fields.iter().map(|f| &f.name).collect();
        self.duplicate_fields(&names);

        let mut fields: Vec<(Arc<str>, Type)> = Vec::new();

        for FieldType { name, ty, .. } in &r.fields {
          let ty = self.convert(ty);

          if !fields.iter().any(|(n, _)| **n == *name.text) {
            fields.push((Arc::from(name.text.as_str()), ty));
          }
        }

        let tail = match &r.tail {
          None => Tail::anonymous(),
          Some(name) => self.tail_var(name),
        };

        Type::Record(Row::new(fields, tail))
      }
      TypeExpr::Invalid(invalid) => {
        ice("an invalid type reached the checker", Some(&invalid.span))
      }
    }
  }

  fn type_var(&mut self, name: &Name) -> Type {
    if self.vars.rows.contains(&name.text) {
      self.row_clash(name);
      return self.fresh();
    }

    if let Some(ty) = self.vars.get(&name.text) {
      return ty.clone();
    }

    let ty = match self.vars.mode {
      VarMode::Params => {
        self.push(
          Diagnostic::error(
            UnboundTypeVariable,
            format!("the type variable `{}` is not defined", name.text),
            Label::new(name.span.clone()),
          )
          .with_help(format!(
            "add it to the type's parameters, like `{}<{}>`",
            self.declaring.as_deref().unwrap_or("Name"),
            name.text
          )),
        );
        return self.fresh();
      }
      VarMode::Method => {
        Type::Gen(u32::try_from(self.vars.names.len()).unwrap_or(u32::MAX))
      }
      VarMode::Rigid => Type::Rigid {
        name: Arc::from(name.text.as_str()),
        id: self.fresh_var(),
      },
      VarMode::Flexible => self.fresh(),
    };

    self.vars.names.push((name.text.clone(), ty.clone()));
    ty
  }

  fn row_clash(&mut self, name: &Name) {
    self.push(
      Diagnostic::error(
        TypeMismatch,
        format!("`{}` is used as both a type and an effect row", name.text),
        Label::new(name.span.clone()),
      )
      .with_help("pick a different letter for one of them"),
    );
  }

  fn effect_tail(&mut self, name: &Name) -> EffTail {
    if self.vars.get(&name.text).is_some()
      && !self.vars.rows.contains(&name.text)
    {
      self.row_clash(name);
      return EffTail::Open(self.fresh_var());
    }

    let ty = if let Some(ty) = self.vars.get(&name.text) {
      ty.clone()
    } else {
      let ty = self.type_var(name);

      self.vars.rows.push(name.text.clone());
      ty
    };

    match ty {
      Type::Var(v) => EffTail::Open(v),
      Type::Rigid { id, .. } => EffTail::Rigid(id),
      Type::Gen(i) => EffTail::Gen(i),
      Type::Con { .. } | Type::Fn { .. } | Type::Record(_) => {
        EffTail::Open(self.fresh_var())
      }
    }
  }

  pub(crate) fn convert_effects(&mut self, row: Option<&EffectRow>) -> Effects {
    let Some(row) = row else { return Effects::pure() };
    let mut labels: Vec<EffectLabel> = Vec::new();

    for entry in &row.entries {
      let label = if entry.name.text == "Throws" {
        self.throws_entry(entry)
      } else if entry.name.text == MUT && entry.args.is_empty() {
        Some(EffectLabel::effect(MUT))
      } else {
        self.effect_entry(entry)
      };
      let Some(label) = label else { continue };

      if labels.contains(&label) {
        self.push(
          Diagnostic::error(
            DuplicateDefinition,
            format!("`{}` appears twice in this row", label_text(&label)),
            Label::new(entry.span.clone()),
          )
          .with_help("a row is a set: list each effect once"),
        );
        continue;
      }

      labels.push(label);
    }

    let tail = match &row.tail {
      Some(name) => self.effect_tail(name),
      None => EffTail::Closed,
    };

    Effects::new(labels, tail)
  }

  fn effect_entry(&mut self, entry: &TypeRef) -> Option<EffectLabel> {
    let name = &entry.name.text;

    if !self.effects.is_effect(name) {
      let known: Vec<&str> =
        self.effects.effects.keys().map(String::as_str).collect();
      let help = suggest(name, known.iter().copied()).map_or_else(
        || "effects are declared in an `effects` zone".to_string(),
        |s| format!("did you mean `{s}`?"),
      );

      self.push(
        Diagnostic::error(
          UnknownEffect,
          format!("there is no effect `{name}`"),
          Label::new(entry.name.span.clone()),
        )
        .with_help(help),
      );

      return None;
    }

    if !entry.args.is_empty() {
      self.type_argument_count(name, 0, entry.args.len(), &entry.span);
    }

    Some(EffectLabel::effect(name))
  }

  fn throws_entry(&mut self, entry: &TypeRef) -> Option<EffectLabel> {
    let [arg] = entry.args.as_slice() else {
      self.push(Diagnostic::error(
        BadThrowsType,
        "`Throws` needs an error type: `Throws<E>`",
        Label::new(entry.span.clone()),
      ));
      return None;
    };

    let named = match arg {
      TypeExpr::Ref(r) if r.args.is_empty() => Some(r),
      _ => None,
    };
    let Some(r) = named else {
      self.push(
        Diagnostic::error(
          BadThrowsType,
          "`Throws` needs a type without parameters",
          Label::new(arg.span().clone()),
        )
        .with_help("wrap it: `IntError = IntError(List<Int>)`"),
      );
      return None;
    };
    let ty = self.convert_ref(r);
    let label = self.throws_label(&ty);

    if label.is_none() {
      self.bad_throws(&r.name.text, arg.span());
    }

    label
  }

  pub(crate) fn bad_throws(&mut self, written: &str, span: &Span) {
    self.push(
      Diagnostic::error(
        BadThrowsType,
        format!("`Throws` needs a named error type, not `{written}`"),
        Label::new(span.clone()),
      )
      .with_help(
        "declare one in `types`, like `ParseError = ParseError(String)`",
      ),
    );
  }

  pub(crate) fn throws_label(&self, ty: &Type) -> Option<EffectLabel> {
    match self.store.resolve(ty) {
      Type::Con { name, args }
        if args.is_empty() && !PRIMITIVES.contains(&&*name) =>
      {
        Some(EffectLabel::throws(&self.error_path(&name)))
      }
      _ => None,
    }
  }

  pub(crate) fn error_path(&self, name: &str) -> String {
    if name.contains('.') {
      return name.to_string();
    }

    match &self.module_name {
      Some(module) => format!("{module}.{name}"),
      None => name.to_string(),
    }
  }

  fn tail_var(&mut self, name: &Name) -> Tail {
    match self.type_var(name) {
      Type::Var(v) => Tail::Open(v),
      Type::Rigid { id, .. } => Tail::Rigid(id),
      Type::Gen(i) => Tail::Gen(i),
      Type::Con { .. } | Type::Fn { .. } | Type::Record(_) => Tail::anonymous(),
    }
  }

  fn convert_ref(&mut self, r: &TypeRef) -> Type {
    let name = &r.name.text;

    let Some(entry) = self.env.entries.get(name).cloned() else {
      for arg in &r.args {
        self.convert(arg);
      }

      let help =
        suggest(name, self.env.names()).map(|s| format!("did you mean `{s}`?"));
      let mut diagnostic = Diagnostic::error(
        UnknownType,
        format!("cannot find type `{name}`"),
        Label::new(r.name.span.clone()),
      );

      if let Some(help) = help {
        diagnostic = diagnostic.with_help(help);
      }

      self.push(diagnostic);
      return self.fresh();
    };

    let arity = match &entry {
      Entry::Prim => 0,
      Entry::Def(def) => def.arity(),
      Entry::Pending(decl) | Entry::Converting(decl) => decl.params.len(),
    };

    if r.args.len() != arity {
      for arg in &r.args {
        self.shallow(arg);
      }

      self.type_argument_count(name, arity, r.args.len(), &r.span);
      return self.fresh();
    }

    if self.shallow {
      return self.fresh();
    }

    let def = match entry {
      Entry::Prim => return Type::con(name),
      Entry::Def(def) => Some(def),
      Entry::Pending(decl) | Entry::Converting(decl)
        if matches!(decl.body, TypeBody::Variants(_)) =>
      {
        let phantom: Vec<bool> =
          decl.params.iter().map(|p| !declared_uses(decl, &p.text)).collect();
        let args = self.variant_args(&r.args, &phantom);

        return Type::Con {
          name: Arc::from(self.qualified(&decl.name.text)),
          args,
        };
      }
      Entry::Pending(_) | Entry::Converting(_) => self.resolve_local(name),
    };

    match def {
      Some(TypeDef::Variant { name, ctors, .. }) => {
        let phantom: Vec<bool> = (0..r.args.len())
          .map(|i| {
            let i = u32::try_from(i).unwrap_or(u32::MAX);

            !ctors.is_empty()
              && !ctors.iter().any(|(_, scheme)| match &scheme.ty {
                Type::Fn { params, .. } => params.iter().any(|p| uses(p, i)),
                _ => false,
              })
          })
          .collect();
        let args = self.variant_args(&r.args, &phantom);

        Type::Con { name, args }
      }
      Some(TypeDef::Alias { name, body, .. }) => {
        let mut args = Vec::with_capacity(r.args.len());

        for (i, arg) in r.args.iter().enumerate() {
          if uses(&body, u32::try_from(i).unwrap_or(u32::MAX)) {
            args.push(self.convert(arg));
          } else {
            self.shallow(arg);
            args.push(Type::unit());
          }
        }

        expand(&name, &body, &args)
      }
      None => {
        for arg in &r.args {
          self.shallow(arg);
        }

        self.fresh()
      }
    }
  }

  fn variant_args(&mut self, args: &[TypeExpr], phantom: &[bool]) -> Vec<Type> {
    args
      .iter()
      .enumerate()
      .map(|(i, arg)| {
        let nominal = phantom
          .get(i)
          .copied()
          .unwrap_or(false)
          .then(|| self.nominal_alias(arg))
          .flatten();

        nominal.unwrap_or_else(|| self.convert(arg))
      })
      .collect()
  }

  fn nominal_alias(&self, arg: &TypeExpr) -> Option<Type> {
    let TypeExpr::Ref(r) = arg else { return None };

    if !r.args.is_empty() {
      return None;
    }

    let name: Arc<str> = match self.env.entries.get(&r.name.text)? {
      Entry::Pending(decl) | Entry::Converting(decl)
        if decl.params.is_empty()
          && matches!(decl.body, TypeBody::Alias(TypeExpr::Record(_))) =>
      {
        Arc::from(self.qualified(&decl.name.text))
      }
      Entry::Def(TypeDef::Alias { name, arity: 0, body: Type::Record(_) }) => {
        name.clone()
      }
      _ => return None,
    };

    Some(Type::Con { name, args: Vec::new() })
  }

  fn shallow(&mut self, expr: &TypeExpr) {
    let saved = std::mem::replace(&mut self.shallow, true);

    self.convert(expr);
    self.shallow = saved;
  }

  fn type_argument_count(
    &mut self,
    name: &str,
    expected: usize,
    found: usize,
    span: &Span,
  ) {
    let args = if expected == 1 { "type argument" } else { "type arguments" };
    let were = if found == 1 { "was" } else { "were" };

    self.push(Diagnostic::error(
      TypeArgumentCount,
      format!("`{name}` takes {expected} {args} but {found} {were} given"),
      Label::new(span.clone()),
    ));
  }

  pub(crate) fn duplicate_fields(&mut self, names: &[&Name]) {
    for (i, name) in names.iter().enumerate() {
      if let Some(first) = names[..i].iter().find(|n| n.text == name.text) {
        self.push(
          Diagnostic::error(
            DuplicateField,
            format!("field `{}` is given twice", name.text),
            Label::new(name.span.clone()),
          )
          .with_secondary(
            Label::new(first.span.clone()).with_message("first given here"),
          ),
        );
      }
    }
  }
}

fn declared_uses(decl: &TypeDecl, param: &str) -> bool {
  let TypeBody::Variants(variants) = &decl.body else { return true };

  if variants.ctors.is_empty() {
    return true;
  }

  variants.ctors.iter().flat_map(|c| &c.args).any(|a| mentions(a, param))
}

fn mentions(expr: &TypeExpr, param: &str) -> bool {
  match expr {
    TypeExpr::Var(v) => v.name.text == param,
    TypeExpr::Ref(r) => r.args.iter().any(|a| mentions(a, param)),
    TypeExpr::Fn(f) => {
      f.params.iter().any(|p| mentions(p, param)) || mentions(&f.ret, param)
    }
    TypeExpr::Record(r) => {
      r.fields.iter().any(|f| mentions(&f.ty, param))
        || r.tail.as_ref().is_some_and(|t| t.text == param)
    }
    TypeExpr::Invalid(_) => false,
  }
}

fn uses(ty: &Type, i: u32) -> bool {
  match ty {
    Type::Gen(j) => *j == i,
    Type::Var(_) | Type::Rigid { .. } => false,
    Type::Con { args, .. } => args.iter().any(|a| uses(a, i)),
    Type::Fn { params, ret, effects } => {
      effects.tail == EffTail::Gen(i)
        || params.iter().any(|p| uses(p, i))
        || uses(ret, i)
    }
    Type::Record(row) => {
      row.tail == Tail::Gen(i) || row.fields.iter().any(|(_, t)| uses(t, i))
    }
  }
}

fn expand(name: &Arc<str>, body: &Type, args: &[Type]) -> Type {
  let mut ty = substitute(body, args);

  if args.is_empty()
    && let Type::Record(row) = &mut ty
    && row.tail.is_closed()
  {
    row.tail = Tail::Closed(Brand::Named(name.clone()));
  }

  ty
}

#[must_use]
pub fn substitute(ty: &Type, args: &[Type]) -> Type {
  match ty {
    Type::Gen(i) => args.get(*i as usize).cloned().unwrap_or(Type::Gen(*i)),
    Type::Var(_) | Type::Rigid { .. } => ty.clone(),
    Type::Con { name, args: inner } => Type::Con {
      name: name.clone(),
      args: inner.iter().map(|a| substitute(a, args)).collect(),
    },
    Type::Fn { params, ret, effects } => {
      let mut labels = effects.labels.clone();
      let tail = match effects.tail {
        EffTail::Gen(i) => match args.get(i as usize) {
          Some(Type::Var(v)) => EffTail::Open(*v),
          Some(Type::Rigid { id, .. }) => EffTail::Rigid(*id),
          Some(Type::Gen(j)) => EffTail::Gen(*j),
          Some(Type::Con { .. } | Type::Fn { .. } | Type::Record(_)) | None => {
            EffTail::Gen(i)
          }
        },
        tail => tail,
      };

      labels.sort();
      Type::Fn {
        params: params.iter().map(|p| substitute(p, args)).collect(),
        ret: Box::new(substitute(ret, args)),
        effects: Effects::new(labels, tail),
      }
    }
    Type::Record(row) => {
      let mut fields: Vec<(Arc<str>, Type)> = row
        .fields
        .iter()
        .map(|(n, t)| (n.clone(), substitute(t, args)))
        .collect();
      let tail = match &row.tail {
        Tail::Gen(i) => match args.get(*i as usize) {
          Some(Type::Var(v)) => Tail::Open(*v),
          Some(Type::Rigid { id, .. }) => Tail::Rigid(*id),
          Some(Type::Record(more)) => {
            fields.extend(more.fields.iter().cloned());
            more.tail.clone()
          }
          Some(Type::Gen(j)) => Tail::Gen(*j),
          Some(Type::Con { .. } | Type::Fn { .. }) | None => Tail::anonymous(),
        },
        tail @ (Tail::Closed(_) | Tail::Open(_) | Tail::Rigid(_)) => {
          tail.clone()
        }
      };

      Type::Record(Row { fields, tail })
    }
  }
}

#[must_use]
pub fn label_text(label: &EffectLabel) -> String {
  match &label.arg {
    Some(arg) => format!("{}<{}>", label.name, crate::types::print::bare(arg)),
    None => label.name.to_string(),
  }
}
