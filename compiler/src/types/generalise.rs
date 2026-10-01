use std::cmp::Ordering;

use crate::types::{
  store::Store,
  ty::{
    Brand, Constraint, EffTail, Effects, Pred, Row, Scheme, TVar, Tail, Type,
  },
};

#[must_use]
pub fn generalise(store: &Store, ty: &Type) -> Scheme {
  generalise_with(store, ty, &[], false)
}

#[must_use]
pub fn generalise_all(store: &Store, ty: &Type) -> Scheme {
  generalise_with(store, ty, &[], true)
}

#[must_use]
pub fn generalise_with(
  store: &Store,
  ty: &Type,
  preds: &[Pred],
  rigids: bool,
) -> Scheme {
  generalise_indexed(store, ty, preds, rigids).0
}

#[must_use]
pub fn generalise_indexed(
  store: &Store,
  ty: &Type,
  preds: &[Pred],
  rigids: bool,
) -> (Scheme, Vec<usize>) {
  let mut generaliser = Generaliser { store, slots: Vec::new(), rigids };
  let ty = generaliser.ty(&store.zonk(ty));
  let mut kept: Vec<(usize, Pred)> = Vec::new();

  for (i, pred) in preds.iter().enumerate() {
    let ty = generaliser.ty(&store.zonk(&pred.ty));

    if first_gen(&ty).is_some() {
      kept.push((i, Pred { trait_path: pred.trait_path.clone(), ty }));
    }
  }

  kept.sort_by(|(_, a), (_, b)| pred_order(a, b));

  let count = u32::try_from(generaliser.slots.len()).expect("too many slots");
  let order = kept.iter().map(|(i, _)| *i).collect();
  let preds = kept.into_iter().map(|(_, p)| p).collect();

  (Scheme { count, ty, preds }, order)
}

pub fn sort_preds(preds: &mut [Pred]) {
  preds.sort_by(pred_order);
}

fn pred_order(a: &Pred, b: &Pred) -> Ordering {
  first_gen(&a.ty)
    .cmp(&first_gen(&b.ty))
    .then_with(|| a.trait_path.cmp(&b.trait_path))
}

#[must_use]
pub fn first_gen(ty: &Type) -> Option<u32> {
  match ty {
    Type::Gen(i) => Some(*i),
    Type::Var(_) | Type::Rigid { .. } => None,
    Type::Con { args, .. } => args.iter().find_map(first_gen),
    Type::Fn { params, ret, effects } => {
      params.iter().find_map(first_gen).or_else(|| first_gen(ret)).or(
        match effects.tail {
          EffTail::Gen(i) => Some(i),
          EffTail::Closed | EffTail::Open(_) | EffTail::Rigid(_) => None,
        },
      )
    }
    Type::Record(row) => {
      row.fields.iter().find_map(|(_, t)| first_gen(t)).or(match row.tail {
        Tail::Gen(i) => Some(i),
        Tail::Closed(_) | Tail::Open(_) | Tail::Rigid(_) => None,
      })
    }
  }
}

pub fn instantiate(store: &mut Store, scheme: &Scheme) -> Type {
  instantiate_with_preds(store, scheme).0
}

pub fn instantiate_open(
  store: &mut Store,
  scheme: &Scheme,
) -> (Type, Vec<Pred>) {
  let (ty, preds) = instantiate_with_preds(store, scheme);

  (open_row(store, ty), preds)
}

pub fn open_row(store: &mut Store, ty: Type) -> Type {
  match ty {
    Type::Fn { params, ret, effects } if effects.tail == EffTail::Closed => {
      let tail = EffTail::Open(store.fresh());

      Type::Fn { params, ret, effects: Effects::new(effects.labels, tail) }
    }
    other => other,
  }
}

pub fn close_row(scheme: &mut Scheme) {
  let mut counts: Vec<(u32, usize)> = Vec::new();

  count_rows(&scheme.ty, &mut counts);

  for pred in &scheme.preds {
    count_rows(&pred.ty, &mut counts);
  }

  let once: Vec<u32> =
    counts.iter().filter(|(_, n)| *n == 1).map(|(i, _)| *i).collect();

  close_positive(&mut scheme.ty, &once);
}

