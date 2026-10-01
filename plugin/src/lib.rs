use serde::{Deserialize, Serialize};
use std::{
  fmt::Write as _,
  panic::{AssertUnwindSafe, catch_unwind},
};

pub const ABI: u32 = 1;

#[derive(
  Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize,
)]
pub struct Span {
  pub start: usize,
  pub end: usize,
}

impl Span {
  #[must_use]
  pub fn join(self, other: Span) -> Span {
    Span { start: self.start.min(other.start), end: self.end.max(other.end) }
  }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Token {
  pub kind: String,
  pub text: String,
  pub span: Span,
  pub line: usize,
  pub column: usize,
  pub newline_before: bool,
}

impl Token {
  #[must_use]
  pub fn is(&self, kind: &str) -> bool {
    self.kind == kind
  }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
  pub span: Span,
  pub source: String,
  pub tokens: Vec<Token>,
  #[serde(default)]
  pub comments: Vec<Token>,
}

impl Entry {
  #[must_use]
  pub fn lines(&self) -> Vec<Vec<&Token>> {
    let mut lines: Vec<Vec<&Token>> = Vec::new();

    for token in &self.tokens {
      match lines.last_mut() {
        Some(line) if !token.newline_before => line.push(token),
        _ => lines.push(vec![token]),
      }
    }

    lines
  }

