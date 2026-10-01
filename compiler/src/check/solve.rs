use std::{
  collections::{HashMap, HashSet},
  sync::Arc,
};

use crate::{
  check::Checker,
  shared::codes::DiagnosticCode::{AmbiguousType, MissingImpl, MissingWhere},
  shared::diagnostic::{Diagnostic, Label},
  shared::ice::{ice, invariant},
  shared::source::Span,
  types::{
    generalise::{default_brands, default_numbers, generalise_indexed},
    print::{Printer, bare},
    ty::{Brand, Pred, Scheme, TVar, Tail, Type},
  },
};

#[derive(Debug, Clone, PartialEq)]
pub enum Evidence {
  Impl {
    trait_path: Arc<str>,
    target: Arc<str>,
    module: Option<String>,
    args: Vec<Evidence>,
  },
  Param(usize),
}

pub(crate) type Evidences =
  (HashMap<(usize, usize), Vec<Evidence>>, HashMap<(usize, usize), Evidence>);

#[derive(Debug, Clone)]
pub(crate) enum Source {
  Method,
  Function { name: String, requirement: String },
  Operator,
  Interpolation,
  Derive { ty: String, trait_name: String },
}

#[derive(Debug, Clone)]
pub(crate) struct Wanted {
  pub(crate) pred: Pred,
  pub(crate) key: Span,
  pub(crate) blame: Span,
  pub(crate) source: Source,
  pub(crate) owner: Option<usize>,
}

#[derive(Debug, Clone)]
pub(crate) struct GroupCall {
  pub(crate) key: Span,
  pub(crate) callee: usize,
  pub(crate) owner: usize,
}

#[derive(Debug, Clone)]
enum Ev {
  Impl {
    trait_path: Arc<str>,
    target: Arc<str>,
    module: Option<String>,
    args: Vec<Ev>,
  },
  Given(usize),
  Hole(usize),
}

#[derive(Debug, Clone)]
enum Fail {
  Missing { pred: Pred, chain: Vec<Pred> },
  NoWhere(Pred),
  Ambiguous(Pred),
}

struct Holes {
  preds: Vec<Pred>,
  vars: HashSet<TVar>,
  open: bool,
}

impl Holes {
  fn closed() -> Self {
    Self { preds: Vec::new(), vars: HashSet::new(), open: false }
  }

  fn hole(&mut self, pred: &Pred) -> usize {
    if let Some(i) = self.preds.iter().position(|p| p == pred) {
      return i;
    }

    self.preds.push(pred.clone());
    self.preds.len() - 1
  }
}

