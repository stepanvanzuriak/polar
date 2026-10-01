use crate::types::ty::{
  Brand, Constraint, EffTail, Effects, Row, TVar, Tail, Type,
};

#[derive(Debug, Clone)]
enum Slot {
  Unknown { level: u32, constraint: Constraint },
  Is(Type),
  IsRow(Row),
  IsBrand(Brand),
  IsEffects(Effects),
}

#[derive(Debug, Default, Clone)]
pub struct Store {
  slots: Vec<Slot>,
  level: u32,
}

impl Store {
  pub fn fresh(&mut self) -> TVar {
    self.fresh_constrained(Constraint::None)
  }

  pub fn fresh_constrained(&mut self, constraint: Constraint) -> TVar {
    let id = u32::try_from(self.slots.len()).expect("too many type variables");
    self.slots.push(Slot::Unknown { level: self.level, constraint });
    TVar(id)
  }

  pub fn fresh_brand(&mut self) -> Brand {
    Brand::Var(self.fresh())
  }

  pub fn set_brand(&mut self, var: TVar, brand: Brand) {
    let slot = &mut self.slots[var.0 as usize];
    debug_assert!(
      matches!(slot, Slot::Unknown { .. }),
      "{var:?} is already bound"
    );
    *slot = Slot::IsBrand(brand);
  }

  #[must_use]
  pub fn resolve_brand(&self, brand: &Brand) -> Brand {
    let mut brand = brand.clone();

    while let Brand::Var(v) = brand {
      match &self.slots[v.0 as usize] {
        Slot::IsBrand(next) => brand = next.clone(),
        Slot::Unknown { .. } => break,
        Slot::Is(_) | Slot::IsRow(_) | Slot::IsEffects(_) => {
          unreachable!("{v:?} is not a brand variable")
        }
      }
    }

    brand
  }

  pub fn enter_level(&mut self) {
    self.level += 1;
  }

  pub fn leave_level(&mut self) {
    self.level -= 1;
  }

  #[must_use]
  pub fn current_level(&self) -> u32 {
    self.level
  }

  #[must_use]
  pub fn level_of(&self, var: TVar) -> Option<u32> {
    match self.slots[var.0 as usize] {
      Slot::Unknown { level, .. } => Some(level),
      Slot::Is(_) | Slot::IsRow(_) | Slot::IsBrand(_) | Slot::IsEffects(_) => {
        None
      }
    }
  }

  #[must_use]
  pub fn constraint_of(&self, var: TVar) -> Option<Constraint> {
    match self.slots[var.0 as usize] {
      Slot::Unknown { constraint, .. } => Some(constraint),
      Slot::Is(_) | Slot::IsRow(_) | Slot::IsBrand(_) | Slot::IsEffects(_) => {
        None
      }
    }
  }

  pub fn lower(&mut self, var: TVar, to: u32) {
    if let Slot::Unknown { level, .. } = &mut self.slots[var.0 as usize] {
      *level = (*level).min(to);
    }
  }

  pub fn restrict(&mut self, var: TVar, with: Constraint) {
    if let Slot::Unknown { constraint, .. } = &mut self.slots[var.0 as usize] {
      *constraint = constraint.meet(with);
    }
  }

  pub fn set(&mut self, var: TVar, ty: Type) {
    let slot = &mut self.slots[var.0 as usize];
    debug_assert!(
      matches!(slot, Slot::Unknown { .. }),
      "{var:?} is already bound"
    );
    *slot = Slot::Is(ty);
  }

  pub fn set_row(&mut self, var: TVar, row: Row) {
    let slot = &mut self.slots[var.0 as usize];
    debug_assert!(
      matches!(slot, Slot::Unknown { .. }),
      "{var:?} is already bound"
    );
    *slot = Slot::IsRow(row);
  }

  pub fn set_effects(&mut self, var: TVar, effects: Effects) {
    let slot = &mut self.slots[var.0 as usize];
    debug_assert!(
      matches!(slot, Slot::Unknown { .. }),
      "{var:?} is already bound"
    );
    *slot = Slot::IsEffects(effects);
  }

  #[must_use]
  pub fn zonk_effects(&self, effects: &Effects) -> Effects {
    let mut labels = effects.labels.clone();
    let mut tail = effects.tail;

    while let EffTail::Open(v) = tail {
      match &self.slots[v.0 as usize] {
        Slot::IsEffects(more) => {
          labels.extend(more.labels.iter().cloned());
          tail = more.tail;
        }
        Slot::Unknown { .. } => break,
        Slot::Is(_) | Slot::IsRow(_) | Slot::IsBrand(_) => {
          unreachable!("{v:?} is not an effect row variable")
        }
      }
    }

    Effects::new(labels, tail)
  }

  #[must_use]
  pub fn resolve(&self, ty: &Type) -> Type {
    let mut ty = ty.clone();

    while let Type::Var(v) = ty {
      match &self.slots[v.0 as usize] {
        Slot::Is(next) => ty = next.clone(),
        Slot::Unknown { .. } => break,
        Slot::IsRow(_) | Slot::IsBrand(_) | Slot::IsEffects(_) => {
          unreachable!("{v:?} is a row or brand variable, not a type")
        }
      }
    }

    ty
  }

  #[must_use]
  pub fn zonk(&self, ty: &Type) -> Type {
    match self.resolve(ty) {
      Type::Var(v) => Type::Var(v),
      Type::Con { name, args } => {
        Type::Con { name, args: args.iter().map(|a| self.zonk(a)).collect() }
      }
      Type::Fn { params, ret, effects } => Type::Fn {
        params: params.iter().map(|p| self.zonk(p)).collect(),
        ret: Box::new(self.zonk(&ret)),
        effects: self.zonk_effects(&effects),
      },
      Type::Record(row) => Type::Record(self.zonk_row(&row)),
      Type::Gen(i) => Type::Gen(i),
      rigid @ Type::Rigid { .. } => rigid,
    }
  }

  #[must_use]
  pub fn zonk_row(&self, row: &Row) -> Row {
    let mut fields: Vec<_> =
      row.fields.iter().map(|(n, t)| (n.clone(), self.zonk(t))).collect();
    let mut tail = row.tail.clone();

    while let Tail::Open(v) = tail {
      match &self.slots[v.0 as usize] {
        Slot::IsRow(more) => {
          fields
            .extend(more.fields.iter().map(|(n, t)| (n.clone(), self.zonk(t))));
          tail = more.tail.clone();
        }
        Slot::Unknown { .. } => break,
        Slot::Is(_) | Slot::IsBrand(_) | Slot::IsEffects(_) => {
          unreachable!("{v:?} is not a row variable")
        }
      }
    }

    if let Tail::Closed(brand) = &tail {
      tail = Tail::Closed(self.resolve_brand(brand));
    }

    Row { fields, tail }
  }
}
