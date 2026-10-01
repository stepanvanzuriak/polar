#![allow(
  clippy::result_large_err,
  reason = "a failed unification carries both types, which the diagnostic prints"
)]

use std::sync::Arc;

use crate::types::{
  store::Store,
  ty::{Brand, Constraint, EffTail, Effects, Label, Row, TVar, Tail, Type},
};

#[derive(Debug, Clone, PartialEq)]
pub enum UnifyError {
  Mismatch { expected: Type, found: Type },
  Occurs { var: TVar, ty: Type },
  ParamCount { expected: usize, found: usize },
  MissingField { label: Arc<str>, record: Type },
  ExtraField { label: Arc<str>, record: Type },
  NotNumeric { constraint: Constraint, found: Type },
  RigidEscape { rigid: Type, other: Type },
  MissingEffect { label: Label, row: Effects },
  ExtraEffect { label: Label, row: Effects },
  EffectMismatch { expected: Effects, found: Effects },
  EffectEscape { expected: Effects, found: Effects },
}

type Fields = Vec<(Arc<str>, Type)>;

pub fn unify(
  store: &mut Store,
  expected: &Type,
  found: &Type,
) -> Result<(), UnifyError> {
  let expected = store.resolve(expected);
  let found = store.resolve(found);

  match (&expected, &found) {
    (Type::Var(a), Type::Var(b)) if a == b => Ok(()),
    (Type::Var(a), Type::Var(b)) => {
      link(store, *a, *b);
      Ok(())
    }
    (Type::Var(v), other) | (other, Type::Var(v)) => bind(store, *v, other),
    (Type::Rigid { id: a, .. }, Type::Rigid { id: b, .. }) if a == b => Ok(()),
    (Type::Rigid { .. }, other) => Err(UnifyError::RigidEscape {
      rigid: expected.clone(),
      other: store.zonk(other),
    }),
    (other, Type::Rigid { .. }) => Err(UnifyError::RigidEscape {
      rigid: found.clone(),
      other: store.zonk(other),
    }),
    (Type::Con { name: n1, args: a1 }, Type::Con { name: n2, args: a2 }) => {
      if n1 != n2 || a1.len() != a2.len() {
        return Err(mismatch(store, &expected, &found));
      }

      for (a, b) in a1.iter().zip(a2) {
        match unify(store, a, b) {
          Err(UnifyError::Mismatch { .. }) => {
            return Err(mismatch(store, &expected, &found));
          }
          other => other?,
        }
      }

      Ok(())
    }
    (
      Type::Fn { params: p1, ret: r1, effects: e1 },
      Type::Fn { params: p2, ret: r2, effects: e2 },
    ) => {
      if p1.len() != p2.len() {
        return Err(UnifyError::ParamCount {
          expected: p1.len(),
          found: p2.len(),
        });
      }

      for (a, b) in p1.iter().zip(p2) {
        unify(store, a, b)?;
      }

      unify(store, r1, r2)?;

      match unify_effects(store, e1, e2) {
        Ok(()) => Ok(()),
        Err(
          UnifyError::MissingEffect { .. }
          | UnifyError::ExtraEffect { .. }
          | UnifyError::EffectMismatch { .. }
          | UnifyError::EffectEscape { .. },
        ) => Err(mismatch(store, &expected, &found)),
        Err(other) => Err(other),
      }
    }
    (Type::Record(e), Type::Record(f)) => unify_rows(store, e, f),
    (
      Type::Con { .. } | Type::Fn { .. } | Type::Record(_) | Type::Gen(_),
      _,
    ) => Err(mismatch(store, &expected, &found)),
  }
}

fn mismatch(store: &Store, expected: &Type, found: &Type) -> UnifyError {
  UnifyError::Mismatch {
    expected: store.zonk(expected),
    found: store.zonk(found),
  }
}

fn link(store: &mut Store, a: TVar, b: TVar) {
  let level_a = store.level_of(a).expect("resolved placeholder is unknown");
  let level_b = store.level_of(b).expect("resolved placeholder is unknown");
  let (survivor, loser) = if level_a <= level_b { (a, b) } else { (b, a) };
  let constraint = store.constraint_of(loser).unwrap_or(Constraint::None);

  store.restrict(survivor, constraint);
  store.set(loser, Type::Var(survivor));
}

fn bind(store: &mut Store, var: TVar, ty: &Type) -> Result<(), UnifyError> {
  let constraint = store.constraint_of(var).unwrap_or(Constraint::None);
  let admitted = match ty {
    Type::Con { name, .. } => constraint.admits(name),
    Type::Fn { .. } | Type::Record(_) | Type::Gen(_) | Type::Rigid { .. } => {
      constraint == Constraint::None
    }
    Type::Var(_) => true,
  };

  if !admitted {
    return Err(UnifyError::NotNumeric { constraint, found: store.zonk(ty) });
  }

  let ty = store.zonk(ty);
  let level = store.level_of(var).expect("bound placeholder is unknown");

  if occurs_and_lower(store, var, level, &ty) {
    return Err(UnifyError::Occurs { var, ty });
  }

  store.set(var, ty);
  Ok(())
}