  #[must_use]
  pub fn text(&self, span: Span) -> &str {
    let from = span.start.saturating_sub(self.span.start);
    let to = span.end.saturating_sub(self.span.start);

    self.source.get(from..to).unwrap_or_default()
  }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Zone {
  pub keyword: String,
  pub after: String,
  pub blank_between_entries: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Import {
  pub path: Vec<String>,
  pub alias: Option<String>,
  pub span: Span,
}

impl Import {
  #[must_use]
  pub fn local(&self) -> &str {
    self
      .alias
      .as_deref()
      .or(self.path.last().map(String::as_str))
      .unwrap_or_default()
  }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Param {
  pub name: String,
  pub span: Span,
  pub ty: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Function {
  pub name: String,
  pub span: Span,
  pub params: Vec<Param>,
  pub ret: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Module {
  pub name: Option<String>,
  pub imports: Vec<Import>,
  pub functions: Vec<Function>,
  pub types: Vec<String>,
}

impl Module {
  #[must_use]
  pub fn import(&self, path: &[&str]) -> Option<&Import> {
    self
      .imports
      .iter()
      .find(|i| i.path.iter().map(String::as_str).eq(path.iter().copied()))
  }

  #[must_use]
  pub fn function(&self, name: &str) -> Option<&Function> {
    self.functions.iter().find(|f| f.name == name)
  }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Origin {
  pub start: usize,
  pub end: usize,
  pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Generated {
  pub zone: String,
  pub source: String,
  pub origin: Span,
  pub origins: Vec<Origin>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Label {
  pub span: Span,
  pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
  pub message: String,
  pub span: Span,
  pub label: Option<String>,
  pub secondary: Vec<Label>,
  pub help: Option<String>,
  pub notes: Vec<String>,
}

impl Diagnostic {
  #[must_use]
  pub fn error(message: impl Into<String>, span: Span) -> Self {
    Self {
      message: message.into(),
      span,
      label: None,
      secondary: Vec::new(),
      help: None,
      notes: Vec::new(),
    }
  }

  #[must_use]
  pub fn with_label(mut self, label: impl Into<String>) -> Self {
    self.label = Some(label.into());
    self
  }

  #[must_use]
  pub fn with_secondary(
    mut self,
    span: Span,
    message: impl Into<String>,
  ) -> Self {
    self.secondary.push(Label { span, message: message.into() });
    self
  }

  #[must_use]
  pub fn with_help(mut self, help: impl Into<String>) -> Self {
    self.help = Some(help.into());
    self
  }

  #[must_use]
  pub fn with_note(mut self, note: impl Into<String>) -> Self {
    self.notes.push(note.into());
    self
  }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Expansion {
  pub generated: Vec<Generated>,
  pub diagnostics: Vec<Diagnostic>,
}

impl Expansion {
  pub fn error(&mut self, diagnostic: Diagnostic) {
    self.diagnostics.push(diagnostic);
  }

  pub fn emit(&mut self, generated: Generated) {
    self.generated.push(generated);
  }

  #[must_use]
  pub fn has_errors(&self) -> bool {
    !self.diagnostics.is_empty()
  }
}

#[derive(Debug, Clone, Default)]
pub struct Source {
  text: String,
  origins: Vec<Origin>,
}

impl Source {
  #[must_use]
  pub fn new() -> Self {
    Self::default()
  }

  pub fn push(&mut self, text: &str) -> &mut Self {
    self.text.push_str(text);
    self
  }

  pub fn from(&mut self, span: Span, text: &str) -> &mut Self {
    let start = self.text.len();

    self.text.push_str(text);
    self.origins.push(Origin { start, end: self.text.len(), span });
    self
  }

  #[must_use]
  pub fn finish(self, zone: &str, origin: Span) -> Generated {
    Generated {
      zone: zone.to_string(),
      source: self.text,
      origin,
      origins: self.origins,
    }
  }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "request")]
pub enum Request {
  Zones,
  Expand { keyword: String, zone: Span, entries: Vec<Entry>, module: Module },
  Print { keyword: String, entries: Vec<Entry> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "response", content = "value")]
pub enum Response {
  Zones(Vec<Zone>),
  Expanded(Expansion),
  Printed(Vec<Vec<String>>),
  Failed(String),
}

pub trait ZonePlugin: Sync {
  fn zone(&self) -> Zone;

  fn expand(&self, zone: Span, entries: &[Entry], module: &Module)
  -> Expansion;

  fn print(&self, entries: &[Entry]) -> Vec<Vec<String>> {
    entries.iter().map(reindent).collect()
  }
}

#[must_use]
pub fn reindent(entry: &Entry) -> Vec<String> {
  let raw: Vec<&str> = entry
    .source
    .lines()
    .map(str::trim_end)
    .filter(|l| !l.trim().is_empty())
    .collect();
  let width = |l: &str| l.len() - l.trim_start().len();
  let mut widths: Vec<usize> = raw.iter().skip(1).map(|l| width(l)).collect();

  widths.sort_unstable();
  widths.dedup();

  raw
    .iter()
    .enumerate()
    .map(|(i, line)| {
      let level = if i == 0 {
        0
      } else {
        1 + widths.iter().position(|w| *w == width(line)).unwrap_or(0)
      };

      format!("{}{}", "  ".repeat(level), line.trim_start())
    })
    .collect()
}

#[must_use]
pub fn align(rows: &[Vec<String>]) -> Vec<String> {
  let columns = rows.iter().map(Vec::len).max().unwrap_or(0);
  let widths: Vec<usize> = (0..columns)
    .map(|c| {
      rows
        .iter()
        .filter(|r| r.len() > c + 1)
        .map(|r| r[c].len())
        .max()
        .unwrap_or(0)
    })
    .collect();

  rows
    .iter()
    .map(|row| {
      let mut line = String::new();

      for (c, cell) in row.iter().enumerate() {
        if c + 1 < row.len() {
          let _ = write!(line, "{cell:w$}  ", w = widths[c]);
        } else {
          line.push_str(cell);
        }
      }

      line
    })
    .collect()
}

#[must_use]
pub fn dispatch(plugins: &[&dyn ZonePlugin], request: &[u8]) -> Vec<u8> {
  let response = catch_unwind(AssertUnwindSafe(|| answer(plugins, request)))
    .unwrap_or_else(|payload| {
      let message = payload
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| payload.downcast_ref::<&str>().map(|s| (*s).to_string()))
        .unwrap_or_else(|| "a panic with no message".to_string());

      Response::Failed(format!("the plugin panicked: {message}"))
    });

  serde_json::to_vec(&response).unwrap_or_default()
}

fn answer(plugins: &[&dyn ZonePlugin], request: &[u8]) -> Response {
  let request: Request = match serde_json::from_slice(request) {
    Ok(request) => request,
    Err(e) => return Response::Failed(format!("a malformed request: {e}")),
  };
  let find =
    |keyword: &str| plugins.iter().find(|p| p.zone().keyword == keyword);

  match request {
    Request::Zones => {
      Response::Zones(plugins.iter().map(|p| p.zone()).collect())
    }
    Request::Expand { keyword, zone, entries, module } => {
      match find(&keyword) {
        Some(plugin) => {
          Response::Expanded(plugin.expand(zone, &entries, &module))
        }
        None => Response::Failed(format!("no zone `{keyword}` in this plugin")),
      }
    }
    Request::Print { keyword, entries } => match find(&keyword) {
      Some(plugin) => Response::Printed(plugin.print(&entries)),
      None => Response::Failed(format!("no zone `{keyword}` in this plugin")),
    },
  }
}

#[macro_export]
macro_rules! export {
  ($($plugin:expr),+ $(,)?) => {
    #[allow(unsafe_code)]
    #[unsafe(no_mangle)]
    pub extern "C" fn polar_plugin_abi() -> u32 {
      $crate::ABI
    }

    #[allow(unsafe_code, clippy::missing_safety_doc)]
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn polar_plugin_call(
      input: *const u8,
      len: usize,
      out_len: *mut usize,
    ) -> *mut u8 {
      let request = unsafe { ::std::slice::from_raw_parts(input, len) };
      let plugins: &[&dyn $crate::ZonePlugin] = &[$(&$plugin),+];
      let reply = $crate::dispatch(plugins, request).into_boxed_slice();

      unsafe { *out_len = reply.len() };
      ::std::boxed::Box::into_raw(reply).cast::<u8>()
    }

    #[allow(unsafe_code, clippy::missing_safety_doc)]
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn polar_plugin_free(ptr: *mut u8, len: usize) {
      drop(unsafe {
        ::std::boxed::Box::from_raw(::std::ptr::slice_from_raw_parts_mut(ptr, len))
      });
    }
  };
}
