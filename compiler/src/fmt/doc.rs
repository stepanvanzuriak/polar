use crate::shared::ice::invariant;
use std::rc::Rc;

#[derive(Debug, Clone)]
pub struct Doc {
  node: Rc<DocNode>,
  flags: Flags,
}

#[derive(Debug, Clone, Copy, Default)]
#[allow(clippy::struct_excessive_bools, reason = "six independent facts")]
struct Flags {
  hard: bool,
  line: bool,
  sure: bool,
  pending: bool,
  must_break: bool,
  suffix: bool,
}

#[derive(Debug)]
pub enum DocNode {
  Text(String),
  Concat(Vec<Doc>),
  Line,
  SoftLine,
  HardLine,
  Nest(u16, Doc),
  Group { doc: Doc, has_hard_line: bool },
  IfBreak { broken: Doc, flat: Doc },
  LineSuffix(Doc, bool),
  FreshLine,
  Anchor { leads: bool },
  Closer(u16, Doc),
}

impl Doc {
  fn new(node: DocNode, flags: Flags) -> Self {
    Self { node: Rc::new(node), flags }
  }

  #[must_use]
  pub fn node(&self) -> &DocNode {
    &self.node
  }

  #[must_use]
  pub fn has_hard_line(&self) -> bool {
    self.flags.hard
  }
}

thread_local! {
  static HOLLOW: Rc<DocNode> = Rc::new(DocNode::Concat(Vec::new()));
}

impl Drop for Doc {
  fn drop(&mut self) {
    if Rc::strong_count(&self.node) != 1 {
      return;
    }

    let mut work = vec![hollow(self)];

    while let Some(rc) = work.pop() {
      let Ok(mut node) = Rc::try_unwrap(rc) else { continue };

      match &mut node {
        DocNode::Concat(parts) => work.extend(parts.iter_mut().map(hollow)),
        DocNode::Nest(_, doc)
        | DocNode::Group { doc, .. }
        | DocNode::LineSuffix(doc, _)
        | DocNode::Closer(_, doc) => work.push(hollow(doc)),
        DocNode::IfBreak { broken, flat } => {
          work.push(hollow(broken));
          work.push(hollow(flat));
        }
        DocNode::Text(_)
        | DocNode::Line
        | DocNode::SoftLine
        | DocNode::HardLine
        | DocNode::FreshLine
        | DocNode::Anchor { .. } => {}
      }
    }
  }
}

fn hollow(doc: &mut Doc) -> Rc<DocNode> {
  std::mem::replace(&mut doc.node, HOLLOW.with(Rc::clone))
}

#[track_caller]
#[must_use]
pub fn text(s: impl Into<String>) -> Doc {
  let s = s.into();

  invariant(!s.contains('\n'), || {
    format!("a `Text` document contains a newline: {s:?}")
  });

  Doc::new(DocNode::Text(s), Flags::default())
}

#[must_use]
pub fn nil() -> Doc {
  Doc::new(DocNode::Concat(Vec::new()), Flags::default())
}

#[must_use]
pub fn concat(parts: impl IntoIterator<Item = Doc>) -> Doc {
  let parts: Vec<Doc> = parts.into_iter().collect();
  let mut flags = Flags::default();

  for part in &parts {
    let f = part.flags;

    flags.must_break |= f.must_break || (flags.pending && f.line && !f.sure);
    flags.pending = f.pending || (flags.pending && !f.line);
    flags.sure = if flags.line { flags.sure } else { f.sure };
    flags.hard |= f.hard;
    flags.line |= f.line;
    flags.suffix |= f.suffix;
  }

  Doc::new(DocNode::Concat(parts), flags)
}

fn line_flags() -> Flags {
  Flags { line: true, ..Flags::default() }
}

#[must_use]
pub fn line() -> Doc {
  Doc::new(DocNode::Line, line_flags())
}

#[must_use]
pub fn softline() -> Doc {
  Doc::new(DocNode::SoftLine, line_flags())
}

#[must_use]
pub fn hardline() -> Doc {
  let flags = Flags { hard: true, sure: true, ..line_flags() };

  Doc::new(DocNode::HardLine, flags)
}

