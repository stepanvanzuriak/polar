pub mod convert;
pub mod decl;
pub mod dump;
pub mod effects;
pub mod env;
pub mod exhaustive;
pub mod expr;
pub mod hosts;
pub mod pattern;
pub mod report;
pub mod solve;

use std::{
  collections::{BTreeMap, HashMap, HashSet},
  sync::Arc,
};

use crate::{
  core::{
    effects::EffectTable,
    ir::{CModule, CtorId, Sym},
    lower::{Lowered, Resolutions, Resolved, STD_OPTION, STD_RESULT},
  },
  shared::diagnostic::{Diagnostic, DiagnosticBag},
  shared::source::Span,
  stdlib::Interface,
  syntax::ast::{Expr, FnDecl, Module},
  types::{
    generalise::{default_brands, default_numbers, instantiate, open_row},
    store::Store,
    ty::{
      Constraint, EffTail, Effects, Label as EffectLabel, Pred, Scheme, TVar,
      Type,
    },
    unify::unify,
  },
};
use convert::TypeVars;
use env::{ImplDef, TraitDef, TypeDef, TypeEnv};
use report::Because;
use solve::{Evidence, GroupCall, Wanted};

#[derive(Debug, Clone, PartialEq)]
pub struct BridgeOp {
  pub op: String,
  pub params: Vec<Option<Evidence>>,
  pub ret: Option<Evidence>,
  pub throws: Vec<(String, Evidence)>,
}

#[derive(Debug, Default)]
pub struct Types {
  pub decls: Vec<(String, Scheme)>,
  pub exprs: HashMap<(usize, usize), Type>,
  pub type_defs: Vec<(String, TypeDef)>,
  pub failed: HashSet<String>,
  pub evidence: HashMap<(usize, usize), Vec<Evidence>>,
  pub shows: HashMap<(usize, usize), Evidence>,
  pub decl_preds: HashMap<String, Vec<Pred>>,
  pub traits: Vec<TraitDef>,
  pub impls: Vec<ImplDef>,
  pub impl_heads: HashMap<(usize, usize), (Arc<str>, Arc<str>)>,
  pub op_schemes: BTreeMap<(String, String), Scheme>,
  pub extern_types: BTreeMap<String, Type>,
  pub bind_rows: BTreeMap<(String, String, String), Effects>,
  pub bind_uses: BTreeMap<(String, String, String), Vec<(EffectLabel, Span)>>,
  pub handles: HashMap<(usize, usize), Vec<String>>,
  pub throws: HashMap<(usize, usize), String>,
  pub hosts: Vec<(String, hosts::HostSet)>,
  pub first_uses: HashMap<String, Vec<(EffectLabel, Span)>>,
  pub signatures: HashMap<(usize, usize), Type>,
  pub closed_rows: HashMap<(usize, usize), Effects>,
  pub bridges: BTreeMap<String, Vec<BridgeOp>>,
}

impl Types {
  #[must_use]
  pub fn evidence_at(&self, span: &Span) -> Option<&[Evidence]> {
    self
      .evidence
      .get(&(span.start, span.end))
      .map(Vec::as_slice)
      .filter(|e| !e.is_empty())
  }

  #[must_use]
  pub fn show_at(&self, span: &Span) -> Option<&Evidence> {
    self.shows.get(&(span.start, span.end))
  }
}

impl Types {
  #[must_use]
  pub fn scheme(&self, name: &str) -> Option<&Scheme> {
    self.decls.iter().find(|(n, _)| n == name).map(|(_, s)| s)
  }

  #[must_use]
  pub fn expr(&self, span: &Span) -> Option<&Type> {
    self.exprs.get(&(span.start, span.end))
  }
}

pub fn check(
  module: &Module,
  lowered: &Lowered,
  bag: &mut DiagnosticBag,
) -> Types {
  check_as(module, lowered, None, bag)
}

pub fn check_as(
  module: &Module,
  lowered: &Lowered,
  qualifier: Option<&str>,
  bag: &mut DiagnosticBag,
) -> Types {
  let mut checker = Checker {
    store: Store::default(),
    bag,
    resolutions: &lowered.resolutions,
    derived: &lowered.derived,
    core: &lowered.core,
    interfaces: &lowered.interfaces,
    effects: &lowered.effects,
    env: TypeEnv::with_primitives(),
    locals: HashMap::new(),
    exprs: HashMap::new(),
    vars: TypeVars::flexible(),
    qualifier: qualifier.map(str::to_string),
    top_fns: HashMap::new(),
    int_literals: HashMap::new(),
    matches: Vec::new(),
    local_types: Vec::new(),
    field_spans: HashMap::new(),
    shallow: false,
    declaring: None,
    module_name: module.name.as_ref().map(|n| n.text.clone()),
    wanted: Vec::new(),
    uses: HashMap::new(),
    interp_uses: HashMap::new(),
    solved: HashMap::new(),
    reported: HashSet::new(),
    givens: Vec::new(),
    owner: None,
    group_members: HashMap::new(),
    group_calls: Vec::new(),
    group_evidence: HashMap::new(),
    signature_span: None,
    local_traits: HashSet::new(),
    method_spans: HashMap::new(),
    impl_heads: HashMap::new(),
    decl_preds: HashMap::new(),
    ambient: Effects::pure(),
    row_owner: effects::RowOwner::Free,
    first_use: effects::FirstUses::default(),
    last_uses: effects::FirstUses::default(),
    op_schemes: BTreeMap::new(),
    extern_schemes: BTreeMap::new(),
    bind_rows: BTreeMap::new(),
    handles: HashMap::new(),
    throws: HashMap::new(),
    callee: None,
    decl_uses: HashMap::new(),
    reported_hosts: Vec::new(),
    signatures: HashMap::new(),
    closed_rows: HashMap::new(),
    bridges: BTreeMap::new(),
  };

  checker.module(module)
}

