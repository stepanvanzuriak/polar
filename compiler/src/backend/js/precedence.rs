use crate::backend::js::ast::{BinaryOp, Expr, LogicalOp};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Prec {
  Assign,
  Conditional,
  Or,
  And,
  BitOr,
  BitXor,
  BitAnd,
  Equality,
  Relational,
  Additive,
  Multiplicative,
  Unary,
  Call,
  Primary,
}

impl Prec {
  #[must_use]
  pub fn tighter(self) -> Self {
    match self {
      Self::Assign => Self::Conditional,
      Self::Conditional => Self::Or,
      Self::Or => Self::And,
      Self::And => Self::BitOr,
      Self::BitOr => Self::BitXor,
      Self::BitXor => Self::BitAnd,
      Self::BitAnd => Self::Equality,
      Self::Equality => Self::Relational,
      Self::Relational => Self::Additive,
      Self::Additive => Self::Multiplicative,
      Self::Multiplicative => Self::Unary,
      Self::Unary => Self::Call,
      Self::Call | Self::Primary => Self::Primary,
    }
  }
}

#[must_use]
pub fn binary(op: BinaryOp) -> Prec {
  match op {
    BinaryOp::Add | BinaryOp::Sub => Prec::Additive,
    BinaryOp::Mul | BinaryOp::Div | BinaryOp::Mod => Prec::Multiplicative,
    BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge => {
      Prec::Relational
    }
    BinaryOp::StrictEq | BinaryOp::StrictNe => Prec::Equality,
    BinaryOp::BitAnd => Prec::BitAnd,
    BinaryOp::BitOr => Prec::BitOr,
    BinaryOp::BitXor => Prec::BitXor,
  }
}

#[must_use]
pub fn logical(op: LogicalOp) -> Prec {
  match op {
    LogicalOp::Or => Prec::Or,
    LogicalOp::And => Prec::And,
  }
}

#[must_use]
pub fn of(e: &Expr) -> Prec {
  match e {
    Expr::Ident(_)
    | Expr::Literal(_)
    | Expr::TemplateLit(_)
    | Expr::ObjectLit(_)
    | Expr::ArrayLit(_) => Prec::Primary,
    Expr::Call(_) | Expr::Member(_) => Prec::Call,
    Expr::Unary(_) | Expr::Await(_) => Prec::Unary,
    Expr::Binary(b) => binary(b.op),
    Expr::Logical(l) => logical(l.op),
    Expr::Conditional(_) => Prec::Conditional,
    Expr::ArrowFn(_) | Expr::Assign(_) => Prec::Assign,
  }
}
