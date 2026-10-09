use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TVar(pub u32);

#[derive(Debug, Clone, PartialEq)]
pub enum Type {
  Var(TVar),
  Con { name: Arc<str>, args: Vec<Type> },
  Fn { params: Vec<Type>, ret: Box<Type>, effects: Effects },
  Record(Row),
  Gen(u32),
  Rigid { name: Arc<str>, id: TVar },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Label {
  pub name: Arc<str>,
  pub arg: Option<Arc<str>>,
}

pub const MUT: &str = "Mut";
pub const ASYNC: &str = "Async";

impl Label {
  #[must_use]
  pub fn effect(name: &str) -> Label {
    Label { name: Arc::from(name), arg: None }
  }

  #[must_use]
  pub fn throws(ty: &str) -> Label {
    Label { name: Arc::from("Throws"), arg: Some(Arc::from(ty)) }
  }

  #[must_use]
  pub fn is_throws(&self) -> bool {
    &*self.name == "Throws"
  }

  #[must_use]
  pub fn is_mut(&self) -> bool {
    &*self.name == MUT
  }

  #[must_use]
  pub fn is_async(&self) -> bool {
    &*self.name == ASYNC
  }

  #[must_use]
  pub fn is_builtin(&self) -> bool {
    self.is_free() || self.is_async()
  }

  #[must_use]
  pub fn is_free(&self) -> bool {
    self.is_throws() || self.is_mut()
  }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffTail {
  Closed,
  Open(TVar),
  Gen(u32),
  Rigid(TVar),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Effects {
  pub labels: Vec<Label>,
  pub tail: EffTail,
}

impl Effects {
  #[must_use]
  pub fn pure() -> Effects {
    Effects { labels: Vec::new(), tail: EffTail::Closed }
  }

  #[must_use]
  pub fn new(mut labels: Vec<Label>, tail: EffTail) -> Effects {
    labels.sort();
    labels.dedup();

    Effects { labels, tail }
  }

  #[must_use]
  pub fn open(var: TVar) -> Effects {
    Effects { labels: Vec::new(), tail: EffTail::Open(var) }
  }

  #[must_use]
  pub fn is_pure(&self) -> bool {
    self.labels.is_empty() && self.tail == EffTail::Closed
  }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Row {
  pub fields: Vec<(Arc<str>, Type)>,
  pub tail: Tail,
}

impl Row {
  #[must_use]
  pub fn new(fields: Vec<(Arc<str>, Type)>, tail: Tail) -> Self {
    Self { fields, tail }
  }

  #[must_use]
  pub fn brand(&self) -> Option<&Arc<str>> {
    match &self.tail {
      Tail::Closed(Brand::Named(name)) => Some(name),
      _ => None,
    }
  }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Brand {
  Named(Arc<str>),
  Var(TVar),
  Anonymous,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Tail {
  Closed(Brand),
  Open(TVar),
  Gen(u32),
  Rigid(TVar),
}

impl Tail {
  #[must_use]
  pub fn anonymous() -> Tail {
    Tail::Closed(Brand::Anonymous)
  }

  #[must_use]
  pub fn is_closed(&self) -> bool {
    matches!(self, Tail::Closed(_))
  }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Pred {
  pub trait_path: Arc<str>,
  pub ty: Type,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Scheme {
  pub count: u32,
  pub ty: Type,
  pub preds: Vec<Pred>,
}

impl Scheme {
  #[must_use]
  pub fn new(count: u32, ty: Type) -> Self {
    Self { count, ty, preds: Vec::new() }
  }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Constraint {
  None,
  Num,
  Ord,
}

impl Constraint {
  #[must_use]
  pub fn admits(self, name: &str) -> bool {
    match self {
      Constraint::None => true,
      Constraint::Num => matches!(name, "Int" | "Float"),
      Constraint::Ord => matches!(name, "Int" | "Float" | "String"),
    }
  }

  #[must_use]
  pub fn meet(self, other: Constraint) -> Constraint {
    match (self, other) {
      (Constraint::Num, _) | (_, Constraint::Num) => Constraint::Num,
      (Constraint::Ord, _) | (_, Constraint::Ord) => Constraint::Ord,
      (Constraint::None, Constraint::None) => Constraint::None,
    }
  }
}

impl Type {
  #[must_use]
  pub fn con(name: &str) -> Type {
    Type::Con { name: Arc::from(name), args: Vec::new() }
  }

  #[must_use]
  pub fn int() -> Type {
    Type::con("Int")
  }

  #[must_use]
  pub fn float() -> Type {
    Type::con("Float")
  }

  #[must_use]
  pub fn string() -> Type {
    Type::con("String")
  }

  #[must_use]
  pub fn bool() -> Type {
    Type::con("Bool")
  }

  #[must_use]
  pub fn func(params: Vec<Type>, ret: Type) -> Type {
    Type::func_with(params, ret, Effects::pure())
  }

  #[must_use]
  pub fn func_with(params: Vec<Type>, ret: Type, effects: Effects) -> Type {
    Type::Fn { params, ret: Box::new(ret), effects }
  }

  /// `{}`, the empty record. `Log.info` returns this ("unit").
  #[must_use]
  pub fn unit() -> Type {
    Type::Record(Row::new(Vec::new(), Tail::anonymous()))
  }
}