pub(crate) struct Checker<'a> {
  pub(crate) store: Store,
  pub(crate) bag: &'a mut DiagnosticBag,
  pub(crate) resolutions: &'a Resolutions,
  pub(crate) derived: &'a HashMap<(usize, usize), (String, String)>,
  pub(crate) core: &'a CModule,
  pub(crate) interfaces: &'a BTreeMap<String, Interface>,
  pub(crate) effects: &'a EffectTable,
  pub(crate) env: TypeEnv<'a>,
  pub(crate) locals: HashMap<u32, Scheme>,
  pub(crate) exprs: HashMap<(usize, usize), Type>,
  pub(crate) vars: TypeVars,
  pub(crate) qualifier: Option<String>,
  pub(crate) top_fns: HashMap<u32, &'a FnDecl>,
  pub(crate) int_literals: HashMap<(usize, usize), String>,
  pub(crate) matches: Vec<exhaustive::Site<'a>>,
  pub(crate) local_types: Vec<(String, TypeDef)>,
  pub(crate) field_spans: HashMap<(String, String), Span>,
  pub(crate) shallow: bool,
  pub(crate) declaring: Option<String>,
  pub(crate) module_name: Option<String>,
  pub(crate) wanted: Vec<Wanted>,
  pub(crate) uses: HashMap<(usize, usize), Vec<usize>>,
  pub(crate) interp_uses: HashMap<(usize, usize), Vec<usize>>,
  pub(crate) solved: HashMap<usize, Evidence>,
  pub(crate) reported: HashSet<(usize, usize)>,
  pub(crate) givens: Vec<Pred>,
  pub(crate) owner: Option<usize>,
  pub(crate) group_members: HashMap<u32, usize>,
  pub(crate) group_calls: Vec<GroupCall>,
  pub(crate) group_evidence: HashMap<(usize, usize), Vec<Evidence>>,
  pub(crate) signature_span: Option<Span>,
  pub(crate) local_traits: HashSet<String>,
  pub(crate) method_spans: HashMap<(Arc<str>, String), Span>,
  pub(crate) impl_heads: HashMap<(usize, usize), (Arc<str>, Arc<str>)>,
  pub(crate) decl_preds: HashMap<String, Vec<Pred>>,
  pub(crate) ambient: Effects,
  pub(crate) row_owner: effects::RowOwner,
  pub(crate) first_use: effects::FirstUses,
  pub(crate) last_uses: effects::FirstUses,
  pub(crate) op_schemes: BTreeMap<(String, String), Scheme>,
  pub(crate) extern_schemes: BTreeMap<String, Scheme>,
  pub(crate) bind_rows: effects::BindRows,
  pub(crate) handles: HashMap<(usize, usize), Vec<String>>,
  pub(crate) throws: HashMap<(usize, usize), String>,
  pub(crate) signatures: HashMap<(usize, usize), Type>,
  pub(crate) closed_rows: HashMap<(usize, usize), Effects>,
  pub(crate) callee: Option<String>,
  pub(crate) decl_uses: HashMap<String, effects::FirstUses>,
  pub(crate) reported_hosts: Vec<(String, String)>,
  pub(crate) bridges: BTreeMap<String, Vec<BridgeOp>>,
}