pub fn seal_closed(store: &mut Store, found: &Type, scheme: &Type) {
  match (store.zonk(found), scheme) {
    (
      Type::Fn { ret, effects, .. },
      Type::Fn { ret: closed_ret, effects: closed, .. },
    ) => {
      if closed.tail == EffTail::Closed
        && let EffTail::Open(var) = effects.tail
      {
        store.set_effects(var, Effects::pure());
      }

      seal_closed(store, &ret, closed_ret);
    }
    (Type::Record(row), Type::Record(closed)) => {
      for (name, field) in &row.fields {
        if let Some((_, closed_field)) =
          closed.fields.iter().find(|(n, _)| n == name)
        {
          seal_closed(store, field, closed_field);
        }
      }
    }
    _ => {}
  }
}

fn count_rows(ty: &Type, counts: &mut Vec<(u32, usize)>) {
  let bump = |i: u32, counts: &mut Vec<(u32, usize)>| match counts
    .iter_mut()
    .find(|(j, _)| *j == i)
  {
    Some((_, n)) => *n += 1,
    None => counts.push((i, 1)),
  };

  match ty {
    Type::Gen(i) => bump(*i, counts),
    Type::Var(_) | Type::Rigid { .. } => {}
    Type::Con { args, .. } => {
      for arg in args {
        count_rows(arg, counts);
      }
    }
    Type::Fn { params, ret, effects } => {
      if let EffTail::Gen(i) = effects.tail {
        bump(i, counts);
      }

      for param in params {
        count_rows(param, counts);
      }

      count_rows(ret, counts);
    }
    Type::Record(row) => {
      if let Tail::Gen(i) = row.tail {
        bump(i, counts);
      }

      for (_, field) in &row.fields {
        count_rows(field, counts);
      }
    }
  }
}

fn close_positive(ty: &mut Type, once: &[u32]) {
  match ty {
    Type::Fn { ret, effects, .. } => {
      if let EffTail::Gen(i) = effects.tail
        && once.contains(&i)
      {
        effects.tail = EffTail::Closed;
      }

      close_positive(ret, once);
    }
    Type::Record(row) => {
      for (_, field) in &mut row.fields {
        close_positive(field, once);
      }
    }
    Type::Var(_) | Type::Rigid { .. } | Type::Gen(_) | Type::Con { .. } => {}
  }
}

pub fn instantiate_with_preds(
  store: &mut Store,
  scheme: &Scheme,
) -> (Type, Vec<Pred>) {
  let fresh: Vec<TVar> = (0..scheme.count).map(|_| store.fresh()).collect();
  let preds = scheme
    .preds
    .iter()
    .map(|p| Pred {
      trait_path: p.trait_path.clone(),
      ty: substitute(&p.ty, &fresh),
    })
    .collect();

  (substitute(&scheme.ty, &fresh), preds)
}

pub fn default_brands(store: &mut Store, ty: &Type) {
  match store.zonk(ty) {
    Type::Var(_) | Type::Gen(_) | Type::Rigid { .. } => {}
    Type::Con { args, .. } => {
      for arg in &args {
        default_brands(store, arg);
      }
    }
    Type::Fn { params, ret, .. } => {
      for param in &params {
        default_brands(store, param);
      }

      default_brands(store, &ret);
    }
    Type::Record(row) => {
      for (_, field) in &row.fields {
        default_brands(store, field);
      }

      if let Tail::Closed(Brand::Var(v)) = row.tail {
        store.set_brand(v, Brand::Anonymous);
      }
    }
  }
}

pub fn default_numbers(store: &mut Store, ty: &Type) {
  match store.zonk(ty) {
    Type::Var(v) => {
      if matches!(
        store.constraint_of(v),
        Some(Constraint::Num | Constraint::Ord)
      ) {
        store.set(v, Type::int());
      }
    }
    Type::Con { args, .. } => {
      for arg in &args {
        default_numbers(store, arg);
      }
    }
    Type::Fn { params, ret, .. } => {
      for param in &params {
        default_numbers(store, param);
      }

      default_numbers(store, &ret);
    }
    Type::Record(row) => {
      for (_, field) in &row.fields {
        default_numbers(store, field);
      }
    }
    Type::Gen(_) | Type::Rigid { .. } => {}
  }
}

