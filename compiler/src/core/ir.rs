use crate::{
  shared::source::Span,
  types::ty::{Effects, Type},
};
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Sym {
  pub id: u32,
  pub name: Arc<str>,
}

#[derive(Debug, Default)]
pub struct SymGen {
  next: u32,
}

impl SymGen {
  #[must_use]
  pub fn starting_at(next: u32) -> Self {
    Self { next }
  }

  pub fn fresh(&mut self, name: &str) -> Sym {
    let id = self.next;

    self.next += 1;

    Sym { id, name: Arc::from(name) }
  }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrimOp {
  Add,
  Sub,
  Mul,
  Div,
  Mod,
  Neg,
  Lt,
  Le,
  Gt,
  Ge,
  Eq,
  Ne,
  And,
  Or,
  Not,
  BitAnd,
  BitOr,
  BitXor,
  BitNot,
}

impl PrimOp {
  #[must_use]
  pub fn as_str(self) -> &'static str {
    match self {
      Self::Add => "Add",
      Self::Sub => "Sub",
      Self::Mul => "Mul",
      Self::Div => "Div",
      Self::Mod => "Mod",
      Self::Neg => "Neg",
      Self::Lt => "Lt",
      Self::Le => "Le",
      Self::Gt => "Gt",
      Self::Ge => "Ge",
      Self::Eq => "Eq",
      Self::Ne => "Ne",
      Self::And => "And",
      Self::Or => "Or",
      Self::Not => "Not",
      Self::BitAnd => "BitAnd",
      Self::BitOr => "BitOr",
      Self::BitXor => "BitXor",
      Self::BitNot => "BitNot",
    }
  }