impl Checker<'_> {
  pub(crate) fn fresh(&mut self) -> Type {
    Type::Var(self.store.fresh())
  }

  pub(crate) fn fresh_var(&mut self) -> TVar {
    self.store.fresh()
  }

  pub(crate) fn fresh_constrained(&mut self, constraint: Constraint) -> Type {
    Type::Var(self.store.fresh_constrained(constraint))
  }

  pub(crate) fn instantiate(&mut self, scheme: &Scheme) -> Type {
    instantiate(&mut self.store, scheme)
  }

  pub(crate) fn expect(
    &mut self,
    expected: &Type,
    found: &Type,
    span: &Span,
  ) -> bool {
    self.expect_because(expected, found, span, Because::Nothing)
  }

  pub(crate) fn expect_because(
    &mut self,
    expected: &Type,
    found: &Type,
    span: &Span,
    because: Because,
  ) -> bool {
    let before = (self.store.zonk(expected), self.store.zonk(found));

    match unify(&mut self.store, expected, found) {
      Ok(()) => true,
      Err(error) => {
        let literal = self.int_literals.get(&(span.start, span.end)).cloned();
        let rows = self.only_rows_differ(&before.0, &before.1);
        let mut diagnostic = report::Report {
          error: &error,
          expected: &before.0,
          found: &before.1,
          span: span.clone(),
          because,
          int_literal: literal,
        }
        .build(&self.store);

        if let Some(rows) = rows {
          let note = Self::rows_note(&rows, self.callee.as_deref());

          diagnostic = diagnostic.with_note(note);
        }

        if let Some(label) = self.declared_field(&error, &before.0) {
          diagnostic = diagnostic.with_secondary(label);
        }

        self.bag.push(diagnostic);
        false
      }
    }
  }

  fn declared_field(
    &self,
    error: &crate::types::unify::UnifyError,
    expected: &Type,
  ) -> Option<crate::shared::diagnostic::Label> {
    let crate::types::unify::UnifyError::MissingField { label, .. } = error
    else {
      return None;
    };
    let Type::Record(row) = expected else { return None };
    let brand = row.brand()?;
    let span = self.field_spans.get(&(brand.to_string(), label.to_string()))?;

    Some(
      crate::shared::diagnostic::Label::new(span.clone())
        .with_message(format!("`{label}` is declared here")),
    )
  }

  pub(crate) fn push(&mut self, diagnostic: Diagnostic) {
    self.bag.push(diagnostic);
  }

  pub(crate) fn resolved(&self, span: &Span) -> Option<Resolved> {
    self.resolutions.get(span).cloned()
  }

  pub(crate) fn local_sym(&self, span: &Span) -> Option<Sym> {
    match self.resolutions.get(span) {
      Some(Resolved::Local(sym) | Resolved::Top(sym)) => Some(sym.clone()),
      _ => None,
    }
  }

  pub(crate) fn bind_mono(&mut self, span: &Span, ty: Type) {
    if let Some(sym) = self.local_sym(span) {
      self.locals.insert(sym.id, Scheme::new(0, ty));
    }
  }

  pub(crate) fn ctor_scheme(&self, id: CtorId) -> Option<Scheme> {
    let info = self.core.ctors.get(id.0 as usize)?;

    if info.owner == STD_RESULT || info.owner == STD_OPTION {
      return self
        .env
        .ctors_of(&info.owner)?
        .iter()
        .find(|(name, _)| *name == info.name)
        .map(|(_, scheme)| scheme.clone());
    }

    self.env.ctor(&info.name).cloned()
  }

  pub(crate) fn trait_path(&self, lowered: &str) -> Arc<str> {
    if self.local_traits.contains(lowered) {
      Arc::from(self.qualified(lowered))
    } else {
      Arc::from(lowered)
    }
  }

  pub(crate) fn prelude_trait(&self, name: &str) -> Option<Arc<str>> {
    let imported: Arc<str> = Arc::from(format!("Std.Prelude.{name}"));

    if self.env.traits.contains_key(&imported) {
      return Some(imported);
    }

    let local = self.trait_path(name);

    (self.local_traits.contains(name) && self.env.traits.contains_key(&local))
      .then_some(local)
  }

  pub(crate) fn qualified(&self, name: &str) -> String {
    match &self.qualifier {
      Some(q) => format!("{q}.{name}"),
      None => name.to_string(),
    }
  }

  pub(crate) fn record_expr(&mut self, span: &Span, ty: &Type) {
    self.exprs.insert((span.start, span.end), ty.clone());
  }

  pub(crate) fn finish_exprs(&mut self) -> HashMap<(usize, usize), Type> {
    let exprs = std::mem::take(&mut self.exprs);

    for ty in exprs.values() {
      default_numbers(&mut self.store, ty);
      default_brands(&mut self.store, ty);
    }

    exprs.into_iter().map(|(k, t)| (k, self.store.zonk(&t))).collect()
  }

  pub(crate) fn opened(&mut self, key: &Span, ty: Type) -> Type {
    if let Type::Fn { effects, .. } = &ty
      && effects.tail == EffTail::Closed
    {
      self.closed_rows.insert((key.start, key.end), effects.clone());
    }

    open_row(&mut self.store, ty)
  }

  pub(crate) fn record_signature(&mut self, span: &Span, ty: &Type) {
    self.signatures.insert((span.start, span.end), ty.clone());
  }
}

#[must_use]
pub fn last_expr(expr: &Expr) -> &Expr {
  match expr {
    Expr::Block(block) => last_expr(&block.result),
    other => other,
  }
}