struct Generaliser<'a> {
  store: &'a Store,
  slots: Vec<TVar>,
  rigids: bool,
}

impl Generaliser<'_> {
  fn index(&mut self, var: TVar) -> u32 {
    let i = self.slots.iter().position(|&v| v == var).unwrap_or_else(|| {
      self.slots.push(var);
      self.slots.len() - 1
    });

    u32::try_from(i).expect("too many slots")
  }

  fn slot(&mut self, var: TVar) -> Option<u32> {
    let level = self.store.level_of(var)?;

    if level <= self.store.current_level()
      || self.store.constraint_of(var) != Some(Constraint::None)
    {
      return None;
    }

    Some(self.index(var))
  }

  fn ty(&mut self, ty: &Type) -> Type {
    match ty {
      Type::Var(v) => match self.slot(*v) {
        Some(i) => Type::Gen(i),
        None => Type::Var(*v),
      },
      Type::Rigid { id, .. } if self.rigids => Type::Gen(self.index(*id)),
      Type::Rigid { name, id } => Type::Rigid { name: name.clone(), id: *id },
      Type::Con { name, args } => Type::Con {
        name: name.clone(),
        args: args.iter().map(|a| self.ty(a)).collect(),
      },
      Type::Fn { params, ret, effects } => Type::Fn {
        params: params.iter().map(|p| self.ty(p)).collect(),
        ret: Box::new(self.ty(ret)),
        effects: self.effects(effects),
      },
      Type::Record(row) => Type::Record(Row {
        fields: row
          .fields
          .iter()
          .map(|(n, t)| (n.clone(), self.ty(t)))
          .collect(),
        tail: match &row.tail {
          Tail::Open(v) => match self.slot(*v) {
            Some(i) => Tail::Gen(i),
            None => Tail::Open(*v),
          },
          Tail::Rigid(v) if self.rigids => Tail::Gen(self.index(*v)),
          tail @ (Tail::Closed(_) | Tail::Gen(_) | Tail::Rigid(_)) => {
            tail.clone()
          }
        },
      }),
      Type::Gen(i) => Type::Gen(*i),
    }
  }

  fn effects(&mut self, effects: &Effects) -> Effects {
    let tail = match effects.tail {
      EffTail::Open(v) => match self.slot(v) {
        Some(i) => EffTail::Gen(i),
        None => EffTail::Open(v),
      },
      EffTail::Rigid(v) if self.rigids => EffTail::Gen(self.index(v)),
      tail @ (EffTail::Closed | EffTail::Gen(_) | EffTail::Rigid(_)) => tail,
    };

    Effects::new(effects.labels.clone(), tail)
  }
}

fn substitute(ty: &Type, fresh: &[TVar]) -> Type {
  match ty {
    Type::Gen(i) => Type::Var(fresh[*i as usize]),
    Type::Var(v) => Type::Var(*v),
    Type::Rigid { name, id } => Type::Rigid { name: name.clone(), id: *id },
    Type::Con { name, args } => Type::Con {
      name: name.clone(),
      args: args.iter().map(|a| substitute(a, fresh)).collect(),
    },
    Type::Fn { params, ret, effects } => Type::Fn {
      params: params.iter().map(|p| substitute(p, fresh)).collect(),
      ret: Box::new(substitute(ret, fresh)),
      effects: Effects::new(
        effects.labels.clone(),
        match effects.tail {
          EffTail::Gen(i) => EffTail::Open(fresh[i as usize]),
          tail @ (EffTail::Closed | EffTail::Open(_) | EffTail::Rigid(_)) => {
            tail
          }
        },
      ),
    },
    Type::Record(row) => Type::Record(Row {
      fields: row
        .fields
        .iter()
        .map(|(n, t)| (n.clone(), substitute(t, fresh)))
        .collect(),
      tail: match &row.tail {
        Tail::Gen(i) => Tail::Open(fresh[*i as usize]),
        tail @ (Tail::Closed(_) | Tail::Open(_) | Tail::Rigid(_)) => {
          tail.clone()
        }
      },
    }),
  }
}
