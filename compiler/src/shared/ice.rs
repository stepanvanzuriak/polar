use std::{backtrace::Backtrace, fmt, panic::Location};

use crate::shared::source::Span;

#[derive(Debug)]
pub struct InternalCompilerError {
  pub message: String,
  pub span: Option<Span>,
  pub location: &'static Location<'static>,
  pub backtrace: Backtrace,
}

impl fmt::Display for InternalCompilerError {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    write!(f, "internal compiler error: {}", self.message)?;

    if let Some(span) = &self.span {
      write!(f, " at {}@{}..{}", span.file, span.start, span.end)?;
    }

    writeln!(f)?;
    writeln!(f, "at {}", self.location)?;
    write!(f, "this is a bug in the Polar compiler, please report it")
  }
}

#[track_caller]
pub fn ice(message: impl Into<String>, span: Option<&Span>) -> ! {
  std::panic::panic_any(InternalCompilerError {
    message: message.into(),
    span: span.cloned(),
    location: Location::caller(),
    backtrace: Backtrace::force_capture(),
  })
}

#[track_caller]
pub fn invariant(cond: bool, message: impl FnOnce() -> String) {
  if !cond {
    ice(message(), None);
  }
}