fn bind_row(store: &mut Store, var: TVar, row: &Row) -> Result<(), UnifyError> {
  let level = store.level_of(var).expect("bound row is unknown");
  let row = store.zonk_row(row);

  if occurs_in_row(store, var, level, &row) {
    return Err(UnifyError::Occurs { var, ty: Type::Record(row) });
  }

  store.set_row(var, row);
  Ok(())
}

fn occurs_and_lower(
  store: &mut Store,
  var: TVar,
  level: u32,
  ty: &Type,
) -> bool {
  match ty {
    Type::Var(v) => {
      if *v == var {
        return true;
      }

      store.lower(*v, level);
      false
    }
    Type::Con { args, .. } => {
      args.iter().any(|a| occurs_and_lower(store, var, level, a))
    }
    Type::Fn { params, ret, effects } => {
      if let EffTail::Open(v) = effects.tail {
        store.lower(v, level);
      }

      params.iter().any(|p| occurs_and_lower(store, var, level, p))
        || occurs_and_lower(store, var, level, ret)
    }
    Type::Record(row) => occurs_in_row(store, var, level, row),
    Type::Gen(_) | Type::Rigid { .. } => false,
  }
}

fn occurs_in_row(store: &mut Store, var: TVar, level: u32, row: &Row) -> bool {
  if row.fields.iter().any(|(_, t)| occurs_and_lower(store, var, level, t)) {
    return true;
  }

  match row.tail {
    Tail::Open(v) => {
      if v == var {
        return true;
      }

      store.lower(v, level);
      false
    }
    Tail::Closed(_) | Tail::Gen(_) | Tail::Rigid(_) => false,
  }
}

fn split(expected: &Row, found: &Row) -> (Vec<(Type, Type)>, Fields, Fields) {
  let mut shared = Vec::new();
  let mut only_expected = Vec::new();

  for (label, ty) in &expected.fields {
    match found.fields.iter().find(|(l, _)| l == label) {
      Some((_, other)) => shared.push((ty.clone(), other.clone())),
      None => only_expected.push((label.clone(), ty.clone())),
    }
  }

  let mut only_found: Fields = found
    .fields
    .iter()
    .filter(|(l, _)| !expected.fields.iter().any(|(e, _)| e == l))
    .cloned()
    .collect();

  only_expected.sort_by(|a, b| a.0.cmp(&b.0));
  only_found.sort_by(|a, b| a.0.cmp(&b.0));

  (shared, only_expected, only_found)
}

fn is_unknown(store: &Store, tail: &Tail) -> bool {
  match tail {
    Tail::Open(v) => store.level_of(*v).is_some(),
    Tail::Closed(_) | Tail::Gen(_) | Tail::Rigid(_) => true,
  }
}

fn unify_brands(
  store: &mut Store,
  expected: &Brand,
  found: &Brand,
) -> Result<(), ()> {
  let expected = store.resolve_brand(expected);
  let found = store.resolve_brand(found);

  match (&expected, &found) {
    (Brand::Var(a), Brand::Var(b)) if a == b => Ok(()),
    (Brand::Var(v), other) | (other, Brand::Var(v)) => {
      store.set_brand(*v, other.clone());
      Ok(())
    }
    (Brand::Named(a), Brand::Named(b)) if a != b => Err(()),
    (
      Brand::Named(_) | Brand::Anonymous,
      Brand::Named(_) | Brand::Anonymous,
    ) => Ok(()),
  }
}

