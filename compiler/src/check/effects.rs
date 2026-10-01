use std::{collections::BTreeMap, sync::Arc};

use crate::{
  check::{
    BridgeOp, Checker,
    convert::{TypeVars, VarMode, label_text, substitute},
    exhaustive::Site,
    last_expr,
    report::Because,
    solve::Evidence,
  },
  core::lower::Resolved,
  shared::codes::DiagnosticCode::{
    BadThrowsType, BridgeNotWireSafe, CatchNeedsType, EffectNotAllowed,
  },
  shared::diagnostic::{Diagnostic, Label},
  shared::source::Span,
  syntax::ast::{BindDecl, Decl, FnDecl, Pattern, Throw, Try},
  types::{
    generalise::{instantiate, instantiate_open},
    print::{Printer, bare},
    ty::{EffTail, Effects, Label as EffectLabel, Pred, Scheme, TVar, Type},
    unify::{unify, unify_effects},
  },
};

#[derive(Debug, Clone)]
pub(crate) enum RowOwner {
  Free,
  Function { name: String, row: Option<Span>, params: Vec<(String, Type)> },
  Lambda { row: Option<Span> },
  Constant { name: String },
  Method { name: String, trait_name: String, trait_span: Option<Span> },
}

struct Uses {
  names: String,
  used: String,
  primary: Label,
  full: String,
  listed: String,
  allowed: String,
  tail_name: String,
  ambient: Effects,
}

enum ArmType {
  Loose,
  Unresolved,
  Other,
  Typed(Type),
}

#[must_use]
pub fn error_tag(path: &str) -> String {
  let parts: Vec<&str> = path.split('.').collect();

  match parts.as_slice() {
    [.., module, name] => format!("{module}.{name}"),
    _ => path.to_string(),
  }
}

fn join_labels(labels: &[String]) -> String {
  let quoted: Vec<String> = labels.iter().map(|l| format!("`{l}`")).collect();

  match quoted.as_slice() {
    [] => String::new(),
    [one] => one.clone(),
    [init @ .., last] => format!("{} and {last}", init.join(", ")),
  }
}

fn open_if_closed(checker: &mut Checker<'_>, row: &Effects) -> Effects {
  match row.tail {
    EffTail::Closed => {
      Effects::new(row.labels.clone(), EffTail::Open(checker.fresh_var()))
    }
    EffTail::Open(_) | EffTail::Gen(_) | EffTail::Rigid(_) => row.clone(),
  }
}

fn mentions_row(ty: &Type, var: TVar) -> bool {
  match ty {
    Type::Fn { params, ret, effects } => {
      matches!(effects.tail, EffTail::Rigid(v) | EffTail::Open(v) if v == var)
        || params.iter().any(|p| mentions_row(p, var))
        || mentions_row(ret, var)
    }
    Type::Con { args, .. } => args.iter().any(|a| mentions_row(a, var)),
    Type::Record(row) => row.fields.iter().any(|(_, t)| mentions_row(t, var)),
    Type::Var(_) | Type::Gen(_) | Type::Rigid { .. } => false,
  }
}

impl<'a> Checker<'a> {
  pub(crate) fn with_ambient<T>(
    &mut self,
    row: Effects,
    f: impl FnOnce(&mut Self) -> T,
  ) -> (T, Effects) {
    let owner = self.row_owner.clone();

    self.with_row(row, owner, f)
  }

  pub(crate) fn with_row<T>(
    &mut self,
    row: Effects,
    owner: RowOwner,
    f: impl FnOnce(&mut Self) -> T,
  ) -> (T, Effects) {
    let outer = std::mem::replace(&mut self.ambient, row);
    let outer_owner = std::mem::replace(&mut self.row_owner, owner);
    let outer_uses = std::mem::take(&mut self.first_use);
    let result = f(self);
    let inner = std::mem::replace(&mut self.ambient, outer);

    self.row_owner = outer_owner;
    self.last_uses = std::mem::replace(&mut self.first_use, outer_uses);

    (result, inner)
  }

  pub(crate) fn perform(&mut self, row: &Effects, span: &Span) {
    self.perform_at(row, span, span);
  }

  pub(crate) fn perform_at(&mut self, row: &Effects, span: &Span, at: &Span) {
    let zonked = self.store.zonk_effects(row);
    let opened = open_if_closed(self, &zonked);
    let ambient = self.ambient.clone();

    for label in &zonked.labels {
      self.first_use.note(label, at);
    }

    if unify_effects(&mut self.store, &ambient, &opened).is_err() {
      let diagnostic = self.effect_not_allowed(&zonked, span);

      self.push(diagnostic);
    }
  }

