use crate::{shared::source::SourceFile, syntax::lexer::token::Token};
use std::fmt::Write as _;

pub fn dump_tokens(file: &SourceFile, tokens: &[Token]) -> String {
  let mut out = String::new();

  for token in tokens {
    let pos = file.position_at(token.span.start);
    let col = file.code_point_column(pos);
    let marker = if token.newline_before { '↵' } else { ' ' };
    let kind = format!("{:?}", token.kind);
    let text = file.slice(&token.span);

    let _ = write!(out, "{}:{} {marker} {kind:<12}{text:?}", pos.line + 1, col);

    if let Some(value) = &token.value {
      if value != text {
        let _ = write!(out, " value={value:?}");
      }
    }

    out.push('\n');
  }

  out
}
