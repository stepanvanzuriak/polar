use std::collections::BTreeMap;

use crate::{
  core::{
    ir::{Boundary, CBind, CDecl, CExpr, CExprKind, CExtern, DeclKind, Sym},
    lower::{Lowerer, Resolved},
    scope::suggest,
  },
  shared::codes::DiagnosticCode::{
    BridgeChain, BridgeTargetUnbound, BuiltinModuleAsValue, CallArity,
    DuplicateBind, DuplicateDefinition, ExtraOperation, IncompleteTraitExport,
    MissingOperation, NativeBind, NativeWithoutHost, NeedlessForce,
    OperationParamCount, OrphanBind, UnknownEffect, UnknownHost,
    UnknownOperation,
  },
  shared::diagnostic::{Diagnostic, Label},
  shared::source::Span,
  stdlib::Interface,
  syntax::ast::{
    BindDecl, Decl, EffectDecl, ExportDecl, ExternDecl, FieldAccess, FnDecl,
    MethodSig, Name, StringLit, StringPart,
  },
  types::ty::Scheme,
};

#[derive(Debug, Clone)]
pub struct HostInfo {
  pub local: bool,
  pub exported: bool,
  pub span: Span,
}

#[derive(Debug, Clone)]
pub struct EffectInfo {
  pub native: bool,
  pub host: Option<String>,
  pub ops: Vec<(String, usize)>,
  pub params: Vec<Vec<String>>,
  pub local: bool,
  pub exported: bool,
  pub span: Span,
  pub sigs: Vec<MethodSig>,
  pub schemes: Vec<(String, Scheme)>,
  pub module: Option<String>,
}

impl EffectInfo {
  #[must_use]
  pub fn arity(&self, op: &str) -> Option<usize> {
    self.ops.iter().find(|(name, _)| name == op).map(|&(_, arity)| arity)
  }
}

#[derive(Debug, Clone)]
pub struct BindInfo {
  pub effect: String,
  pub host: String,
  pub via: Option<String>,
  pub module: Option<String>,
  pub span: Option<Span>,
}

#[derive(Debug, Clone)]
pub struct BoundExtern {
  pub name: String,
  pub module: String,
  pub effect: String,
  pub op: String,
  pub span: Span,
}

#[derive(Debug, Clone, Default)]
pub struct EffectTable {
  pub hosts: BTreeMap<String, HostInfo>,
  pub effects: BTreeMap<String, EffectInfo>,
  pub binds: Vec<BindInfo>,
  pub externs: BTreeMap<String, ExternDecl>,
  pub bound: Vec<BoundExtern>,
}

impl EffectTable {
  #[must_use]
  pub fn host_names(&self) -> Vec<String> {
    self.hosts.keys().cloned().collect()
  }

  #[must_use]
  pub fn is_effect(&self, name: &str) -> bool {
    self.effects.contains_key(name)
  }
}

