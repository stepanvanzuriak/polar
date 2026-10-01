use crate::{
  shared::diagnostic::DiagnosticBag,
  shared::source::SourceFile,
  syntax::lexer::{
    cursor::{Cursor, Frame, recover, scan_code_token, scan_string_piece},
    token::{Comment, Token, TokenKind::Eof},
  },
};

pub mod cursor;
pub mod dump;
pub mod keywords;
pub mod token;

pub struct LexResult {
  pub tokens: Vec<Token>,
  pub comments: Vec<Comment>,
}

pub fn lex(file: &SourceFile, diagnostics: &mut DiagnosticBag) -> LexResult {
  let mut cursor = Cursor::new(file);
  let mut stack: Vec<Frame> = vec![];

  loop {
    if matches!(stack.last(), Some(Frame::Str { .. })) {
      scan_string_piece(&mut cursor, &mut stack, diagnostics);
    } else if cursor.at_eof() {
      if stack.is_empty() {
        cursor.push(Eof, cursor.pos);
        break;
      }

      recover(&mut cursor, &mut stack, diagnostics);
    } else {
      scan_code_token(&mut cursor, &mut stack, diagnostics);
    }
  }

  let (tokens, comments) = cursor.into_parts();

  LexResult { tokens, comments }
}