#[must_use]
pub fn nest(indent: u16, doc: Doc) -> Doc {
  let flags = doc.flags;

  Doc::new(DocNode::Nest(indent, doc), flags)
}

#[must_use]
pub fn group(doc: Doc) -> Doc {
  let inner = doc.flags;
  let has_hard_line = inner.hard || inner.must_break;

  let flags = Flags {
    hard: inner.hard,
    line: inner.line,
    sure: inner.line,
    pending: inner.pending,
    must_break: false,
    suffix: inner.suffix,
  };

  Doc::new(DocNode::Group { doc, has_hard_line }, flags)
}

#[must_use]
pub fn if_break(broken: Doc, flat: Doc) -> Doc {
  let flags = Flags {
    hard: broken.flags.hard || flat.flags.hard,
    suffix: broken.flags.suffix || flat.flags.suffix,
    ..broken.flags
  };

  Doc::new(DocNode::IfBreak { broken, flat }, flags)
}

#[must_use]
pub fn line_suffix(doc: Doc) -> Doc {
  let flags = Flags { suffix: true, ..Flags::default() };

  Doc::new(DocNode::LineSuffix(doc, false), flags)
}

#[must_use]
pub fn breaking_suffix(doc: Doc) -> Doc {
  let flags = Flags { pending: true, suffix: true, ..Flags::default() };

  Doc::new(DocNode::LineSuffix(doc, true), flags)
}

#[must_use]
pub fn fresh_line() -> Doc {
  Doc::new(DocNode::FreshLine, Flags::default())
}

#[must_use]
pub fn anchor() -> Doc {
  Doc::new(DocNode::Anchor { leads: true }, Flags::default())
}

#[must_use]
pub fn tail_anchor() -> Doc {
  Doc::new(DocNode::Anchor { leads: false }, Flags::default())
}

#[must_use]
pub fn closer(step: u16, doc: Doc) -> Doc {
  let flags = doc.flags;

  Doc::new(DocNode::Closer(step, doc), flags)
}

#[must_use]
pub fn join(docs: impl IntoIterator<Item = Doc>, sep: &Doc) -> Doc {
  let mut parts = Vec::new();

  for (i, doc) in docs.into_iter().enumerate() {
    if i > 0 {
      parts.push(sep.clone());
    }

    parts.push(doc);
  }

  concat(parts)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
  Flat,
  Break,
}

