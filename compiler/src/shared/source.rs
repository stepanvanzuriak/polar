use crate::shared::ice::{ice, invariant};
use std::{
  cmp,
  sync::{Arc, OnceLock},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
  pub file: Arc<str>,
  pub start: usize,
  pub end: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Position {
  pub line: usize,
  pub column: usize,
}

#[derive(Debug, Clone)]
pub struct SourceFile {
  name: String,
  text: String,
  line_starts: OnceLock<Vec<usize>>,
}

impl Span {
  #[must_use]
  pub fn new(file: Arc<str>, start: usize, end: usize) -> Self {
    Self { file, start, end }
  }

  #[must_use]
  pub fn empty(file: Arc<str>, at: usize) -> Self {
    Self { file, start: at, end: at }
  }

  /// The smallest span covering both `self` and `other`.
  ///
  /// # Panics
  ///
  /// Raises an internal compiler error if the spans are in different files.
  #[must_use]
  pub fn join(&self, other: &Span) -> Span {
    invariant(self.file == other.file, || {
      format!("cannot join spans from {} and {}", self.file, other.file)
    });

    Span {
      file: self.file.clone(),
      start: cmp::min(self.start, other.start),
      end: cmp::max(self.end, other.end),
    }
  }
}

impl SourceFile {
  pub fn new(name: impl Into<String>, text: impl Into<String>) -> Self {
    Self { name: name.into(), text: text.into(), line_starts: OnceLock::new() }
  }

  #[must_use]
  pub fn name(&self) -> &str {
    &self.name
  }

  #[must_use]
  pub fn text(&self) -> &str {
    &self.text
  }

  #[must_use]
  pub fn line_count(&self) -> usize {
    self.lines().len()
  }

  #[must_use]
  pub fn position_at(&self, offset: usize) -> Position {
    invariant(self.text.is_char_boundary(offset), || {
      format!("offset {offset} out of range or not on a char boundary")
    });

    let starts = self.line_starts();
    let line = starts.partition_point(|&start| start <= offset) - 1;

    Position { line, column: offset - starts[line] }
  }

  #[must_use]
  pub fn offset_at(&self, position: Position) -> usize {
    self.line_start(position.line) + position.column
  }

  #[must_use]
  pub fn code_point_column(&self, position: Position) -> usize {
    self.line_text(position.line)[..position.column].chars().count() + 1
  }

  pub fn line_text(&self, line: usize) -> &str {
    match self.lines().get(line) {
      Some(&line) => line,
      None => ice(format!("line {line} out of range"), None),
    }
  }

  pub fn slice(&self, span: &Span) -> &str {
    match self.text.get(span.start..span.end) {
      Some(text) => text,
      None => ice(String::from("span out of range"), Some(span)),
    }
  }

  pub fn line_start(&self, line: usize) -> usize {
    match self.line_starts().get(line) {
      Some(&start) => start,
      None => ice(format!("line {line} out of range"), None),
    }
  }

  fn line_starts(&self) -> &[usize] {
    self.line_starts.get_or_init(|| {
      let bytes = self.text.as_bytes();
      let mut starts = vec![0];
      let mut i = 0;

      while i < bytes.len() {
        match bytes[i] {
          b'\n' => starts.push(i + 1),
          b'\r' => {
            if bytes.get(i + 1) == Some(&b'\n') {
              i += 1;
            }
            starts.push(i + 1);
          }
          _ => {}
        }

        i += 1;
      }

      starts
    })
  }

  fn lines(&self) -> Vec<&str> {
    let starts = self.line_starts();

    starts
      .iter()
      .enumerate()
      .map(|(i, &start)| {
        let end = starts.get(i + 1).map_or(self.text.len(), |&next| next);
        let line = &self.text[start..end];
        let line = line.strip_suffix('\n').unwrap_or(line);

        line.strip_suffix('\r').unwrap_or(line)
      })
      .collect()
  }
}