fn unify_rows(
  store: &mut Store,
  expected: &Row,
  found: &Row,
) -> Result<(), UnifyError> {
  let expected = store.zonk_row(expected);
  let found = store.zonk_row(found);
  let (shared, only_expected, only_found) = split(&expected, &found);

  for (e, f) in &shared {
    unify(store, e, f)?;
  }

  if !is_unknown(store, &expected.tail) || !is_unknown(store, &found.tail) {
    return unify_rows(store, &expected, &found);
  }

  let missing = |store: &Store| UnifyError::MissingField {
    label: only_expected[0].0.clone(),
    record: Type::Record(store.zonk_row(&found)),
  };
  let extra = |store: &Store| UnifyError::ExtraField {
    label: only_found[0].0.clone(),
    record: Type::Record(store.zonk_row(&found)),
  };

  match (expected.tail.clone(), found.tail.clone()) {
    (Tail::Open(r1), Tail::Open(r2)) if r1 == r2 => {
      if only_expected.is_empty() && only_found.is_empty() {
        Ok(())
      } else {
        Err(UnifyError::Mismatch {
          expected: Type::Record(store.zonk_row(&expected)),
          found: Type::Record(store.zonk_row(&found)),
        })
      }
    }
    (Tail::Open(r1), Tail::Open(r2)) => {
      let level =
        store.level_of(r1).unwrap_or(0).min(store.level_of(r2).unwrap_or(0));
      let r3 = store.fresh();
      store.lower(r3, level);

      bind_row(store, r1, &Row::new(only_found, Tail::Open(r3)))?;
      bind_row(store, r2, &Row::new(only_expected, Tail::Open(r3)))
    }
    (
      Tail::Open(r1),
      tail @ (Tail::Closed(_) | Tail::Gen(_) | Tail::Rigid(_)),
    ) => {
      if !only_expected.is_empty() {
        return Err(missing(store));
      }

      bind_row(store, r1, &Row::new(only_found, tail))
    }
    (
      tail @ (Tail::Closed(_) | Tail::Gen(_) | Tail::Rigid(_)),
      Tail::Open(r2),
    ) => {
      if !only_found.is_empty() {
        return Err(extra(store));
      }

      bind_row(store, r2, &Row::new(only_expected, tail))
    }
    (
      e @ (Tail::Closed(_) | Tail::Gen(_) | Tail::Rigid(_)),
      f @ (Tail::Closed(_) | Tail::Gen(_) | Tail::Rigid(_)),
    ) => {
      if !only_expected.is_empty() {
        return Err(missing(store));
      }

      if !only_found.is_empty() {
        return Err(extra(store));
      }

      match (e, f) {
        (Tail::Rigid(a), Tail::Rigid(b)) if a == b => Ok(()),
        (Tail::Rigid(_), _) => Err(UnifyError::RigidEscape {
          rigid: Type::Record(store.zonk_row(&expected)),
          other: Type::Record(store.zonk_row(&found)),
        }),
        (_, Tail::Rigid(_)) => Err(UnifyError::RigidEscape {
          rigid: Type::Record(store.zonk_row(&found)),
          other: Type::Record(store.zonk_row(&expected)),
        }),
        (Tail::Closed(a), Tail::Closed(b)) => unify_brands(store, &a, &b)
          .map_err(|()| UnifyError::Mismatch {
            expected: Type::Record(store.zonk_row(&expected)),
            found: Type::Record(store.zonk_row(&found)),
          }),
        _ => Ok(()),
      }
    }
  }
}

fn split_labels(
  expected: &Effects,
  found: &Effects,
) -> (Vec<Label>, Vec<Label>) {
  let only_expected = expected
    .labels
    .iter()
    .filter(|l| !found.labels.contains(l))
    .cloned()
    .collect();
  let only_found = found
    .labels
    .iter()
    .filter(|l| !expected.labels.contains(l))
    .cloned()
    .collect();

  (only_expected, only_found)
}

fn bind_effects(store: &mut Store, var: TVar, effects: Effects) {
  let level = store.level_of(var).expect("bound row is unknown");

  if let EffTail::Open(tail) = effects.tail {
    store.lower(tail, level);
  }

  store.set_effects(var, effects);
}

pub fn unify_effects(
  store: &mut Store,
  expected: &Effects,
  found: &Effects,
) -> Result<(), UnifyError> {
  let expected = store.zonk_effects(expected);
  let found = store.zonk_effects(found);
  let (only_expected, only_found) = split_labels(&expected, &found);

  match (expected.tail, found.tail) {
    (EffTail::Open(r1), EffTail::Open(r2)) if r1 == r2 => {
      if only_expected.is_empty() && only_found.is_empty() {
        Ok(())
      } else {
        Err(UnifyError::EffectMismatch { expected, found })
      }
    }
    (EffTail::Open(r1), EffTail::Open(r2)) => {
      let level =
        store.level_of(r1).unwrap_or(0).min(store.level_of(r2).unwrap_or(0));
      let r3 = store.fresh();

      store.lower(r3, level);
      bind_effects(store, r1, Effects::new(only_found, EffTail::Open(r3)));
      bind_effects(store, r2, Effects::new(only_expected, EffTail::Open(r3)));
      Ok(())
    }
    (EffTail::Open(r1), tail) => {
      if let Some(label) = only_expected.into_iter().next() {
        return Err(UnifyError::MissingEffect { label, row: found });
      }

      bind_effects(store, r1, Effects::new(only_found, tail));
      Ok(())
    }
    (tail, EffTail::Open(r2)) => {
      if let Some(label) = only_found.into_iter().next() {
        return Err(UnifyError::ExtraEffect { label, row: expected });
      }

      bind_effects(store, r2, Effects::new(only_expected, tail));
      Ok(())
    }
    (e, f) => {
      let same_tail = match (e, f) {
        (EffTail::Rigid(a), EffTail::Rigid(b)) => a == b,
        (EffTail::Gen(a), EffTail::Gen(b)) => a == b,
        (EffTail::Closed, EffTail::Closed) => true,
        _ => false,
      };
      let rigid = matches!(e, EffTail::Rigid(_) | EffTail::Gen(_))
        || matches!(f, EffTail::Rigid(_) | EffTail::Gen(_));

      if rigid && !same_tail {
        return Err(UnifyError::EffectEscape { expected, found });
      }

      if let Some(label) = only_found.into_iter().next() {
        return Err(UnifyError::ExtraEffect { label, row: expected });
      }

      if let Some(label) = only_expected.into_iter().next() {
        return Err(UnifyError::MissingEffect { label, row: found });
      }

      Ok(())
    }
  }
}
