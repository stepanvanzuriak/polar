use crate::{
  shared::diagnostic::{Diagnostic, Severity},
  shared::source::{SourceFile, Span},
};
use std::{
  collections::{BTreeSet, HashMap},
  fmt::Write as _,
  hash::BuildHasher,
  sync::Arc,
};

#[derive(Debug, Clone, Copy)]
pub struct RenderOptions {
  pub color: bool,
}

struct Style {
  color: bool,
}

impl Style {
  fn wrap(&self, code: &str, text: &str) -> String {
    if self.color {
      format!("\u{1b}[{code}m{text}\u{1b}[0m")
    } else {
      text.to_string()
    }
  }

  fn severity(&self, severity: Severity, text: &str) -> String {
    let code = match severity {
      Severity::Error => "31",
      Severity::Warning => "33",
      Severity::Info => "34",
    };

    self.wrap(code, text)
  }

  fn bold(&self, text: &str) -> String {
    self.wrap("1", text)
  }
}

fn pad(n: usize) -> String {
  " ".repeat(n)
}

fn digits(n: usize) -> usize {
  n.to_string().len()
}

fn clamp_boundary(text: &str, mut idx: usize) -> usize {
  if idx > text.len() {
    idx = text.len();
  }

  while idx > 0 && !text.is_char_boundary(idx) {
    idx -= 1;
  }

  idx
}

fn code_point_col(line_text: &str, byte_col: usize) -> usize {
  let byte_col = clamp_boundary(line_text, byte_col);

  line_text[..byte_col].chars().count() + 1
}

fn render_col(line_text: &str, byte_col: usize) -> usize {
  let byte_col = clamp_boundary(line_text, byte_col);

  line_text[..byte_col].chars().map(|c| if c == '\t' { 4 } else { 1 }).sum()
}

fn render_width(line_text: &str) -> usize {
  render_col(line_text, line_text.len())
}

fn first_non_ws_col(line_text: &str) -> usize {
  let byte_idx = line_text.find(|c: char| !c.is_whitespace()).unwrap_or(0);

  render_col(line_text, byte_idx)
}

fn display_line(text: &str) -> String {
  text.replace('\r', "").replace('\t', "    ")
}

struct Resolved<'a> {
  is_primary: bool,
  true_line: usize,
  true_col: usize,
  context_lines: Vec<usize>,
  underlines: Vec<(usize, usize, usize, Option<&'a str>)>,
}

fn resolve_label<'a>(
  file: &SourceFile,
  span: &Span,
  is_primary: bool,
  message: Option<&'a str>,
) -> Resolved<'a> {
  let true_pos = file.position_at(span.start);
  let true_col = code_point_col(file.line_text(true_pos.line), true_pos.column);

  if span.start == span.end {
    let line_text = file.line_text(true_pos.line);

    let (line, col) = if true_pos.line > 0
      && true_pos.line == file.line_count() - 1
      && line_text.is_empty()
    {
      let prev = true_pos.line - 1;

      (prev, render_width(file.line_text(prev)))
    } else {
      (true_pos.line, render_col(line_text, true_pos.column))
    };

    return Resolved {
      is_primary,
      true_line: true_pos.line,
      true_col,
      context_lines: vec![line],
      underlines: vec![(line, col, col + 1, message)],
    };
  }

  let end_pos_raw = file.position_at(span.end);
  let (end_line, end_col_bytes) =
    if end_pos_raw.column == 0 && end_pos_raw.line > true_pos.line {
      let prev = end_pos_raw.line - 1;

      (prev, file.line_text(prev).len())
    } else {
      (end_pos_raw.line, end_pos_raw.column)
    };

  let start_line_text = file.line_text(true_pos.line);
  let end_line_text = file.line_text(end_line);
  let start_col = render_col(start_line_text, true_pos.column);

  if true_pos.line == end_line {
    let end_col = render_col(end_line_text, end_col_bytes).max(start_col + 1);

    return Resolved {
      is_primary,
      true_line: true_pos.line,
      true_col,
      context_lines: vec![true_pos.line],
      underlines: vec![(true_pos.line, start_col, end_col, message)],
    };
  }

  let end_col = render_col(end_line_text, end_col_bytes).max(1);
  let span_lines = end_line - true_pos.line + 1;

  let context_lines = if span_lines <= 4 {
    (true_pos.line..=end_line).collect()
  } else {
    vec![true_pos.line, true_pos.line + 1, end_line]
  };

  Resolved {
    is_primary,
    true_line: true_pos.line,
    true_col,
    context_lines,
    underlines: vec![
      (
        true_pos.line,
        start_col,
        render_width(start_line_text).max(start_col + 1),
        None,
      ),
      (end_line, first_non_ws_col(end_line_text), end_col, message),
    ],
  }
}

enum Row {
  Source(usize),
  Gap,
}

fn plan_rows(resolved: &[Resolved]) -> Vec<Row> {
  let set: BTreeSet<usize> =
    resolved.iter().flat_map(|r| r.context_lines.iter().copied()).collect();

  let mut rows = Vec::new();
  let mut last: Option<usize> = None;

  for line in set {
    match last {
      None => rows.push(Row::Source(line)),
      Some(l) if line == l + 1 => rows.push(Row::Source(line)),
      Some(l) if line == l + 2 => {
        rows.push(Row::Source(l + 1));
        rows.push(Row::Source(line));
      }
      _ => {
        rows.push(Row::Gap);
        rows.push(Row::Source(line));
      }
    }

    last = Some(line);
  }

  rows
}