  fn uses_of(&self, row: &Effects, span: &Span) -> Uses {
    let ambient = self.store.zonk_effects(&self.ambient);
    let extra: Vec<EffectLabel> = row
      .labels
      .iter()
      .filter(|l| !ambient.labels.contains(l))
      .cloned()
      .collect();
    let texts: Vec<String> = extra.iter().map(label_text).collect();
    let names = if texts.is_empty() {
      "effects".to_string()
    } else {
      join_labels(&texts)
    };
    let used = if texts.is_empty() {
      "effects its signature can't promise".to_string()
    } else {
      names.clone()
    };
    let primary = Label::new(span.clone()).with_message(if texts.is_empty() {
      "this may use any effect".to_string()
    } else {
      format!("this uses {names}")
    });
    let mut all_labels = ambient.labels.clone();

    all_labels.extend(extra);
    all_labels.sort();

    let tail_name = match ambient.tail {
      EffTail::Rigid(v) | EffTail::Open(v) => {
        self.vars.row_name(v).map_or_else(|| "e".to_string(), str::to_string)
      }
      EffTail::Closed | EffTail::Gen(_) => "e".to_string(),
    };

    Uses {
      names,
      used,
      primary,
      full: all_labels.iter().map(label_text).collect::<Vec<_>>().join(", "),
      listed: texts.join(", "),
      allowed: ambient
        .labels
        .iter()
        .map(label_text)
        .collect::<Vec<_>>()
        .join(", "),
      tail_name,
      ambient,
    }
  }

  fn effect_not_allowed(&mut self, row: &Effects, span: &Span) -> Diagnostic {
    let u = self.uses_of(row, span);
    let pure_note = "a signature with `->` and no `/ {…}` promises no effects";

    match self.row_owner.clone() {
      RowOwner::Function { name, row: None, .. } => Diagnostic::error(
        EffectNotAllowed,
        format!("`{name}` uses {}, but its signature says it is pure", u.used),
        u.primary,
      )
      .with_note(pure_note)
      .with_help(format!("add `/ {{{}}}` after the return type", u.listed)),
      RowOwner::Function { name, row: Some(row_span), params } => {
        if let EffTail::Rigid(var) = u.ambient.tail {
          return self.rigid_row_error(&name, &row_span, &params, var, u);
        }

        Diagnostic::error(
          EffectNotAllowed,
          format!("`{name}` uses {}, which its signature doesn't list", u.used),
          u.primary,
        )
        .with_secondary(
          Label::new(row_span)
            .with_message(format!("the signature allows `{{{}}}`", u.allowed)),
        )
        .with_help(format!("add {} to the row: `/ {{{}}}`", u.names, u.full))
      }
      RowOwner::Constant { name } => Diagnostic::error(
        EffectNotAllowed,
        format!(
          "the constant `{name}` uses {}; constants must be pure",
          u.used
        ),
        u.primary,
      )
      .with_help("move it into a function"),
      RowOwner::Method { name, trait_name, trait_span } => {
        let message = if u.ambient.labels.is_empty() {
          format!("`{name}` uses {}, but `{trait_name}.{name}` is pure", u.used)
        } else {
          format!(
            "`{name}` uses {}, which `{trait_name}.{name}` doesn't list",
            u.used
          )
        };
        let diagnostic =
          Diagnostic::error(EffectNotAllowed, message, u.primary);

        match trait_span {
          Some(span) => diagnostic.with_secondary(
            Label::new(span)
              .with_message("the trait declares its effects here"),
          ),
          None => diagnostic,
        }
      }
      RowOwner::Lambda { row: None } => Diagnostic::error(
        EffectNotAllowed,
        format!(
          "this function uses {}, but its signature says it is pure",
          u.used
        ),
        u.primary,
      )
      .with_note(pure_note)
      .with_help(format!("add `/ {{{}}}` after the return type", u.listed)),
      RowOwner::Lambda { row: Some(row_span) } => Diagnostic::error(
        EffectNotAllowed,
        format!(
          "this function uses {}, which its signature doesn't list",
          u.used
        ),
        u.primary,
      )
      .with_secondary(Label::new(row_span).with_message("the signature's row"))
      .with_help(format!("add {} to the row: `/ {{{}}}`", u.names, u.full)),
      RowOwner::Free => Diagnostic::error(
        EffectNotAllowed,
        format!("this uses {}, which isn't allowed here", u.used),
        u.primary,
      ),
    }
  }