impl Checker<'_> {
  pub(crate) fn want(
    &mut self,
    preds: Vec<Pred>,
    key: &Span,
    blame: &Span,
    source: &Source,
  ) {
    let source = match self.derived.get(&(blame.start, blame.end)) {
      Some((ty, trait_name)) => {
        Source::Derive { ty: ty.clone(), trait_name: trait_name.clone() }
      }
      None => source.clone(),
    };

    for pred in preds {
      let id = self.wanted.len();
      let uses = match &source {
        Source::Interpolation => &mut self.interp_uses,
        _ => &mut self.uses,
      };

      uses.entry((key.start, key.end)).or_default().push(id);
      self.wanted.push(Wanted {
        pred,
        key: key.clone(),
        blame: blame.clone(),
        source: source.clone(),
        owner: self.owner,
      });
    }
  }

  fn defaults(&mut self, start: usize) {
    for i in start..self.wanted.len() {
      let ty = self.wanted[i].pred.ty.clone();

      default_numbers(&mut self.store, &ty);
      default_brands(&mut self.store, &ty);
    }
  }

  pub(crate) fn solve_closed(&mut self, start: usize) {
    self.defaults(start);

    let mut holes = Holes::closed();
    let mut failures = Vec::new();

    for i in start..self.wanted.len() {
      let pred = self.wanted[i].pred.clone();

      match self.reduce(&pred, &mut holes) {
        Ok(ev) => {
          let evidence = finish(&ev, &|_| None).unwrap_or_else(|| {
            ice("a hole in a closed declaration's evidence", None)
          });

          self.solved.insert(i, evidence);
        }
        Err(fail) => failures.push((i, fail)),
      }
    }

    self.report_all(failures);
  }

  fn report_all(&mut self, failures: Vec<(usize, Fail)>) {
    let mut ambiguous: Vec<(usize, Pred)> = Vec::new();

    for (i, fail) in failures {
      match fail {
        Fail::Ambiguous(pred) => ambiguous.push((i, pred)),
        other => self.report(i, &other),
      }
    }

    let mut done: HashSet<usize> = HashSet::new();

    for (i, pred) in &ambiguous {
      if done.contains(i) {
        continue;
      }

      let vars = free_vars(&pred.ty);
      let related: Vec<usize> = ambiguous
        .iter()
        .filter(|(_, p)| {
          !free_vars(&p.ty).is_disjoint(&vars) || vars.is_empty()
        })
        .map(|(j, _)| *j)
        .collect();
      let chosen = related
        .iter()
        .copied()
        .max_by_key(|&j| self.wanted[j].blame.start)
        .unwrap_or(*i);

      for &j in &related {
        done.insert(j);

        if j != chosen {
          let key = &self.wanted[j].key;

          self.reported.insert((key.start, key.end));
        }
      }

      let pred = ambiguous
        .iter()
        .find(|(j, _)| *j == chosen)
        .map_or_else(|| pred.clone(), |(_, p)| p.clone());

      self.report(chosen, &Fail::Ambiguous(pred));
    }
  }

  pub(crate) fn check_leftovers(&self) {
    if self.bag.has_errors() {
      return;
    }

    for (i, wanted) in self.wanted.iter().enumerate() {
      let key = (wanted.key.start, wanted.key.end);

      invariant(
        self.solved.contains_key(&i) || self.reported.contains(&key),
        || format!("a wanted `{}` was never solved", wanted.pred.trait_path),
      );
    }
  }

  pub(crate) fn solve_group(
    &mut self,
    start: usize,
    types: &[Type],
    givens: &[Vec<Pred>],
    calls: &[GroupCall],
  ) -> Vec<Scheme> {
    self.defaults(start);

    let member_vars: Vec<HashSet<TVar>> =
      types.iter().map(|t| free_vars(&self.store.zonk(t))).collect();
    let mut holes = Holes {
      preds: Vec::new(),
      vars: member_vars.iter().flatten().copied().collect(),
      open: true,
    };

    for given in givens.iter().flatten() {
      holes.hole(given);
    }

    let results: Vec<(usize, Result<Ev, Fail>)> = (start..self.wanted.len())
      .map(|i| {
        let pred = self.wanted[i].pred.clone();

        (i, self.reduce_in(&pred, &mut holes, self.wanted[i].owner, givens))
      })
      .collect();

    let mut schemes = Vec::with_capacity(types.len());
    let mut orders: Vec<Vec<usize>> = Vec::with_capacity(types.len());

    for (ty, vars) in types.iter().zip(&member_vars) {
      let mine: Vec<usize> = (0..holes.preds.len())
        .filter(|&h| pred_vars(&self.store, &holes.preds[h]).is_subset(vars))
        .collect();
      let preds: Vec<Pred> =
        mine.iter().map(|&h| holes.preds[h].clone()).collect();
      let (scheme, order) = generalise_indexed(&self.store, ty, &preds, true);

      orders.push(order.into_iter().map(|k| mine[k]).collect());
      schemes.push(scheme);
    }

    let mut failures = Vec::new();

    for (i, result) in results {
      let owner = self.wanted[i].owner;
      let order = owner.and_then(|o| orders.get(o));

      match result {
        Ok(ev) => {
          let lookup = |h: usize| order?.iter().position(|&x| x == h);

          if let Some(evidence) = finish(&ev, &lookup) {
            self.solved.insert(i, evidence);
          } else {
            let pred = self.zonked(&self.wanted[i].pred.clone());

            failures.push((i, Fail::Ambiguous(pred)));
          }
        }
        Err(fail) => failures.push((i, fail)),
      }
    }

    self.report_all(failures);

    for call in calls {
      let (Some(callee), Some(owner)) =
        (orders.get(call.callee), orders.get(call.owner))
      else {
        continue;
      };
      let evidence: Option<Vec<Evidence>> = callee
        .iter()
        .map(|h| owner.iter().position(|x| x == h).map(Evidence::Param))
        .collect();

      if let Some(evidence) = evidence {
        self.group_evidence.insert((call.key.start, call.key.end), evidence);
      } else {
        let key = (call.key.start, call.key.end);

        if self.reported.insert(key) {
          self.push(
            Diagnostic::error(
              AmbiguousType,
              "cannot tell which type this is",
              Label::new(call.key.clone()).with_message(
                "this call needs a trait for a type the caller doesn't know",
              ),
            )
            .with_help("add a type annotation to the function"),
          );
        }
      }
    }

    schemes
  }

  pub(crate) fn evidence_for(&mut self, pred: &Pred) -> Option<Evidence> {
    let ev = self.reduce(pred, &mut Holes::closed()).ok()?;

    finish(&ev, &|_| None)
  }

  fn reduce(&mut self, pred: &Pred, holes: &mut Holes) -> Result<Ev, Fail> {
    self.reduce_in(pred, holes, None, &[])
  }

  fn reduce_in(
    &mut self,
    pred: &Pred,
    holes: &mut Holes,
    owner: Option<usize>,
    group_givens: &[Vec<Pred>],
  ) -> Result<Ev, Fail> {
    let ty = self.store.resolve(&pred.ty);
    let trait_path = &pred.trait_path;

    match &ty {
      Type::Var(v) => {
        let generalisable = self
          .store
          .level_of(*v)
          .is_some_and(|level| level > self.store.current_level());

        if holes.open && generalisable && holes.vars.contains(v) {
          Ok(Ev::Hole(holes.hole(&Pred { trait_path: trait_path.clone(), ty })))
        } else {
          Err(Fail::Ambiguous(self.zonked(pred)))
        }
      }
      Type::Rigid { id, .. } => {
        let matches = |g: &Pred| {
          g.trait_path == *trait_path
            && matches!(&g.ty, Type::Rigid { id: other, .. } if other == id)
        };

        if holes.open {
          let given = owner
            .and_then(|o| group_givens.get(o))
            .and_then(|gs| gs.iter().find(|g| matches(g)));

          return match given {
            Some(given) => Ok(Ev::Hole(holes.hole(given))),
            None => Err(Fail::NoWhere(self.zonked(pred))),
          };
        }

        match self.givens.iter().position(matches) {
          Some(k) => Ok(Ev::Given(k)),
          None => Err(Fail::NoWhere(self.zonked(pred))),
        }
      }
      Type::Con { name, args } => {
        self.by_impl(pred, name, args, holes, owner, group_givens)
      }
      Type::Record(row) => {
        let row = self.store.zonk_row(row);

        match &row.tail {
          Tail::Closed(Brand::Named(name)) => {
            self.by_impl(pred, name, &[], holes, owner, group_givens)
          }
          Tail::Closed(Brand::Anonymous) | Tail::Rigid(_) | Tail::Gen(_) => {
            Err(Fail::Missing { pred: self.zonked(pred), chain: Vec::new() })
          }
          Tail::Closed(Brand::Var(_)) | Tail::Open(_) => {
            Err(Fail::Ambiguous(self.zonked(pred)))
          }
        }
      }
      Type::Fn { .. } => {
        Err(Fail::Missing { pred: self.zonked(pred), chain: Vec::new() })
      }
      Type::Gen(_) => Err(Fail::Ambiguous(self.zonked(pred))),
    }
  }

  fn by_impl(
    &mut self,
    pred: &Pred,
    name: &Arc<str>,
    args: &[Type],
    holes: &mut Holes,
    owner: Option<usize>,
    group_givens: &[Vec<Pred>],
  ) -> Result<Ev, Fail> {
    let key = (pred.trait_path.clone(), name.clone());
    let Some(def) = self.env.impls.get(&key).cloned() else {
      return Err(Fail::Missing { pred: self.zonked(pred), chain: Vec::new() });
    };
    let mut evidence = Vec::with_capacity(def.bounds.len());

    for (bound, index) in &def.bounds {
      let Some(arg) = args.get(*index) else {
        return Err(Fail::Missing {
          pred: self.zonked(pred),
          chain: Vec::new(),
        });
      };
      let sub = Pred { trait_path: bound.clone(), ty: arg.clone() };

      match self.reduce_in(&sub, holes, owner, group_givens) {
        Ok(ev) => evidence.push(ev),
        Err(Fail::Missing { pred: inner, mut chain }) => {
          chain.push(self.zonked(pred));

          return Err(Fail::Missing { pred: inner, chain });
        }
        Err(other) => return Err(other),
      }
    }

    Ok(Ev::Impl {
      trait_path: def.trait_path.clone(),
      target: def.target.clone(),
      module: def.module.clone(),
      args: evidence,
    })
  }

  fn zonked(&self, pred: &Pred) -> Pred {
    Pred { trait_path: pred.trait_path.clone(), ty: self.store.zonk(&pred.ty) }
  }

  fn report(&mut self, id: usize, fail: &Fail) {
    let wanted = self.wanted[id].clone();
    let key = (wanted.key.start, wanted.key.end);

    if !self.reported.insert(key) {
      return;
    }

    let diagnostic = match fail {
      Fail::Missing { pred, chain } => missing_impl(&wanted, pred, chain),
      Fail::NoWhere(pred) => self.missing_where(&wanted, pred),
      Fail::Ambiguous(pred) => ambiguous(&wanted, pred),
    };

    self.push(diagnostic);
  }

  fn missing_where(&self, wanted: &Wanted, pred: &Pred) -> Diagnostic {
    let mut printer = Printer::new(&[&pred.ty], true);
    let ty = printer.print(&pred.ty);
    let name = trait_name(&pred.trait_path);
    let mut diagnostic = Diagnostic::error(
      MissingWhere,
      format!("the annotation `{ty}` doesn't promise `{name}`"),
      Label::new(wanted.blame.clone())
        .with_message(format!("this needs `{name}<{ty}>`")),
    )
    .with_help(format!("add `where {name}<{ty}>` after the return type"));

    if let Some(signature) = &self.signature_span {
      diagnostic = diagnostic.with_secondary(
        Label::new(signature.clone())
          .with_message(format!("`{ty}` is declared here")),
      );
    }

    diagnostic
  }

  pub(crate) fn finish_evidence(&mut self) -> Evidences {
    let mut out = std::mem::take(&mut self.group_evidence);

    for (key, ids) in &self.uses {
      let evidence: Option<Vec<Evidence>> =
        ids.iter().map(|id| self.solved.get(id).cloned()).collect();

      if let Some(evidence) = evidence {
        out.insert(*key, evidence);
      }
    }

    let shows = self
      .interp_uses
      .iter()
      .filter_map(|(key, ids)| {
        let evidence = self.solved.get(ids.first()?)?.clone();

        Some((*key, evidence))
      })
      .collect();

    (out, shows)
  }
}

