use crate::{
  syntax::ast::{BinaryOp, Expr, UnaryOp},
  syntax::lexer::token::TokenKind,
  syntax::parser::precedence::{
    Assoc, InfixEntry, PREFIX_BINDING_POWER, PrecedenceTable,
  },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
  Left,
  Right,
}

#[must_use]
pub fn binary_token(op: BinaryOp) -> TokenKind {
  match op {
    BinaryOp::Or => TokenKind::OrOr,
    BinaryOp::And => TokenKind::AndAnd,
    BinaryOp::Eq => TokenKind::EqEq,
    BinaryOp::NotEq => TokenKind::BangEq,
    BinaryOp::Lt => TokenKind::Lt,
    BinaryOp::LtEq => TokenKind::Le,
    BinaryOp::Gt => TokenKind::Gt,
    BinaryOp::GtEq => TokenKind::Ge,
    BinaryOp::Add => TokenKind::Plus,
    BinaryOp::Sub => TokenKind::Minus,
    BinaryOp::Mul => TokenKind::Star,
    BinaryOp::Div => TokenKind::Slash,
    BinaryOp::Rem => TokenKind::Percent,
    BinaryOp::BitAnd => TokenKind::Amp,
    BinaryOp::BitOr => TokenKind::Bar,
    BinaryOp::BitXor => TokenKind::Caret,
  }
}

#[must_use]
pub fn entry(table: &PrecedenceTable, kind: TokenKind) -> InfixEntry {
  table.infix(kind).unwrap_or_else(|| {
    crate::shared::ice::ice(
      format!("{} is not in the precedence table", kind.display_name()),
      None,
    )
  })
}

fn binding_power(expr: &Expr, table: &PrecedenceTable) -> Option<u8> {
  match expr {
    Expr::Binary(b) => Some(entry(table, binary_token(b.op)).bp),
    Expr::Pipe(_) => Some(entry(table, TokenKind::PipeOp).bp),
    Expr::Unary(_) => Some(PREFIX_BINDING_POWER),
    _ => None,
  }
}

fn is_open_ended(expr: &Expr) -> bool {
  matches!(
    expr,
    Expr::If(_)
      | Expr::Match(_)
      | Expr::Lambda(_)
      | Expr::Throw(_)
      | Expr::Try(_)
  )
}

#[must_use]
pub fn operand_needs_parens(
  parent: InfixEntry,
  child: &Expr,
  side: Side,
  table: &PrecedenceTable,
) -> bool {
  if is_open_ended(child) {
    return true;
  }

  let Some(bp) = binding_power(child, table) else { return false };

  if bp != parent.bp {
    return bp < parent.bp;
  }

  match parent.assoc {
    Assoc::Left => side == Side::Right,
    Assoc::Right => side == Side::Left,
    Assoc::None => true,
  }
}

#[must_use]
pub fn postfix_target_needs_parens(child: &Expr) -> bool {
  matches!(
    child,
    Expr::Pipe(_) | Expr::Binary(_) | Expr::Unary(_) | Expr::Block(_)
  ) || is_open_ended(child)
}

#[must_use]
pub fn unary_operand_needs_parens(
  child: &Expr,
  table: &PrecedenceTable,
) -> bool {
  if matches!(child, Expr::Unary(_)) || is_open_ended(child) {
    return true;
  }

  binding_power(child, table).is_some_and(|bp| bp < PREFIX_BINDING_POWER)
}

#[must_use]
pub fn exposes_record(expr: &Expr) -> bool {
  let mut work = vec![expr];

  while let Some(expr) = work.pop() {
    match expr {
      Expr::Record(_) => return true,
      Expr::Binary(b) => work.extend([&*b.left, &*b.right]),
      Expr::Pipe(p) => work.extend([&*p.left, &*p.right]),
      Expr::Unary(u) => work.push(&u.operand),
      Expr::Throw(t) => work.push(&t.value),
      Expr::Field(f) => work.push(&f.target),
      Expr::Call(c) => work.push(&c.callee),
      _ => {}
    }
  }

  false
}

#[must_use]
pub fn starts_with_minus(expr: &Expr) -> bool {
  let mut expr = expr;

  loop {
    expr = match expr {
      Expr::Unary(u) => return u.op == UnaryOp::Negate,
      Expr::Binary(b) => &b.left,
      Expr::Pipe(p) => &p.left,
      Expr::Call(c) => &c.callee,
      Expr::Field(f) => &f.target,
      _ => return false,
    };
  }
}