  fn rigid_row_error(
    &self,
    name: &str,
    row_span: &Span,
    params: &[(String, Type)],
    var: TVar,
    u: Uses,
  ) -> Diagnostic {
    let param = params
      .iter()
      .find(|(_, ty)| mentions_row(&self.store.zonk(ty), var))
      .map(|(p, _)| p.clone());
    let allows = match param {
      Some(param) => format!("what `{param}` does"),
      None => format!("`{}`", u.tail_name),
    };

    Diagnostic::error(
      EffectNotAllowed,
      format!("`{name}` uses {}, but its row only allows {allows}", u.used),
      u.primary,
    )
    .with_secondary(
      Label::new(row_span.clone()).with_message("the signature's row"),
    )
    .with_help(format!(
      "add {} before the `|`: `/ {{{} | {}}}`",
      u.names, u.full, u.tail_name
    ))
  }

  pub(crate) fn build_effects(&mut self) {
    let table = self.effects;

    for (effect, info) in &table.effects {
      if info.local {
        for sig in &info.sigs {
          let saved = std::mem::replace(
            &mut self.vars,
            TypeVars {
              names: Vec::new(),
              mode: VarMode::Method,
              rows: Vec::new(),
            },
          );
          let params: Vec<Type> = sig
            .params
            .iter()
            .map(|p| match &p.ty {
              Some(ty) => self.convert(ty),
              None => self.fresh(),
            })
            .collect();
          let ret = self.convert(&sig.return_type);
          let own = self.convert_effects(sig.effects.as_ref());
          let count = u32::try_from(self.vars.names.len()).unwrap_or(u32::MAX);
          let mut labels = own.labels;

          labels.push(EffectLabel::effect(effect));
          self.vars = saved;
          self.op_schemes.insert(
            (effect.clone(), sig.name.text.clone()),
            Scheme::new(
              count,
              Type::func_with(params, ret, Effects::new(labels, own.tail)),
            ),
          );
        }
      } else {
        for (op, scheme) in &info.schemes {
          self.op_schemes.insert((effect.clone(), op.clone()), scheme.clone());
        }
      }
    }

    for (name, decl) in &table.externs {
      let saved = std::mem::replace(
        &mut self.vars,
        TypeVars { names: Vec::new(), mode: VarMode::Method, rows: Vec::new() },
      );
      let params: Vec<Type> = decl
        .params
        .iter()
        .map(|p| match &p.ty {
          Some(ty) => self.convert(ty),
          None => self.fresh(),
        })
        .collect();
      let ret = self.convert(&decl.return_type);
      let row = self.convert_effects(decl.effects.as_ref());
      let count = u32::try_from(self.vars.names.len()).unwrap_or(u32::MAX);

      self.vars = saved;
      self.extern_schemes.insert(
        name.clone(),
        Scheme::new(count, Type::func_with(params, ret, row)),
      );
    }
  }

  pub(crate) fn extern_types(&self) -> BTreeMap<String, Type> {
    let own = self
      .extern_schemes
      .iter()
      .map(|(name, scheme)| (name.clone(), scheme.ty.clone()));
    let bound = self.effects.bound.iter().filter_map(|b| {
      let scheme = self.op_schemes.get(&(b.effect.clone(), b.op.clone()))?;

      Some((b.name.clone(), scheme.ty.clone()))
    });

    own.chain(bound).collect()
  }

  pub(crate) fn operation_type(&mut self, effect: &str, op: &str) -> Type {
    let scheme = self.op_schemes.get(&(effect.to_string(), op.to_string()));

    match scheme.cloned() {
      Some(scheme) => instantiate_open(&mut self.store, &scheme).0,
      None => self.fresh(),
    }
  }

  pub(crate) fn extern_type(&mut self, name: &str, key: &Span) -> Type {
    match self.extern_schemes.get(name).cloned() {
      Some(scheme) => {
        let ty = instantiate(&mut self.store, &scheme);

        self.opened(key, ty)
      }
      None => self.fresh(),
    }
  }