impl Lowerer<'_> {
  pub(super) fn import_effects(
    &mut self,
    path: &str,
    interface: &Interface,
    at: &Span,
  ) {
    for host in &interface.hosts {
      self.effects.hosts.entry(host.clone()).or_insert_with(|| HostInfo {
        local: false,
        exported: false,
        span: at.clone(),
      });
    }

    for sig in &interface.effects {
      if self.effects.effects.contains_key(&sig.name) {
        let name = Name { text: sig.name.clone(), span: at.clone() };

        self.duplicate(&name, "effect", at.clone());
        continue;
      }

      self.effects.effects.insert(
        sig.name.clone(),
        EffectInfo {
          native: sig.native,
          host: sig.host.clone(),
          ops: sig.ops.clone(),
          params: sig
            .ops
            .iter()
            .map(|(_, n)| (1..=*n).map(|i| format!("x{i}")).collect())
            .collect(),
          local: false,
          exported: false,
          span: at.clone(),
          sigs: Vec::new(),
          schemes: sig.schemes.clone(),
          module: Some(path.to_string()),
        },
      );
    }

    for (effect, host) in &interface.binds {
      self.effects.binds.push(BindInfo {
        effect: effect.clone(),
        host: host.clone(),
        via: None,
        module: Some(path.to_string()),
        span: None,
      });
    }
  }

  pub(super) fn declare_effects(&mut self, decls: &[&Decl]) {
    for decl in decls {
      if let Decl::Host(h) = decl {
        self.declare_host(&h.name);
      }
    }

    for decl in decls {
      match decl {
        Decl::Effect(e) => self.declare_effect(e),
        Decl::Extern(e) => self.declare_extern(e),
        _ => {}
      }
    }
  }

  fn declare_host(&mut self, name: &Name) {
    if let Some(first) = self.effects.hosts.get(&name.text)
      && first.local
    {
      let first = first.span.clone();

      self.duplicate(name, "host", first);
      return;
    }

    self.effects.hosts.insert(
      name.text.clone(),
      HostInfo { local: true, exported: false, span: name.span.clone() },
    );
  }

  fn declare_effect(&mut self, e: &EffectDecl) {
    let name = &e.name;

    self.not_a_builtin_module(name, "effect");

    if let Some(first) = self.effects.effects.get(&name.text) {
      let first = first.span.clone();

      self.duplicate(name, "effect", first);
      return;
    }

    if self.imports.contains_key(&name.text) {
      self.error(
        DuplicateDefinition,
        format!(
          "the effect `{}` has the name of an imported module",
          name.text
        ),
        name.span.clone(),
        Some("choose another name".to_string()),
      );
    }

    if let Some((_, first)) = self.ctor_ids.get(&name.text) {
      let first = first.clone();

      self.duplicate(name, "effect", first);
    }

    match &e.host {
      Some(host) if !self.effects.hosts.contains_key(&host.text) => {
        self.unknown_host(host);
      }
      None if e.native => self.error(
        NativeWithoutHost,
        format!("the `native` effect `{}` needs a host", name.text),
        name.span.clone(),
        Some(format!(
          "write `native {} in <Host>`, or drop `native` to let any host bind it",
          name.text
        )),
      ),
      _ => {}
    }

    let mut ops: Vec<(String, usize)> = Vec::new();
    let mut params = Vec::new();

    for op in &e.ops {
      if let Some(first) = e
        .ops
        .iter()
        .take_while(|o| !std::ptr::eq(*o, op))
        .find(|o| o.name.text == op.name.text)
      {
        self.duplicate(&op.name, "operation", first.name.span.clone());
        continue;
      }

      ops.push((op.name.text.clone(), op.params.len()));
      params.push(op.params.iter().map(|p| p.name.text.clone()).collect());
    }

    self.effects.effects.insert(
      name.text.clone(),
      EffectInfo {
        native: e.native,
        host: e.host.as_ref().map(|h| h.text.clone()),
        ops,
        params,
        local: true,
        exported: false,
        span: name.span.clone(),
        sigs: e.ops.clone(),
        schemes: Vec::new(),
        module: None,
      },
    );
  }

  fn declare_extern(&mut self, e: &ExternDecl) {
    let name = &e.name;

    if let Some(first) = self.effects.externs.get(&name.text) {
      let first = first.name.span.clone();

      self.duplicate(name, "extern", first);
      return;
    }

    if let Some(first) = self.top_span(&name.text) {
      self.duplicate(name, "extern", first);
      return;
    }

    self.effects.externs.insert(name.text.clone(), e.clone());
  }

  fn unknown_host(&mut self, name: &Name) {
    let hosts: Vec<&str> =
      self.effects.hosts.keys().map(String::as_str).collect();
    let help = suggest(&name.text, hosts.iter().copied())
      .map(|s| format!("did you mean `{s}`?"))
      .or_else(|| {
        Some(format!("declare it in a `hosts` zone: `hosts\n  {}`", name.text))
      });

    self.error(
      UnknownHost,
      format!("there is no host `{}`", name.text),
      name.span.clone(),
      help,
    );
  }

  pub(super) fn effect_target(&self, field: &FieldAccess) -> Option<String> {
    match &*field.target {
      crate::syntax::ast::Expr::Var(var)
        if self.effects.effects.contains_key(&var.name.text)
          && !self.module_member(&var.name.text, &field.field.text) =>
      {
        Some(var.name.text.clone())
      }
      _ => None,
    }
  }

  fn module_member(&self, effect: &str, member: &str) -> bool {
    self.effects.effects[effect].arity(member).is_none()
      && self.imports.get(effect).is_some_and(|(_, interface)| {
        interface.function(member).is_some() || interface.constant(member)
      })
  }

  pub(super) fn operation(
    &mut self,
    effect: &str,
    field: &FieldAccess,
  ) -> CExpr {
    let origin = Some(field.span.clone());
    let op = &field.field.text;
    let Some(info) = self.effects.effects.get(effect) else {
      return CExpr::new(
        CExprKind::Lit(crate::core::ir::Lit::Bool(false)),
        origin,
      );
    };

    if info.arity(op).is_none() {
      let names: Vec<&str> = info.ops.iter().map(|(n, _)| n.as_str()).collect();
      let help = suggest(op, names.iter().copied())
        .map(|s| format!("did you mean `{s}`?"))
        .or_else(|| {
          (!names.is_empty()).then(|| {
            let listed: Vec<String> =
              names.iter().map(|n| format!("`{n}`")).collect();

            format!("its operations are {}", listed.join(", "))
          })
        });

      self.diagnostics.push({
        let mut d = Diagnostic::error(
          UnknownOperation,
          format!("`{effect}` has no operation `{op}`"),
          Label::new(field.field.span.clone())
            .with_message(format!("not an operation of `{effect}`")),
        );

        if let Some(help) = help {
          d = d.with_help(help);
        }

        d
      });

      return CExpr::new(
        CExprKind::Lit(crate::core::ir::Lit::Bool(false)),
        origin,
      );
    }

    self.resolutions.insert(
      &field.field.span,
      Resolved::Operation { effect: effect.to_string(), op: op.clone() },
    );

    CExpr::new(
      CExprKind::Op { effect: effect.to_string(), op: op.clone() },
      origin,
    )
  }

  pub(super) fn operation_arity(
    &mut self,
    field: &FieldAccess,
    args: usize,
    origin: &Span,
  ) {
    let Some(effect) = self.effect_target(field) else { return };
    let Some(arity) = self.effects.effects[&effect].arity(&field.field.text)
    else {
      return;
    };

    if args != arity {
      let name = format!("{effect}.{}", field.field.text);

      self.error(
        CallArity,
        crate::core::lower::arity_message(&name, arity, args),
        origin.clone(),
        None,
      );
    }
  }

  pub(super) fn effect_as_value(&mut self, name: &Name, span: &Span) -> bool {
    let Some(info) = self.effects.effects.get(&name.text) else {
      return false;
    };
    let example = info
      .ops
      .first()
      .map_or_else(|| "<operation>".to_string(), |(op, _)| op.clone());

    self.error(
      BuiltinModuleAsValue,
      format!("`{}` is an effect, not a value", name.text),
      span.clone(),
      Some(format!(
        "call one of its operations, like `{}.{example}(…)`",
        name.text
      )),
    );

    true
  }

  pub(super) fn extern_ref(
    &mut self,
    name: &Name,
    origin: Option<Span>,
  ) -> Option<CExpr> {
    if !self.effects.externs.contains_key(&name.text) {
      return None;
    }

    self.resolutions.insert(&name.span, Resolved::Extern(name.text.clone()));

    Some(CExpr::new(CExprKind::Extern { name: name.text.clone() }, origin))
  }

  pub(super) fn extern_arity(&mut self, name: &str) -> Option<(usize, Span)> {
    self
      .effects
      .externs
      .get(name)
      .map(|e| (e.params.len(), e.name.span.clone()))
  }

  pub(super) fn within_bind<T>(&mut self, f: impl FnOnce(&mut Self) -> T) -> T {
    let saved = std::mem::replace(&mut self.in_bind, true);
    let out = f(self);

    self.in_bind = saved;
    out
  }

  pub(super) fn binds(&mut self, decls: &[&Decl]) -> Vec<CBind> {
    let mut out = Vec::new();

    for decl in decls {
      if let Decl::Bind(b) = decl
        && let Some(bind) = self.lower_bind(b)
      {
        out.push(bind);
      }
    }

    for decl in decls {
      if let Decl::Bind(b) = decl
        && let Some(target) = &b.from
        && out.iter().any(|c| {
          c.effect == b.effect.text && c.host == b.host.text && c.via.is_some()
        })
      {
        self.bridge_target(b, target);
      }
    }

    out
  }

  fn bridge_target(&mut self, b: &BindDecl, target: &Name) {
    let effect = &b.effect.text;
    let at = self
      .effects
      .binds
      .iter()
      .find(|x| x.effect == *effect && x.host == target.text)
      .cloned();

    match at {
      Some(BindInfo { via: Some(next), span, .. }) => {
        let mut diagnostic = Diagnostic::error(
          BridgeChain,
          format!(
            "`{effect}` in `{}` is itself a bridge to `{next}`, so `{}` can't \
             forward to it",
            target.text, b.host.text
          ),
          Label::new(target.span.clone()),
        )
        .with_help(format!("write `{effect} in {} from {next}`", b.host.text));

        if let Some(span) = span {
          diagnostic = diagnostic
            .with_secondary(Label::new(span).with_message("the other bridge"));
        }

        self.diagnostics.push(diagnostic);
      }
      Some(_) => {}
      None => {
        self.diagnostics.push(
          Diagnostic::error(
            BridgeTargetUnbound,
            format!(
              "`{effect}` has no binding in `{}` for this bridge to forward to",
              target.text
            ),
            Label::new(target.span.clone())
              .with_message(format!("`{effect}` doesn't run here")),
          )
          .with_help(format!(
            "add `{effect} in {} {{ … }}` or `{effect} in {} = \"./file.js\"` \
             to a `binds` zone",
            target.text, target.text
          )),
        );
      }
    }
  }

  fn lower_bind(&mut self, b: &BindDecl) -> Option<CBind> {
    let header = b.effect.span.join(&b.host.span);
    let info = self.bind_target(b, &header)?;

    self.effects.binds.push(BindInfo {
      effect: b.effect.text.clone(),
      host: b.host.text.clone(),
      via: b.from.as_ref().map(|v| v.text.clone()),
      module: None,
      span: Some(header.clone()),
    });
    self.bind_modifiers(b, &info, &header);

    if let Some(target) = &b.from {
      if !self.effects.hosts.contains_key(&target.text) {
        self.unknown_host(target);
        return None;
      }

      return Some(CBind {
        effect: b.effect.text.clone(),
        host: b.host.text.clone(),
        via: Some(target.text.clone()),
        ops: Vec::new(),
        origin: Some(header),
      });
    }

    if let Some(module) = &b.module {
      let module = string_text(module);
      let ops = info
        .ops
        .iter()
        .zip(&info.params)
        .map(|((op, _), params)| self.bound_op(b, &module, op, params, &header))
        .collect();

      return Some(CBind {
        effect: b.effect.text.clone(),
        host: b.host.text.clone(),
        via: None,
        ops,
        origin: Some(header),
      });
    }

    self.bind_ops(b, &info, &header);

    let ops = b
      .ops
      .iter()
      .filter(|op| info.arity(&op.name.text).is_some())
      .map(|op| self.within_bind(|l| l.fn_named(op, &op.name.text)))
      .collect();

    Some(CBind {
      effect: b.effect.text.clone(),
      host: b.host.text.clone(),
      via: None,
      ops,
      origin: Some(header),
    })
  }

  fn bound_op(
    &mut self,
    b: &BindDecl,
    module: &str,
    op: &str,
    params: &[String],
    header: &Span,
  ) -> CDecl {
    let name = format!("{}${}${op}", b.effect.text, b.host.text);
    let params: Vec<Sym> = params.iter().map(|p| self.syms.fresh(p)).collect();
    let args = params
      .iter()
      .map(|p| CExpr::new(CExprKind::Var(p.clone()), None))
      .collect();
    let func = CExpr::new(CExprKind::Extern { name: name.clone() }, None);

    self.effects.bound.push(BoundExtern {
      name,
      module: module.to_string(),
      effect: b.effect.text.clone(),
      op: op.to_string(),
      span: header.clone(),
    });

    CDecl {
      sym: self.syms.fresh(op),
      kind: DeclKind::Function,
      exported: false,
      params,
      body: CExpr::new(
        CExprKind::App { func: Box::new(func), args },
        Some(header.clone()),
      ),
      origin: Some(header.clone()),
      effects: None,
    }
  }

  fn bind_target(&mut self, b: &BindDecl, header: &Span) -> Option<EffectInfo> {
    let effect = self.effects.effects.get(&b.effect.text).cloned();
    let host_known = self.effects.hosts.contains_key(&b.host.text);

    let Some(info) = effect else {
      let names: Vec<&str> =
        self.effects.effects.keys().map(String::as_str).collect();
      let help = suggest(&b.effect.text, names.iter().copied()).map_or_else(
        || "effects are declared in an `effects` zone".to_string(),
        |s| format!("did you mean `{s}`?"),
      );

      self.error(
        UnknownEffect,
        format!("there is no effect `{}`", b.effect.text),
        b.effect.span.clone(),
        Some(help),
      );

      if !host_known {
        self.unknown_host(&b.host);
      }

      return None;
    };

    if !host_known {
      self.unknown_host(&b.host);
      return None;
    }

    let host_local =
      self.effects.hosts.get(&b.host.text).is_some_and(|h| h.local);

    if !info.local && !host_local {
      self.diagnostics.push(
        Diagnostic::error(
          OrphanBind,
          format!(
            "this binding must live next to `{}` or `{}`",
            b.effect.text, b.host.text
          ),
          Label::new(header.clone()),
        )
        .with_help(format!(
          "move it into the module that declares `{}` or the one that \
           declares `{}`",
          b.effect.text, b.host.text
        )),
      );

      return None;
    }

    let first = self
      .effects
      .binds
      .iter()
      .find(|x| x.effect == b.effect.text && x.host == b.host.text)
      .cloned();

    if let Some(first) = first {
      let mut diagnostic = Diagnostic::error(
        DuplicateBind,
        format!("`{}` is already bound in `{}`", b.effect.text, b.host.text),
        Label::new(header.clone()),
      );

      diagnostic = match first.span {
        Some(span) => diagnostic
          .with_secondary(Label::new(span).with_message("the first one")),
        None => diagnostic.with_note(format!(
          "`{}` binds it already",
          first.module.unwrap_or_default()
        )),
      };

      self.diagnostics.push(diagnostic);
      return None;
    }

    Some(info)
  }

  fn bind_modifiers(&mut self, b: &BindDecl, info: &EffectInfo, header: &Span) {
    let native_host = info.host.as_deref() == Some(b.host.text.as_str());
    let bridge = b.from.is_some();

    if info.native && !native_host && !b.force && !bridge {
      self.diagnostics.push(
        Diagnostic::error(
          NativeBind,
          format!(
            "`{}` is native to `{}`; binding it in `{}` needs `force`",
            b.effect.text,
            info.host.as_deref().unwrap_or_default(),
            b.host.text
          ),
          Label::new(header.clone()),
        )
        .with_help(format!(
          "write `force {} in {}` if an emulation is acceptable",
          b.effect.text, b.host.text
        )),
      );
    }

    if b.force && (!info.native || native_host || bridge) {
      let why = if bridge {
        "a bridge runs the real binding, so it emulates nothing".to_string()
      } else if native_host {
        format!("`{}` is the native host of `{}`", b.host.text, b.effect.text)
      } else {
        format!("`{}` is not a `native` effect", b.effect.text)
      };

      self.diagnostics.push(
        Diagnostic::warning(
          NeedlessForce,
          "`force` is not needed here",
          Label::new(b.span.clone()).with_message(why),
        )
        .with_help("remove it"),
      );
    }
  }

  fn bind_ops(&mut self, b: &BindDecl, info: &EffectInfo, header: &Span) {
    let names: Vec<&str> = info.ops.iter().map(|(n, _)| n.as_str()).collect();
    let mut seen: Vec<&FnDecl> = Vec::new();

    for op in &b.ops {
      if let Some(first) = seen.iter().find(|o| o.name.text == op.name.text) {
        let first = first.name.span.clone();

        self.duplicate(&op.name, "operation", first);
        continue;
      }

      seen.push(op);

      match info.arity(&op.name.text) {
        None => {
          let help = suggest(&op.name.text, names.iter().copied()).map_or_else(
            || "remove it".to_string(),
            |s| format!("did you mean `{s}`?"),
          );

          self.error(
            ExtraOperation,
            format!(
              "`{}` is not an operation of `{}`",
              op.name.text, b.effect.text
            ),
            op.name.span.clone(),
            Some(help),
          );
        }
        Some(arity) if arity != op.params.len() => {
          let params = if arity == 1 { "parameter" } else { "parameters" };

          self.error(
            OperationParamCount,
            format!(
              "`{}.{}` takes {arity} {params}, but this takes {}",
              b.effect.text,
              op.name.text,
              op.params.len()
            ),
            op.name.span.clone(),
            None,
          );
        }
        Some(_) => {}
      }
    }

    for (i, (op, _)) in info.ops.iter().enumerate() {
      if b.ops.iter().any(|o| o.name.text == *op) {
        continue;
      }

      let params = info.params.get(i).map(|p| p.join(", ")).unwrap_or_default();

      self.diagnostics.push(
        Diagnostic::error(
          MissingOperation,
          format!(
            "the binding of `{}` in `{}` is missing `{op}`",
            b.effect.text, b.host.text
          ),
          Label::new(header.clone())
            .with_message(format!("`{op}` is not provided")),
        )
        .with_help(format!(
          "a binding provides every operation of its effect: add \
           `{op}({params}) {{ … }}`"
        )),
      );
    }
  }

  pub(super) fn externs(&self) -> Vec<CExtern> {
    self
      .effects
      .externs
      .values()
      .map(|e| CExtern {
        name: e.name.text.clone(),
        module: string_text(&e.module),
        export: e.export.text.clone(),
        params: Vec::new(),
        ret: Boundary::Plain,
        origin: Some(e.name.span.clone()),
      })
      .chain(self.effects.bound.iter().map(|b| CExtern {
        name: b.name.clone(),
        module: b.module.clone(),
        export: b.op.clone(),
        params: Vec::new(),
        ret: Boundary::Plain,
        origin: Some(b.span.clone()),
      }))
      .collect()
  }

  pub(super) fn export_effect_or_host(&mut self, export: &ExportDecl) -> bool {
    let name = &export.name.text;

    if export.methods.is_some() {
      return false;
    }

    if let Some(info) = self.effects.effects.get_mut(name)
      && info.local
    {
      info.exported = true;
      return true;
    }

    if let Some(info) = self.effects.hosts.get_mut(name)
      && info.local
    {
      info.exported = true;
      return true;
    }

    false
  }

  pub(super) fn check_effect_exports(&mut self, exports: &[&ExportDecl]) {
    let incomplete: Vec<(String, String, Span)> = exports
      .iter()
      .filter_map(|export| {
        let info = self.effects.effects.get(&export.name.text)?;
        let name = info.host.as_ref()?;
        let host = self.effects.hosts.get(name)?;

        (info.local && info.exported && host.local && !host.exported).then(
          || (export.name.text.clone(), name.clone(), export.span.clone()),
        )
      })
      .collect();

    for (effect, host, span) in incomplete {
      self.error(
        IncompleteTraitExport,
        format!(
          "exporting the effect `{effect}` needs its host `{host}` exported too"
        ),
        span,
        Some(format!("add `{host}` to `exports`")),
      );
    }
  }
}

fn string_text(lit: &StringLit) -> String {
  lit
    .parts
    .iter()
    .map(|part| match part {
      StringPart::Text(t) => t.value.as_str(),
      StringPart::Interp(_) => "",
    })
    .collect()
}
