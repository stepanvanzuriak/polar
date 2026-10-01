use crate::{
  shared::codes::DiagnosticCode,
  shared::source::{SourceFile, Span},
};
use std::fmt::Write;
use std::{collections::HashSet, sync::Arc};

type DedupKey = (Severity, DiagnosticCode, Arc<str>, usize, usize, String);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Severity {
  Error,
  Warning,
  Info,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Label {
  pub span: Span,
  pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
  pub severity: Severity,
  pub code: DiagnosticCode,
  pub message: String,
  pub primary: Label,
  pub secondary: Vec<Label>,
  pub notes: Vec<String>,
  pub help: Option<String>,
}

#[derive(Debug, Default)]
pub struct DiagnosticBag {
  diagnostics: Vec<Diagnostic>,
  seen: HashSet<DedupKey>,
}

impl Label {
  #[must_use]
  pub fn new(span: Span) -> Self {
    Label { span, message: None }
  }

  #[must_use]
  pub fn with_message(self, message: impl Into<String>) -> Self {
    Label { span: self.span, message: Some(message.into()) }
  }
}

impl Diagnostic {
  pub fn error(
    code: DiagnosticCode,
    message: impl Into<String>,
    primary: Label,
  ) -> Self {
    Self {
      severity: Severity::Error,
      code,
      message: message.into(),
      primary,
      secondary: Vec::new(),
      notes: Vec::new(),
      help: None,
    }
  }

  pub fn warning(
    code: DiagnosticCode,
    message: impl Into<String>,
    primary: Label,
  ) -> Self {
    Self {
      severity: Severity::Warning,
      code,
      message: message.into(),
      primary,
      secondary: Vec::new(),
      notes: Vec::new(),
      help: None,
    }
  }

  pub fn info(
    code: DiagnosticCode,
    message: impl Into<String>,
    primary: Label,
  ) -> Self {
    Self {
      severity: Severity::Info,
      code,
      message: message.into(),
      primary,
      secondary: Vec::new(),
      notes: Vec::new(),
      help: None,
    }
  }

  #[must_use]
  pub fn with_secondary(mut self, label: Label) -> Self {
    self.secondary.push(label);

    self
  }

  #[must_use]
  pub fn with_note(mut self, note: impl Into<String>) -> Self {
    self.notes.push(note.into());

    self
  }

  #[must_use]
  pub fn with_help(mut self, help: impl Into<String>) -> Self {
    self.help = Some(help.into());

    self
  }

  fn dedup_key(&self) -> DedupKey {
    (
      self.severity,
      self.code,
      self.primary.span.file.clone(),
      self.primary.span.start,
      self.primary.span.end,
      self.message.clone(),
    )
  }
}

impl DiagnosticBag {
  pub fn push(&mut self, d: Diagnostic) {
    if self.seen.insert(d.dedup_key()) {
      self.diagnostics.push(d);
    }
  }

  #[must_use]
  pub fn has_errors(&self) -> bool {
    self.diagnostics.iter().any(|d| d.severity == Severity::Error)
  }

  #[must_use]
  pub fn error_count(&self) -> usize {
    self.diagnostics.iter().filter(|d| d.severity == Severity::Error).count()
  }

  #[must_use]
  pub fn warning_count(&self) -> usize {
    self.diagnostics.iter().filter(|d| d.severity == Severity::Warning).count()
  }

  pub fn map_spans(&mut self, f: impl Fn(&Span) -> Span) {
    for d in &mut self.diagnostics {
      d.primary.span = f(&d.primary.span);

      for label in &mut d.secondary {
        label.span = f(&label.span);
      }
    }

    self.seen.clear();

    let diagnostics = std::mem::take(&mut self.diagnostics);

    for d in diagnostics {
      self.push(d);
    }
  }

  #[must_use]
  pub fn into_sorted(mut self) -> Vec<Diagnostic> {
    self.diagnostics.sort_by(|a, b| {
      a.primary
        .span
        .file
        .as_ref()
        .cmp(b.primary.span.file.as_ref())
        .then(a.primary.span.start.cmp(&b.primary.span.start))
        .then(a.primary.span.end.cmp(&b.primary.span.end))
    });

    self.diagnostics
  }
}

pub fn dump_diagnostics(
  file: &SourceFile,
  diagnostics: &[Diagnostic],
) -> String {
  let mut out = String::new();

  for d in diagnostics {
    let severity = match d.severity {
      Severity::Error => "error",
      Severity::Warning => "warning",
      Severity::Info => "info",
    };

    let start = file.position_at(d.primary.span.start);
    let end = file.position_at(d.primary.span.end);
    let start_col = file.code_point_column(start);
    let end_col = file.code_point_column(end);

    let _ = writeln!(
      out,
      "{severity} {} {}:{start_col}-{}:{end_col} {}",
      d.code,
      start.line + 1,
      end.line + 1,
      d.message,
    );
  }

  out
}
