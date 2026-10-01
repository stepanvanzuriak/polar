use crate::syntax::lexer::token::TokenKind::{
  self, Amp, AndAnd, BangEq, Bar, Caret, Dot, EqEq, Ge, Gt, LParen, Le, Lt,
  Minus, OrOr, Percent, PipeOp, Plus, Slash, Star,
};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Assoc {
  Left,
  Right,
  None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InfixEntry {
  pub bp: u8,
  pub assoc: Assoc,
}

impl InfixEntry {
  #[must_use]
  pub fn right_bp(self) -> u8 {
    match self.assoc {
      Assoc::Right => self.bp,
      Assoc::Left | Assoc::None => self.bp + 1,
    }
  }
}

#[derive(Debug, Clone)]
pub struct PrecedenceTable {
  entries: HashMap<TokenKind, InfixEntry>,
}

impl PrecedenceTable {
  #[must_use]
  pub fn infix(&self, kind: TokenKind) -> Option<InfixEntry> {
    self.entries.get(&kind).copied()
  }

  #[must_use]
  pub fn with(mut self, kind: TokenKind, entry: InfixEntry) -> Self {
    self.entries.insert(kind, entry);
    self
  }
}

pub const PREFIX_BINDING_POWER: u8 = 70;

impl Default for PrecedenceTable {
  fn default() -> Self {
    let levels: &[(u8, Assoc, &[TokenKind])] = &[
      (10, Assoc::Left, &[PipeOp]),
      (20, Assoc::Right, &[OrOr]),
      (30, Assoc::Right, &[AndAnd]),
      (40, Assoc::None, &[EqEq, BangEq, Lt, Le, Gt, Ge]),
      (42, Assoc::Left, &[Bar]),
      (44, Assoc::Left, &[Caret]),
      (46, Assoc::Left, &[Amp]),
      (50, Assoc::Left, &[Plus, Minus]),
      (60, Assoc::Left, &[Star, Slash, Percent]),
      (80, Assoc::Left, &[Dot, LParen]),
    ];

    let entries = levels
      .iter()
      .flat_map(|&(bp, assoc, kinds)| {
        kinds.iter().map(move |&kind| (kind, InfixEntry { bp, assoc }))
      })
      .collect();

    Self { entries }
  }
}