fn finish(
  ev: &Ev,
  lookup: &dyn Fn(usize) -> Option<usize>,
) -> Option<Evidence> {
  Some(match ev {
    Ev::Impl { trait_path, target, module, args } => Evidence::Impl {
      trait_path: trait_path.clone(),
      target: target.clone(),
      module: module.clone(),
      args: args.iter().map(|a| finish(a, lookup)).collect::<Option<_>>()?,
    },
    Ev::Given(k) => Evidence::Param(*k),
    Ev::Hole(h) => Evidence::Param(lookup(*h)?),
  })
}

#[must_use]
pub fn trait_name(path: &str) -> &str {
  bare(path)
}

fn pred_vars(store: &crate::types::store::Store, pred: &Pred) -> HashSet<TVar> {
  free_vars(&store.zonk(&pred.ty))
}

#[must_use]
pub fn free_vars(ty: &Type) -> HashSet<TVar> {
  fn go(ty: &Type, out: &mut HashSet<TVar>) {
    match ty {
      Type::Var(v) | Type::Rigid { id: v, .. } => {
        out.insert(*v);
      }
      Type::Gen(_) => {}
      Type::Con { args, .. } => args.iter().for_each(|a| go(a, out)),
      Type::Fn { params, ret, effects } => {
        for p in params {
          go(p, out);
        }
        go(ret, out);

        if let crate::types::ty::EffTail::Open(v)
        | crate::types::ty::EffTail::Rigid(v) = effects.tail
        {
          out.insert(v);
        }
      }
      Type::Record(row) => {
        for (_, t) in &row.fields {
          go(t, out);
        }

        match &row.tail {
          Tail::Open(v) | Tail::Rigid(v) | Tail::Closed(Brand::Var(v)) => {
            out.insert(*v);
          }
          Tail::Closed(_) | Tail::Gen(_) => {}
        }
      }
    }
  }

  let mut out = HashSet::new();

  go(ty, &mut out);
  out
}