  #[must_use]
  pub fn arity(self) -> usize {
    match self {
      Self::Neg | Self::Not | Self::BitNot => 1,
      Self::Add
      | Self::Sub
      | Self::Mul
      | Self::Div
      | Self::Mod
      | Self::BitAnd
      | Self::BitOr
      | Self::BitXor
      | Self::Lt
      | Self::Le
      | Self::Gt
      | Self::Ge
      | Self::Eq
      | Self::Ne
      | Self::And
      | Self::Or => 2,
    }
  }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Lit {
  Number(f64),
  String(String),
  Bool(bool),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CtorId(pub u32);

#[derive(Debug, Clone, PartialEq)]
pub struct TypeSlot(pub Type);

#[derive(Debug, Clone, PartialEq)]
pub struct EffectSlot(pub Effects);

impl TypeSlot {
  #[must_use]
  pub fn row(&self) -> Option<EffectSlot> {
    match &self.0 {
      Type::Fn { effects, .. } => Some(EffectSlot(effects.clone())),
      _ => None,
    }
  }
}

impl EffectSlot {
  #[must_use]
  pub fn pure() -> Self {
    Self(Effects::pure())
  }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CExpr {
  pub kind: CExprKind,
  pub origin: Option<Span>,
  pub ty: Option<TypeSlot>,
  pub effects: Option<EffectSlot>,
}

impl CExpr {
  #[must_use]
  pub fn new(kind: CExprKind, origin: Option<Span>) -> Self {
    Self { kind, origin, ty: None, effects: None }
  }
}

#[derive(Debug, Clone, PartialEq)]
pub enum CExprKind {
  Lit(Lit),
  Var(Sym),
  Builtin {
    module: &'static str,
    member: &'static str,
  },
  Std {
    module: String,
    member: String,
  },
  User {
    module: String,
    member: String,
  },
  Method {
    trait_path: String,
    method: String,
  },
  Dict {
    trait_path: String,
    target: String,
    module: Option<String>,
    args: Vec<CExpr>,
  },
  Lam {
    params: Vec<Sym>,
    body: Box<CExpr>,
  },
  App {
    func: Box<CExpr>,
    args: Vec<CExpr>,
  },
  Prim {
    op: PrimOp,
    args: Vec<CExpr>,
  },
  Let {
    sym: Option<Sym>,
    value: Box<CExpr>,
    body: Box<CExpr>,
  },
  If {
    cond: Box<CExpr>,
    then_branch: Box<CExpr>,
    else_branch: Box<CExpr>,
  },
  Case {
    scrutinee: Box<CExpr>,
    arms: Vec<CArm>,
  },
  Record {
    fields: Vec<(String, CExpr)>,
  },
  Update {
    base: Box<CExpr>,
    fields: Vec<(String, CExpr)>,
  },
  Field {
    target: Box<CExpr>,
    name: String,
  },
  Ctor {
    ctor: CtorId,
    args: Vec<CExpr>,
  },
  CtorFn {
    ctor: CtorId,
  },
  Concat {
    parts: Vec<CExpr>,
  },
  Test {
    test: PatternTest,
    target: Box<CExpr>,
  },
  MatchFail,
  Op {
    effect: String,
    op: String,
  },
  Extern {
    name: String,
  },
  Throw {
    value: Box<CExpr>,
    tag: String,
  },
  Return {
    value: Box<CExpr>,
  },
  Try {
    body: Box<CExpr>,
    caught: Sym,
    handler: Box<CExpr>,
    handles: Vec<String>,
  },
}

#[derive(Debug, Clone, PartialEq)]
pub struct CArm {
  pub pattern: CPattern,
  pub guard: Option<CExpr>,
  pub body: CExpr,
  pub origin: Option<Span>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CPattern {
  Wildcard,
  Bind(Sym),
  Lit(Lit),
  Ctor { ctor: CtorId, args: Vec<CPattern> },
  Record { fields: Vec<(String, CPattern)> },
  Or(Vec<CPattern>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum PatternTest {
  Tag(CtorId),
  Lit(Lit),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CtorInfo {
  pub name: String,
  pub arity: usize,
  pub owner: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeclKind {
  Function,
  Constant,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CDecl {
  pub sym: Sym,
  pub kind: DeclKind,
  pub exported: bool,
  pub params: Vec<Sym>,
  pub body: CExpr,
  pub origin: Option<Span>,
  pub effects: Option<EffectSlot>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CImpl {
  pub trait_path: String,
  pub target: String,
  pub bounds: usize,
  pub methods: Vec<CDecl>,
  pub origin: Option<Span>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CBind {
  pub effect: String,
  pub host: String,
  pub via: Option<String>,
  pub ops: Vec<CDecl>,
  pub origin: Option<Span>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CExtern {
  pub name: String,
  pub module: String,
  pub export: String,
  pub label: String,
  pub params: Vec<Boundary>,
  pub ret: Boundary,
  pub origin: Option<Span>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Boundary {
  #[default]
  Plain,
  Unit,
  Option(Box<Boundary>),
  List(Box<Boundary>),
  Result(Box<Boundary>, Box<Boundary>),
  Record(Vec<(String, Boundary)>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct CBridgeOp {
  pub op: String,
  pub params: Vec<Option<CExpr>>,
  pub ret: Option<CExpr>,
  pub throws: Vec<(String, CExpr)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CBridge {
  pub effect: String,
  pub host: String,
  pub via: String,
  pub ops: Vec<CBridgeOp>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct CModule {
  pub decls: Vec<CDecl>,
  pub impls: Vec<CImpl>,
  pub binds: Vec<CBind>,
  pub bridges: Vec<CBridge>,
  pub host: Option<String>,
  pub externs: Vec<CExtern>,
  pub bind_refs: Vec<(String, Option<String>)>,
  pub ctors: Vec<CtorInfo>,
  pub std_imports: Vec<String>,
  pub user_imports: Vec<(String, String)>,
}

impl CModule {
  #[must_use]
  pub fn ctor(&self, id: CtorId) -> &CtorInfo {
    &self.ctors[id.0 as usize]
  }
}