#[derive(Clone, Copy)]
enum Cmd<'d> {
  Doc(usize, Mode, &'d DocNode),
  SuffixStart,
  SuffixEnd,
  Hold(&'d DocNode),
  Closed,
}

#[derive(Default)]
#[allow(clippy::struct_excessive_bools, reason = "four independent facts")]
struct Out {
  text: String,
  column: usize,
  pending_indent: Option<usize>,
  line_indent: usize,
  in_suffix: bool,
  line_has_suffix: bool,
  holding: bool,
  held: Vec<String>,
  after_closer: bool,
}

impl Out {
  fn write(&mut self, s: &str) {
    if s.is_empty() {
      return;
    }

    if self.holding {
      if let Some(held) = self.held.last_mut() {
        held.push_str(s);
      }

      return;
    }

    if let Some(at) = self.pending_indent
      && (self.in_suffix || s.starts_with("//"))
    {
      self.write_held_at_anchor(at);
    }

    if s.starts_with("//") && self.text.ends_with(|c| c != ' ' && c != '\n') {
      self.text.push(' ');
      self.column += 1;
    }

    if !self.in_suffix && !s.trim_start_matches(' ').is_empty() {
      self.after_closer = false;
    }

    let s = match self.pending_indent.take() {
      Some(n) => {
        self.text.extend(std::iter::repeat_n(' ', n));
        s.trim_start_matches(' ')
      }
      None => s,
    };

    self.text.push_str(s);
    self.column += s.chars().count();

    if self.in_suffix {
      self.line_has_suffix = true;
    }
  }

  fn newline(&mut self, indent: usize) {
    trim_trailing_spaces(&mut self.text);
    self.text.push('\n');
    self.pending_indent = Some(indent);
    self.line_indent = indent;
    self.column = indent;
    self.line_has_suffix = false;
    self.after_closer = false;

    if std::mem::take(&mut self.holding)
      && self.held.last().is_some_and(String::is_empty)
    {
      self.held.pop();
    }
  }

  fn write_held_at_anchor(&mut self, indent: usize) {
    if self.held.is_empty() {
      return;
    }

    for held in std::mem::take(&mut self.held) {
      self.write(held.trim_start_matches(' '));
      self.newline(indent);
    }
  }

  fn write_held_above_closer(&mut self, step: usize) {
    let Some(at) = self.pending_indent else { return };

    for held in std::mem::take(&mut self.held) {
      self.pending_indent = Some(at + step);
      self.write(held.trim_start_matches(' '));
      self.newline(at);
    }
  }

  fn mark(&mut self, cmd: Cmd<'_>) {
    match cmd {
      Cmd::SuffixStart => {
        self.in_suffix = true;
        self.holding = self.line_has_suffix;

        if self.holding {
          self.held.push(String::new());
        } else {
          trim_trailing_spaces(&mut self.text);
        }
      }
      Cmd::SuffixEnd => {
        self.in_suffix = false;
        self.holding = false;
      }
      Cmd::Hold(suffix) => {
        self.held.push(suffix_text(suffix));
        self.line_has_suffix = true;
      }
      Cmd::Closed => self.after_closer = true,
      Cmd::Doc(..) => {}
    }
  }

  fn end_held(&mut self) {
    for held in std::mem::take(&mut self.held) {
      self.newline(self.line_indent);
      self.write(held.trim_start_matches(' '));
    }
  }
}

#[must_use]
pub fn render(doc: &Doc, width: usize) -> String {
  let mut out = Out::default();
  let mut stack: Vec<Cmd<'_>> = vec![Cmd::Doc(0, Mode::Break, doc.node())];
  let mut suffixes: Vec<(usize, Mode, &DocNode)> = Vec::new();
  let mut waiting = false;

  loop {
    let (indent, mode, node) = match stack.pop() {
      Some(Cmd::Doc(indent, mode, node)) => (indent, mode, node),
      Some(cmd) => {
        out.mark(cmd);
        continue;
      }
      None if suffixes.is_empty() => break,
      None => {
        flush(&mut stack, &mut suffixes, 0, Next::End);
        waiting = false;
        continue;
      }
    };

    match node {
      DocNode::Text(s) => out.write(s),
      DocNode::Concat(parts) => stack.extend(
        parts.iter().rev().map(|part| Cmd::Doc(indent, mode, part.node())),
      ),
      DocNode::Nest(n, inner) => {
        stack.push(Cmd::Doc(indent + usize::from(*n), mode, inner.node()));
      }
      DocNode::Group { doc, has_hard_line } => {
        let mode = match mode {
          _ if *has_hard_line => Mode::Break,
          Mode::Flat if !(waiting && doc.flags.line) => Mode::Flat,
          _ => {
            let next = (indent, Mode::Flat, doc.node());
            let room = width.saturating_sub(out.column);
            let context = Pending { suffixes: &suffixes, waiting };

            if fits(next, doc.flags.suffix, &stack, room, context) {
              Mode::Flat
            } else {
              Mode::Break
            }
          }
        };

        stack.push(Cmd::Doc(indent, mode, doc.node()));
      }
      DocNode::IfBreak { broken, flat } => {
        let chosen = if mode == Mode::Break { broken } else { flat };

        stack.push(Cmd::Doc(indent, mode, chosen.node()));
      }
      DocNode::LineSuffix(inner, breaking) => {
        waiting |= *breaking;
        suffixes.push((indent, mode, inner.node()));
      }
      DocNode::FreshLine => {
        if !suffixes.is_empty() || out.after_closer {
          stack.push(Cmd::Doc(indent, Mode::Break, &DocNode::HardLine));
        }
      }
      DocNode::Anchor { .. } if out.held.is_empty() => {}
      DocNode::Anchor { .. } if suffixes.is_empty() => {
        out.write_held_at_anchor(indent);
      }
      DocNode::Anchor { .. } => {
        stack.push(Cmd::Doc(indent, mode, node));
        stack.push(Cmd::Doc(indent, Mode::Break, &DocNode::HardLine));
      }
      DocNode::Closer(step, doc) => {
        out.write_held_above_closer(usize::from(*step));
        stack.push(Cmd::Closed);
        stack.push(Cmd::Doc(indent, mode, doc.node()));
      }
      DocNode::Line | DocNode::SoftLine | DocNode::HardLine => {
        let is_newline =
          mode == Mode::Break || matches!(node, DocNode::HardLine);

        if !is_newline {
          if matches!(node, DocNode::Line) {
            out.write(" ");
          }

          continue;
        }

        if !suffixes.is_empty() {
          let next = next_line(&stack);

          stack.push(Cmd::Doc(indent, mode, node));
          flush(&mut stack, &mut suffixes, indent, next);
          waiting = false;
          continue;
        }

        out.newline(indent);
      }
    }
  }

  out.end_held();
  trim_trailing_spaces(&mut out.text);

  out.text
}

fn flush<'d>(
  stack: &mut Vec<Cmd<'d>>,
  suffixes: &mut Vec<(usize, Mode, &'d DocNode)>,
  at: usize,
  next: Next,
) {
  for (indent, mode, node) in suffixes.drain(..).rev() {
    if next == Next::Text && starts_own_line(node) {
      stack.push(Cmd::Hold(node));
      continue;
    }

    let indent = match next {
      Next::Closer(step) => at + step,
      Next::End => indent.max(at),
      Next::Anchor | Next::Text => at,
    };

    stack.push(Cmd::SuffixEnd);
    stack.push(Cmd::Doc(indent, mode, node));
    stack.push(Cmd::SuffixStart);
  }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Next {
  Closer(usize),
  Anchor,
  Text,
  End,
}

fn next_line(stack: &[Cmd<'_>]) -> Next {
  let mut local: Vec<(Mode, &DocNode)> = Vec::new();
  let mut rest = stack.iter().rev().filter_map(|cmd| match *cmd {
    Cmd::Doc(_, mode, node) => Some((mode, node)),
    _ => None,
  });

  loop {
    let Some((mode, node)) = local.pop().or_else(|| rest.next()) else {
      return Next::End;
    };

    match node {
      DocNode::Closer(step, _) => return Next::Closer(usize::from(*step)),
      DocNode::Anchor { .. } => return Next::Anchor,
      DocNode::Text(s) if s.is_empty() => {}
      DocNode::Text(_) => return Next::Text,
      DocNode::Concat(parts) => {
        local.extend(parts.iter().rev().map(|part| (mode, part.node())));
      }
      DocNode::Nest(_, doc) | DocNode::Group { doc, .. } => {
        local.push((mode, doc.node()));
      }
      DocNode::IfBreak { broken, flat } => {
        let chosen = if mode == Mode::Break { broken } else { flat };

        local.push((mode, chosen.node()));
      }
      DocNode::Line
      | DocNode::SoftLine
      | DocNode::HardLine
      | DocNode::LineSuffix(..)
      | DocNode::FreshLine => {}
    }
  }
}

fn suffix_text(node: &DocNode) -> String {
  match node {
    DocNode::Text(s) => s.clone(),
    DocNode::Concat(parts) => {
      parts.iter().map(|part| suffix_text(part.node())).collect()
    }
    DocNode::Nest(_, doc) => suffix_text(doc.node()),
    _ => String::new(),
  }
}

#[derive(Clone, Copy)]
struct Pending<'s, 'd> {
  suffixes: &'s [(usize, Mode, &'d DocNode)],
  waiting: bool,
}

#[derive(Default)]
#[allow(clippy::struct_excessive_bools, reason = "four independent facts")]
struct Release {
  suffixes: usize,
  holds: bool,
  own_line: bool,
  after_break: bool,
  releasing: bool,
}

impl Release {
  fn suffix(&mut self, node: &DocNode) {
    let own_line = starts_own_line(node);

    self.holds |= self.suffixes > 0 && !own_line;
    self.own_line |= own_line;
    self.suffixes += 1;
  }

  fn line_break(&mut self) {
    *self = Self {
      releasing: self.releasing || self.holds,
      after_break: self.own_line,
      ..Self::default()
    };
  }

  fn leads_at(&mut self, node: &DocNode) -> bool {
    if self.after_break {
      match node {
        DocNode::Text(s) if !s.is_empty() => {
          self.after_break = false;
          self.releasing = true;
        }
        DocNode::Anchor { .. } | DocNode::Closer(..) => {
          self.after_break = false;
        }
        _ => {}
      }
    }

    match node {
      DocNode::Anchor { leads } if self.releasing => {
        self.releasing = false;
        *leads
      }
      DocNode::Closer(..) => {
        self.releasing = false;
        false
      }
      _ => false,
    }
  }
}

fn fits(
  next: (usize, Mode, &DocNode),
  has_suffix: bool,
  rest: &[Cmd<'_>],
  width: usize,
  pending: Pending<'_, '_>,
) -> bool {
  let mut remaining = width;
  let mut local = vec![(next.0, next.1, next.2, true)];
  let mut rest = rest.iter().rev().filter_map(|cmd| match *cmd {
    Cmd::Doc(indent, mode, node) => Some((indent, mode, node, false)),
    _ => None,
  });
  let mut release = Release::default();
  let mut waiting = pending.waiting;
  let mut inside = true;
  let mut broken = false;

  for (_, _, suffix) in pending.suffixes {
    release.suffix(suffix);
  }

  loop {
    let (indent, mode, node, own) = match local.pop() {
      Some(cmd) => cmd,
      None if broken => return true,
      None => {
        inside = false;

        let Some(cmd) = rest.next() else { return true };

        cmd
      }
    };

    let newline = match node {
      DocNode::Line | DocNode::SoftLine if mode == Mode::Flat => {
        if waiting && inside && own && !broken {
          return false;
        }

        waiting && inside
      }
      DocNode::HardLine | DocNode::Line | DocNode::SoftLine => true,
      _ => false,
    };

    if newline {
      if !inside || !(release.holds || release.own_line || has_suffix) {
        return true;
      }

      broken = true;
      release.line_break();
      waiting = false;
      continue;
    }

    if release.leads_at(node) {
      return false;
    }

    match node {
      DocNode::Closer(_, doc) => {
        local.push((indent, mode, doc.node(), own));
      }
      DocNode::Text(_) | DocNode::Line if broken => {}
      DocNode::Text(s) => {
        let n = s.chars().count();

        if n > remaining {
          return false;
        }

        remaining -= n;
      }
      DocNode::Concat(parts) => {
        local.extend(
          parts.iter().rev().map(|part| (indent, mode, part.node(), own)),
        );
      }
      DocNode::Nest(n, inner) => {
        local.push((indent + usize::from(*n), mode, inner.node(), own));
      }
      DocNode::Group { doc, has_hard_line } => {
        let mode = match mode {
          _ if *has_hard_line => Mode::Break,
          _ if broken => Mode::Flat,
          mode => mode,
        };

        local.push((indent, mode, doc.node(), false));
      }
      DocNode::IfBreak { broken, flat } => {
        let chosen = if mode == Mode::Break { broken } else { flat };

        local.push((indent, mode, chosen.node(), own));
      }
      DocNode::Line => {
        if remaining == 0 {
          return false;
        }

        remaining -= 1;
      }
      DocNode::LineSuffix(suffix, breaking) => {
        release.suffix(suffix.node());
        waiting |= *breaking;
      }
      DocNode::HardLine
      | DocNode::SoftLine
      | DocNode::FreshLine
      | DocNode::Anchor { .. } => {}
    }
  }
}

fn starts_own_line(node: &DocNode) -> bool {
  match node {
    DocNode::HardLine => true,
    DocNode::Concat(parts) => {
      parts.first().is_some_and(|part| starts_own_line(part.node()))
    }
    DocNode::Nest(_, doc) => starts_own_line(doc.node()),
    _ => false,
  }
}

fn trim_trailing_spaces(out: &mut String) {
  let trimmed = out.trim_end_matches(' ').len();

  out.truncate(trimmed);
}