  pub(crate) fn bind_bodies(&mut self, decls: &[&'a Decl]) {
    for decl in decls {
      if let Decl::Bind(bind) = decl {
        self.bind_body(bind);
      }
    }
  }

  pub(crate) fn check_bridges(&mut self, decls: &[&'a Decl]) {
    for decl in decls {
      if let Decl::Bind(bind) = decl
        && bind.from.is_some()
        && self.effects.is_effect(&bind.effect.text)
        && self.effects.hosts.contains_key(&bind.host.text)
      {
        self.check_bridge(bind);
      }
    }
  }

  fn check_bridge(&mut self, bind: &'a BindDecl) {
    let effect = bind.effect.text.clone();
    let json: Arc<str> = Arc::from("Std.Json.Json");

    if !self.env.traits.contains_key(&json) {
      self.push(
        Diagnostic::error(
          BridgeNotWireSafe,
          format!("bridging `{effect}` sends values as JSON, but `Json` isn't in scope"),
          Label::new(bind.span.clone()),
        )
        .with_help("add `uses Std.Json`"),
      );
      return;
    }

    let Some(info) = self.effects.effects.get(&effect).cloned() else {
      return;
    };

    let mut ops = Vec::new();
    let mut safe = true;

    for (i, (op, _)) in info.ops.iter().enumerate() {
      let key = (effect.clone(), op.clone());
      let Some(scheme) = self.op_schemes.get(&key).cloned() else { continue };
      let fill: Vec<Type> = (0..scheme.count)
        .map(|i| Type::Rigid {
          name: Arc::from(format!("t{i}").as_str()),
          id: self.fresh_var(),
        })
        .collect();
      let Type::Fn { params, ret, effects } = substitute(&scheme.ty, &fill)
      else {
        continue;
      };
      let sig = info.sigs.iter().find(|s| s.name.text == *op);
      let names = info.params.get(i).cloned().unwrap_or_default();
      let mut bridged = BridgeOp {
        op: op.clone(),
        params: Vec::new(),
        ret: None,
        throws: Vec::new(),
      };

      for (j, ty) in params.iter().enumerate() {
        let what = names.get(j).map_or_else(
          || format!("parameter {}", j + 1),
          |n| format!("parameter `{n}`"),
        );
        let at = sig.and_then(|s| s.params.get(j)).and_then(|p| p.ty.as_ref());

        match self.wire_safe(bind, op, &what, ty, at.map(|t| t.span().clone()))
        {
          Ok(ev) => bridged.params.push(ev),
          Err(()) => safe = false,
        }
      }

      let at = sig.map(|s| s.return_type.span().clone());

      match self.wire_safe(bind, op, "result", &ret, at) {
        Ok(ev) => bridged.ret = ev,
        Err(()) => safe = false,
      }

      for label in effects.labels.iter().filter(|l| l.is_throws()) {
        let Some(arg) = &label.arg else { continue };
        let local = self
          .module_name
          .as_ref()
          .and_then(|m| arg.strip_prefix(&format!("{m}.")))
          .map(|bare| self.qualified(bare));
        let name = local.map_or_else(|| arg.clone(), |n| Arc::from(n.as_str()));
        let ty = Type::Con { name, args: Vec::new() };
        let at = sig.and_then(|s| s.effects.as_ref()).map(|e| e.span.clone());

        match self.wire_safe(bind, op, "error", &ty, at) {
          Ok(Some(ev)) => bridged.throws.push((error_tag(arg), ev)),
          Ok(None) => {}
          Err(()) => safe = false,
        }
      }

      ops.push(bridged);
    }

    if safe {
      self.bridges.entry(effect).or_insert(ops);
    }
  }

  fn wire_safe(
    &mut self,
    bind: &BindDecl,
    op: &str,
    what: &str,
    ty: &Type,
    at: Option<Span>,
  ) -> Result<Option<Evidence>, ()> {
    let unit = matches!(
      ty,
      Type::Record(row) if row.fields.is_empty() && row.tail.is_closed()
    );

    if unit {
      return Ok(None);
    }

    let pred = Pred { trait_path: Arc::from("Std.Json.Json"), ty: ty.clone() };

    if let Some(ev) = self.evidence_for(&pred) {
      return Ok(Some(ev));
    }

    let shown = Printer::new(&[ty], true).print(ty);
    let effect = &bind.effect.text;
    let generic = matches!(ty, Type::Rigid { .. });
    let help = if generic {
      "a bridged operation can't be generic: give it a concrete type"
        .to_string()
    } else {
      format!("add `derive(Json)` to `{shown}`, or give it a `Json` impl")
    };
    let mut diagnostic = Diagnostic::error(
      BridgeNotWireSafe,
      format!(
        "`{effect}.{op}` can't cross from `{}` to `{}`: its {what} is `{shown}`, which has no `Json`",
        bind.host.text,
        bind.from.as_ref().map_or("", |v| v.text.as_str())
      ),
      Label::new(bind.span.clone()),
    )
    .with_help(help);

    if let Some(at) = at {
      diagnostic = diagnostic.with_secondary(
        Label::new(at).with_message(format!("`{shown}` is declared here")),
      );
    }

    self.push(diagnostic);
    Err(())
  }

  fn bind_body(&mut self, bind: &'a BindDecl) {
    let effect = &bind.effect.text;

    if !self.effects.is_effect(effect)
      || !self.effects.hosts.contains_key(&bind.host.text)
    {
      return;
    }

    for op in &bind.ops {
      let key = (effect.clone(), op.name.text.clone());
      let Some(scheme) = self.op_schemes.get(&key).cloned() else { continue };

      self.bind_op(bind, op, &scheme);
    }
  }

  fn bind_op(&mut self, bind: &'a BindDecl, op: &'a FnDecl, scheme: &Scheme) {
    let start = self.wanted.len();

    self.store.enter_level();

    let fill: Vec<Type> = (0..scheme.count)
      .map(|i| Type::Rigid {
        name: Arc::from(format!("t{i}").as_str()),
        id: self.fresh_var(),
      })
      .collect();
    let expected = substitute(&scheme.ty, &fill);
    let Type::Fn { params, ret, effects: declared } = expected else {
      self.store.leave_level();
      return;
    };

    if params.len() != op.params.len() {
      self.store.leave_level();
      return;
    }

    self.vars = TypeVars::rigid();

    for (param, ty) in op.params.iter().zip(&params) {
      if let Some(annotation) = &param.ty {
        let written = self.convert(annotation);

        self.expect(ty, &written, annotation.span());
      }

      self.bind_mono(&param.name.span, ty.clone());
    }

    if let Some(annotation) = &op.return_type {
      let written = self.convert(annotation);

      self.expect(&ret, &written, annotation.span());
    }

    let row = Effects::open(self.fresh_var());
    let (body, row) = self.with_row(row, RowOwner::Free, |c| c.block(&op.body));
    let blame = last_expr(&op.body.result).span();

    self.expect(&ret, &body, blame);
    self.store.leave_level();
    self.solve_closed(start);
    self.vars = TypeVars::flexible();

    let row = self.store.zonk_effects(&row);
    let uses = std::mem::take(&mut self.last_uses);

    for label in row.labels.iter().filter(|l| l.is_throws()) {
      if declared.labels.contains(label) {
        continue;
      }

      let thrown = label.arg.as_deref().map_or("an error", bare);
      let mut diagnostic = Diagnostic::error(
        EffectNotAllowed,
        format!(
          "the binding of `{}` can throw `{thrown}`, but `{}.{}` doesn't declare it",
          op.name.text, bind.effect.text, op.name.text
        ),
        Label::new(op.name.span.clone()),
      )
      .with_help(format!(
        "catch it inside the binding, or add `Throws<{thrown}>` to `{}`'s row \
         in the effect",
        op.name.text
      ));

      if let Some(at) = uses.get(label) {
        diagnostic = diagnostic
          .with_secondary(Label::new(at.clone()).with_message("thrown here"));
      }

      self.push(diagnostic);
    }

    let uses: Vec<(EffectLabel, Span)> = uses.into_vec();

    self.bind_rows.insert(
      (bind.effect.text.clone(), bind.host.text.clone(), op.name.text.clone()),
      (row, uses),
    );
  }

  pub(crate) fn throw_expr(&mut self, t: &'a Throw) -> Type {
    let ty = self.infer(&t.value);

    if let Some(label) = self.throws_label(&ty) {
      if let Some(arg) = &label.arg {
        self.throws.insert((t.span.start, t.span.end), arg.to_string());
      }

      self.perform(&Effects::new(vec![label], EffTail::Closed), &t.span);
    } else {
      let zonked = self.store.zonk(&ty);

      if matches!(zonked, Type::Var(_)) {
        self.push(
          Diagnostic::error(
            BadThrowsType,
            "cannot tell which error type this throws",
            Label::new(t.value.span().clone()),
          )
          .with_help("annotate the value's type, like `(e: ParseError)`"),
        );
      } else {
        let printed = Printer::new(&[&zonked], true).print(&zonked);

        self.bad_throws(&printed, t.value.span());
      }
    }

    self.fresh()
  }

  fn arm_type(&mut self, pattern: &Pattern) -> ArmType {
    match pattern {
      Pattern::Ctor(ctor) => {
        let Some(Resolved::Ctor(id)) = self.resolved(&ctor.name.span) else {
          return ArmType::Unresolved;
        };
        let Some(scheme) = self.ctor_scheme(id) else {
          return ArmType::Unresolved;
        };

        ArmType::Typed(match self.instantiate(&scheme) {
          Type::Fn { ret, .. } => *ret,
          other => other,
        })
      }
      Pattern::Wildcard(_) | Pattern::Var(_) => ArmType::Loose,
      Pattern::Invalid(_) => ArmType::Unresolved,
      Pattern::Lit(_) | Pattern::Record(_) | Pattern::List(_) => ArmType::Other,
    }
  }

  fn catch_needs_type(&mut self, span: &Span) {
    self.push(
      Diagnostic::error(
        CatchNeedsType,
        "this arm could catch any error type; name the constructors",
        Label::new(span.clone()),
      )
      .with_help("a catch arm handles one error type, like `Missing(k) -> …`"),
    );
  }

  pub(crate) fn try_expr(&mut self, t: &'a Try) -> Type {
    let mut groups: Vec<(EffectLabel, Type, Vec<usize>)> = Vec::new();
    let mut loose: Vec<usize> = Vec::new();
    let mut bad: Vec<usize> = Vec::new();

    for (i, arm) in t.arms.iter().enumerate() {
      match self.arm_type(&arm.pattern) {
        ArmType::Loose => loose.push(i),
        ArmType::Unresolved => bad.push(i),
        ArmType::Other => {
          self.catch_needs_type(arm.pattern.span());
          bad.push(i);
        }
        ArmType::Typed(ty) => {
          if let Some(label) = self.throws_label(&ty) {
            if let Some((_, _, arms)) =
              groups.iter_mut().find(|(l, _, _)| *l == label)
            {
              arms.push(i);
            } else {
              groups.push((label, ty, vec![i]));
            }
          } else {
            let zonked = self.store.zonk(&ty);
            let printed = Printer::new(&[&zonked], true).print(&zonked);

            if matches!(&zonked, Type::Con { args, .. } if !args.is_empty()) {
              self.push(
                Diagnostic::error(
                  BadThrowsType,
                  "`Throws` needs a type without parameters",
                  Label::new(arm.pattern.span().clone()),
                )
                .with_help("wrap it: `IntError = IntError(List<Int>)`"),
              );
            } else {
              self.bad_throws(&printed, arm.pattern.span());
            }

            bad.push(i);
          }
        }
      }
    }

    for &i in &loose {
      if let [(_, _, arms)] = groups.as_mut_slice() {
        arms.push(i);
      } else {
        self.catch_needs_type(t.arms[i].pattern.span());
        bad.push(i);
      }
    }

    for (_, ty, arms) in &groups {
      for &i in arms {
        self.check_pattern(&t.arms[i].pattern, ty);
      }
    }

    for &i in &bad {
      let fresh = self.fresh();

      self.check_pattern(&t.arms[i].pattern, &fresh);
    }

    let eps = self.fresh_var();
    let handled: Vec<EffectLabel> =
      groups.iter().map(|(label, _, _)| label.clone()).collect();
    let row = Effects::new(handled.clone(), EffTail::Open(eps));
    let (body_ty, _) = self.with_ambient(row, |c| c.block(&t.body));
    let inner_uses = std::mem::take(&mut self.last_uses);

    for (label, span) in inner_uses.into_vec() {
      if !handled.contains(&label) {
        self.first_use.note(&label, &span);
      }
    }

    let outer = self.first_use.clone();

    self.perform(&Effects::open(eps), &t.span);
    self.first_use = outer;

    let first = last_expr(&t.body.result).span().clone();

    for arm in &t.arms {
      let body = self.infer(&arm.body);
      let blame = last_expr(&arm.body).span().clone();
      let because = Because::Branch(first.clone(), body_ty.clone());

      self.expect_because(&body_ty, &body, &blame, because);
    }

    let mut paths: Vec<String> = handled
      .iter()
      .filter_map(|l| l.arg.as_ref().map(ToString::to_string))
      .collect();

    paths.sort();
    self.handles.insert((t.span.start, t.span.end), paths);

    for (_, ty, arms) in groups {
      let arms = arms.iter().map(|&i| &t.arms[i]).collect();

      self.matches.push(Site::Catch { arms, ty, span: t.catch_span.clone() });
    }

    body_ty
  }

  pub(crate) fn only_rows_differ(
    &self,
    expected: &Type,
    found: &Type,
  ) -> Option<(Effects, Effects)> {
    let expected = self.store.zonk(expected);
    let found = self.store.zonk(found);
    let mut scratch = self.store.clone();

    if unify(&mut scratch, &strip(&expected), &strip(&found)).is_err() {
      return None;
    }

    first_row_difference(&expected, &found)
  }

  pub(crate) fn rows_note(
    rows: &(Effects, Effects),
    callee: Option<&str>,
  ) -> String {
    let (wants, has) = rows;
    let texts = |labels: &[EffectLabel]| -> Vec<String> {
      labels.iter().map(label_text).collect()
    };
    let wanted = if wants.labels.is_empty() {
      "a pure function".to_string()
    } else {
      format!("a function that uses {}", join_labels(&texts(&wants.labels)))
    };
    let extra: Vec<EffectLabel> = has
      .labels
      .iter()
      .filter(|l| !wants.labels.contains(l))
      .cloned()
      .collect();
    let this = if extra.is_empty() {
      "this one may use other effects".to_string()
    } else {
      format!("this one uses {}", join_labels(&texts(&extra)))
    };
    let who = callee.map_or_else(
      || "the expected type".to_string(),
      |name| format!("`{name}`"),
    );

    format!(
      "these functions differ only in their effects: {who} wants {wanted}, \
       and {this}"
    )
  }
}

fn strip(ty: &Type) -> Type {
  match ty {
    Type::Fn { params, ret, .. } => {
      Type::func(params.iter().map(strip).collect(), strip(ret))
    }
    Type::Con { name, args } => {
      Type::Con { name: name.clone(), args: args.iter().map(strip).collect() }
    }
    Type::Record(row) => {
      let mut row = row.clone();

      for (_, t) in &mut row.fields {
        *t = strip(t);
      }

      Type::Record(row)
    }
    other => other.clone(),
  }
}

fn first_row_difference(
  expected: &Type,
  found: &Type,
) -> Option<(Effects, Effects)> {
  match (expected, found) {
    (
      Type::Fn { params: p1, ret: r1, effects: e1 },
      Type::Fn { params: p2, ret: r2, effects: e2 },
    ) => p1
      .iter()
      .zip(p2)
      .find_map(|(a, b)| first_row_difference(a, b))
      .or_else(|| first_row_difference(r1, r2))
      .or_else(|| (e1 != e2).then(|| (e1.clone(), e2.clone()))),
    (Type::Con { args: a1, .. }, Type::Con { args: a2, .. }) => {
      a1.iter().zip(a2).find_map(|(a, b)| first_row_difference(a, b))
    }
    (Type::Record(a), Type::Record(b)) => a.fields.iter().find_map(|(n, t)| {
      b.fields
        .iter()
        .find(|(m, _)| m == n)
        .and_then(|(_, u)| first_row_difference(t, u))
    }),
    _ => None,
  }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct FirstUses(Vec<(EffectLabel, Span)>);

impl FirstUses {
  pub(crate) fn note(&mut self, label: &EffectLabel, span: &Span) {
    if !self.0.iter().any(|(l, _)| l == label) {
      self.0.push((label.clone(), span.clone()));
    }
  }

  pub(crate) fn get(&self, label: &EffectLabel) -> Option<&Span> {
    self.0.iter().find(|(l, _)| l == label).map(|(_, s)| s)
  }

  pub(crate) fn position(&self, label: &EffectLabel) -> Option<usize> {
    self.0.iter().position(|(l, _)| l == label)
  }

  pub(crate) fn into_vec(self) -> Vec<(EffectLabel, Span)> {
    self.0
  }
}

pub(crate) type BindRows = std::collections::BTreeMap<
  (String, String, String),
  (Effects, Vec<(EffectLabel, Span)>),
>;