fn missing_impl(wanted: &Wanted, pred: &Pred, chain: &[Pred]) -> Diagnostic {
  let mut all: Vec<&Type> = vec![&pred.ty];

  all.extend(chain.iter().map(|p| &p.ty));

  let mut printer = Printer::new(&all, true);
  let ty = printer.print(&pred.ty);
  let name = trait_name(&pred.trait_path);
  let chain: Vec<String> = chain
    .iter()
    .map(|p| format!("{}<{}>", trait_name(&p.trait_path), printer.print(&p.ty)))
    .collect();

  let mut diagnostic = match &wanted.source {
    Source::Derive { ty: owner, trait_name } => Diagnostic::error(
      MissingImpl,
      format!("`{owner}` can't derive `{trait_name}`"),
      Label::new(wanted.blame.clone())
        .with_message(format!("`{ty}` has no `{name}` impl")),
    ),
    _ => Diagnostic::error(
      MissingImpl,
      format!("`{ty}` has no `{name}` impl"),
      Label::new(wanted.blame.clone())
        .with_message(format!("this needs `{name}`")),
    ),
  };

  if let Source::Function { name, requirement } = &wanted.source {
    diagnostic = diagnostic.with_note(format!(
      "required by `{name}`, which says `where {requirement}`"
    ));
  }

  if let Some(outer) = chain.last() {
    diagnostic = diagnostic.with_note(format!("needed for `{outer}`"));
  }

  diagnostic
}

fn ambiguous(wanted: &Wanted, pred: &Pred) -> Diagnostic {
  let name = trait_name(&pred.trait_path);

  Diagnostic::error(
    AmbiguousType,
    "cannot tell which type this is",
    Label::new(wanted.blame.clone()).with_message(format!(
      "the type of this is unknown, and it must have `{name}`"
    )),
  )
  .with_help("add a type annotation, like `let x: Post = …`")
}