fn render_frame(
  out: &mut String,
  file_name: &str,
  file: &SourceFile,
  resolved: &[Resolved],
) -> usize {
  let rows = plan_rows(resolved);
  let max_line = rows.iter().filter_map(|r| match r {
    Row::Source(l) => Some(*l + 1),
    Row::Gap => None,
  });
  let w = digits(max_line.max().unwrap_or(1));

  let anchor = resolved.iter().find(|r| r.is_primary).unwrap_or(&resolved[0]);

  let _ = writeln!(
    out,
    "{}--> {}:{}:{}",
    pad(w),
    file_name,
    anchor.true_line + 1,
    anchor.true_col
  );
  let _ = writeln!(out, "{}|", pad(w + 1));

  for row in &rows {
    match row {
      Row::Gap => out.push_str("...\n"),
      Row::Source(line) => {
        let text = display_line(file.line_text(*line));

        let _ = writeln!(out, "{:>width$} | {}", line + 1, text, width = w);

        let mut underlines: Vec<_> = resolved
          .iter()
          .flat_map(|r| {
            r.underlines
              .iter()
              .filter(|u| u.0 == *line)
              .map(move |u| (r.is_primary, u))
          })
          .collect();
        underlines.sort_by_key(|(is_primary, u)| (!is_primary, u.1));

        for (is_primary, (_, start_col, end_col, message)) in underlines {
          out.push_str(&pad(w + 1));
          out.push_str("| ");
          out.push_str(&pad(*start_col));

          let width = end_col.saturating_sub(*start_col).max(1);
          out.push_str(&(if is_primary { "^" } else { "-" }).repeat(width));

          if let Some(msg) = message {
            out.push(' ');
            out.push_str(msg);
          }

          out.push('\n');
        }
      }
    }
  }

  w
}

fn build_frames<'a, S: BuildHasher>(
  d: &'a Diagnostic,
  files: &'a HashMap<Arc<str>, SourceFile, S>,
) -> Vec<(Arc<str>, Vec<Resolved<'a>>)> {
  let primary_file = d.primary.span.file.clone();
  let mut order: Vec<Arc<str>> = vec![primary_file.clone()];
  let mut by_file: HashMap<Arc<str>, Vec<Resolved<'a>>> = HashMap::new();

  by_file.entry(primary_file.clone()).or_default().push(resolve_label(
    &files[&primary_file],
    &d.primary.span,
    true,
    d.primary.message.as_deref(),
  ));

  for label in &d.secondary {
    let file_name = label.span.file.clone();

    if !order.contains(&file_name) {
      order.push(file_name.clone());
    }

    by_file.entry(file_name.clone()).or_default().push(resolve_label(
      &files[&file_name],
      &label.span,
      false,
      label.message.as_deref(),
    ));
  }

  order
    .into_iter()
    .map(|name| (name.clone(), by_file.remove(&name).unwrap_or_default()))
    .collect()
}

#[must_use]
pub fn render_diagnostic<S: BuildHasher>(
  d: &Diagnostic,
  files: &HashMap<Arc<str>, SourceFile, S>,
  options: RenderOptions,
) -> String {
  let style = Style { color: options.color };
  let mut out = String::new();

  let severity_word = match d.severity {
    Severity::Error => "error",
    Severity::Warning => "warning",
    Severity::Info => "info",
  };

  out.push_str(&style.severity(d.severity, severity_word));
  let _ = write!(out, "[{}]: ", d.code);
  out.push_str(&style.bold(&d.message));
  out.push('\n');

  let frames = build_frames(d, files);
  let mut primary_w = 1;

  for (i, (file_name, resolved)) in frames.iter().enumerate() {
    if resolved.is_empty() {
      continue;
    }

    let w = render_frame(&mut out, file_name, &files[file_name], resolved);

    if i == 0 {
      primary_w = w;
    }
  }

  if !d.notes.is_empty() || d.help.is_some() {
    let _ = writeln!(out, "{}|", pad(primary_w + 1));

    for note in &d.notes {
      let _ = writeln!(out, "{}= note: {note}", pad(primary_w + 1));
    }

    if let Some(help) = &d.help {
      let _ = writeln!(out, "{}= help: {help}", pad(primary_w + 1));
    }
  }

  out
}

#[must_use]
pub fn render_diagnostics<S: BuildHasher>(
  ds: &[Diagnostic],
  files: &HashMap<Arc<str>, SourceFile, S>,
  options: RenderOptions,
) -> String {
  let mut out = String::new();

  for d in ds {
    out.push_str(&render_diagnostic(d, files, options));
    out.push('\n');
  }

  let errors = ds.iter().filter(|d| d.severity == Severity::Error).count();
  let warnings = ds.iter().filter(|d| d.severity == Severity::Warning).count();

  if errors > 0 {
    let _ = write!(
      out,
      "error: aborting due to {errors} error{}",
      if errors == 1 { "" } else { "s" }
    );

    if warnings > 0 {
      let _ = write!(
        out,
        "; {warnings} warning{} emitted",
        if warnings == 1 { "" } else { "s" }
      );
    }

    out.push('\n');
  } else if warnings > 0 {
    let _ = writeln!(
      out,
      "warning: {warnings} warning{} emitted",
      if warnings == 1 { "" } else { "s" }
    );
  }

  out
}

#[must_use]
pub fn should_use_color(
  env: &dyn Fn(&str) -> Option<String>,
  is_tty: bool,
  flag: Option<bool>,
) -> bool {
  if let Some(flag) = flag {
    return flag;
  }

  if let Some(v) = env("NO_COLOR") {
    if !v.is_empty() {
      return false;
    }
  }

  if let Some(v) = env("FORCE_COLOR") {
    if v != "0" {
      return true;
    }
  }

  is_tty
}
