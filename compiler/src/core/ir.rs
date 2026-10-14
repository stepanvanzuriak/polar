use crate::{
  shared::source::Span,
  types::ty::{Effects, Type},
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Sym {
  pub id: u32,
  pub name: Arc<str>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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
  IntAdd,
  IntSub,
  IntMul,
  IntDiv,
  IntMod,
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
      Self::IntAdd => "IntAdd",
      Self::IntSub => "IntSub",
      Self::IntMul => "IntMul",
      Self::IntDiv => "IntDiv",
      Self::IntMod => "IntMod",
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
      | Self::IntAdd
      | Self::IntSub
      | Self::IntMul
      | Self::IntDiv
      | Self::IntMod
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Lit {
  Number(f64),
  String(String),
  Bool(bool),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CExpr {
  pub kind: CExprKind,
  pub origin: Option<Span>,
  #[serde(skip)]
  pub ty: Option<TypeSlot>,
  #[serde(skip)]
  pub effects: Option<EffectSlot>,
}

impl CExpr {
  pub fn for_each_child_mut(&mut self, f: &mut impl FnMut(&mut CExpr)) {
    match &mut self.kind {
      CExprKind::Lit(_)
      | CExprKind::Var(_)
      | CExprKind::Builtin { .. }
      | CExprKind::Std { .. }
      | CExprKind::User { .. }
      | CExprKind::Method { .. }
      | CExprKind::CtorFn { .. }
      | CExprKind::MatchFail
      | CExprKind::Op { .. }
      | CExprKind::Extern { .. } => {}
      CExprKind::Throw { value: one, .. }
      | CExprKind::Return { value: one }
      | CExprKind::Lam { body: one, .. }
      | CExprKind::Field { target: one, .. }
      | CExprKind::Test { target: one, .. } => f(one),
      CExprKind::Try { body: first, handler: second, .. }
      | CExprKind::Let { value: first, body: second, .. } => {
        f(first);
        f(second);
      }
      CExprKind::App { func, args } => {
        f(func);
        for arg in args {
          f(arg);
        }
      }
      CExprKind::Prim { args, .. }
      | CExprKind::Ctor { args, .. }
      | CExprKind::Dict { args, .. }
      | CExprKind::Concat { parts: args } => {
        for arg in args {
          f(arg);
        }
      }
      CExprKind::If { cond, then_branch, else_branch } => {
        f(cond);
        f(then_branch);
        f(else_branch);
      }
      CExprKind::Case { scrutinee, arms } => {
        f(scrutinee);

        for arm in arms {
          if let Some(guard) = &mut arm.guard {
            f(guard);
          }

          f(&mut arm.body);
        }
      }
      CExprKind::Record { fields } => {
        for (_, v) in fields {
          f(v);
        }
      }
      CExprKind::Update { base, fields } => {
        f(base);

        for (_, v) in fields {
          f(v);
        }
      }
    }
  }

  #[must_use]
  pub fn new(kind: CExprKind, origin: Option<Span>) -> Self {
    Self { kind, origin, ty: None, effects: None }
  }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CExprKind {
  Lit(Lit),
  Var(Sym),
  Builtin {
    module: String,
    member: String,
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CArm {
  pub pattern: CPattern,
  pub guard: Option<CExpr>,
  pub body: CExpr,
  pub origin: Option<Span>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CPattern {
  Wildcard,
  Bind(Sym),
  Lit(Lit),
  Ctor { ctor: CtorId, args: Vec<CPattern> },
  Record { fields: Vec<(String, CPattern)> },
  Or(Vec<CPattern>),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PatternTest {
  Tag(CtorId),
  Lit(Lit),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CtorInfo {
  pub name: String,
  pub arity: usize,
  pub owner: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeclKind {
  Function,
  Constant,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CDecl {
  pub sym: Sym,
  pub kind: DeclKind,
  pub exported: bool,
  pub params: Vec<Sym>,
  pub body: CExpr,
  pub origin: Option<Span>,
  #[serde(skip)]
  pub effects: Option<EffectSlot>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CImpl {
  pub trait_path: String,
  pub target: String,
  pub bounds: usize,
  pub methods: Vec<CDecl>,
  pub origin: Option<Span>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CBind {
  pub effect: String,
  pub host: String,
  pub via: Option<String>,
  pub ops: Vec<CDecl>,
  pub origin: Option<Span>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CExtern {
  pub name: String,
  pub module: String,
  pub export: String,
  pub rs: Option<String>,
  pub label: String,
  pub params: Vec<Boundary>,
  pub ret: Boundary,
  pub origin: Option<Span>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Boundary {
  #[default]
  Plain,
  Unit,
  Option(Box<Boundary>),
  List(Box<Boundary>),
  Result(Box<Boundary>, Box<Boundary>),
  Record(Vec<(String, Boundary)>),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CBridgeOp {
  pub op: String,
  pub params: Vec<Option<CExpr>>,
  pub ret: Option<CExpr>,
  pub throws: Vec<(String, CExpr)>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CBridge {
  pub effect: String,
  pub host: String,
  pub via: String,
  pub ops: Vec<CBridgeOp>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
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
  pub source: Option<(String, String)>,
}

impl CModule {
  #[must_use]
  pub fn ctor(&self, id: CtorId) -> &CtorInfo {
    &self.ctors[id.0 as usize]
  }
}
