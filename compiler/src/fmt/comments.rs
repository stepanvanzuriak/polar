use super::print::{opens_list, prints_dangling};
use crate::{
  shared::source::Span,
  syntax::ast::{
    Module,
    fields::{AsNode, NodeRef, children},
  },
  syntax::lexer::token::Comment,
};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
  Leading,
  Trailing,
  TrailingOwnLine,
  Dangling,
  AfterOpen,
  AfterBracket,
  Before,
}

#[derive(Debug, Clone)]
pub struct Attached {
  pub span: Span,
  pub placement: Placement,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NodeKey {
  addr: usize,
  kind: &'static str,
}

impl NodeKey {
  #[must_use]
  pub fn of(node: NodeRef<'_>) -> Self {
    Self { addr: std::ptr::from_ref(node.span()) as usize, kind: node.kind() }
  }
}

#[derive(Debug, Default)]
pub struct Attachments {
  map: HashMap<NodeKey, Vec<Attached>>,
}

impl Attachments {
  #[must_use]
  pub fn get(&self, node: NodeRef<'_>) -> &[Attached] {
    self.map.get(&NodeKey::of(node)).map_or(&[], Vec::as_slice)
  }

  #[must_use]
  pub fn is_empty(&self) -> bool {
    self.map.is_empty()
  }

  fn push(&mut self, node: NodeRef<'_>, span: &Span, placement: Placement) {
    self
      .map
      .entry(NodeKey::of(node))
      .or_default()
      .push(Attached { span: span.clone(), placement });
  }
}

#[must_use]
pub fn attach(
  module: &Module,
  comments: &[Comment],
  source: &str,
) -> Attachments {
  let mut out = Attachments::default();

  for comment in comments {
    let span = &comment.span;
    let enclosing = enclosing(module.as_node(), span);

    if matches!(enclosing, NodeRef::PluginEntry(_)) {
      continue;
    }

    let before_header = match enclosing {
      NodeRef::Module(m) => {
        m.name.as_ref().is_some_and(|name| span.end <= name.span.start)
      }
      _ => false,
    };

    if before_header {
      out.push(enclosing, span, Placement::Leading);
      continue;
    }

    let header = match enclosing {
      NodeRef::TraitDecl(_) | NodeRef::EffectDecl(_) | NodeRef::BindDecl(_) => {
        !source[enclosing.span().start..span.start].contains('{')
      }
      _ => false,
    };

    if header {
      out.push(enclosing, span, Placement::Before);
      continue;
    }

    let kids = children(enclosing);
    let preceding = kids.iter().rev().find(|kid| kid.span().end <= span.start);
    let following = kids.iter().find(|kid| kid.span().start >= span.end);

    if after_own_bracket(enclosing, preceding.copied(), source, span.start) {
      out.push(enclosing, span, Placement::AfterBracket);
      continue;
    }

    let after_open = !matches!(enclosing, NodeRef::Module(_))
      && preceding.is_none()
      && (following.is_some()
        || !prints_dangling(enclosing)
        || !opens_bracket(&source[enclosing.span().start..span.start]))
      && !has_line_break(&source[enclosing.span().start..span.start]);

    if after_open {
      match following {
        Some(NodeRef::Bound(_)) => {
          out.push(enclosing, span, Placement::AfterOpen);
        }
        Some(after)
          if !matches!(enclosing, NodeRef::Zone(_))
            && !after_bracket(&source[..span.start])
            && !opens_bracket(&source[span.end..after.span().start]) =>
        {
          out.push(*after, span, Placement::Before);
        }
        _ => out.push(enclosing, span, Placement::AfterOpen),
      }

      continue;
    }

    match (preceding, following) {
      (Some(before), Some(after))
        if !has_line_break(&source[before.span().end..span.start])
          && past_token(&source[before.span().end..span.start]) =>
      {
        out.push(*after, span, Placement::Before);
      }
      (Some(before), _)
        if !has_line_break(&source[before.span().end..span.start]) =>
      {
        out.push(*before, span, Placement::Trailing);
      }
      (Some(before @ NodeRef::Zone(_)), Some(_))
        if column(source, span.start) > 0 =>
      {
        out.push(*before, span, Placement::TrailingOwnLine);
      }
      (Some(before), Some(_)) if closes(&source[span.end..]) => {
        out.push(*before, span, Placement::TrailingOwnLine);
      }
      (_, Some(after)) => out.push(*after, span, Placement::Leading),
      (Some(before), None) => {
        out.push(*before, span, Placement::TrailingOwnLine);
      }
      (None, None) => out.push(enclosing, span, Placement::Dangling),
    }
  }

  out
}

fn enclosing<'a>(root: NodeRef<'a>, span: &Span) -> NodeRef<'a> {
  let mut node = root;

  'descend: loop {
    for kid in children(node) {
      let outer = kid.span();

      if outer.start <= span.start && span.end <= outer.end {
        node = kid;
        continue 'descend;
      }
    }

    return node;
  }
}

fn past_token(between: &str) -> bool {
  between.contains(|c: char| !c.is_whitespace() && !",)]}".contains(c))
}

fn after_own_bracket(
  enclosing: NodeRef<'_>,
  preceding: Option<NodeRef<'_>>,
  source: &str,
  at: usize,
) -> bool {
  let before = source[..at].trim_end_matches([' ', '\t']);

  if !before.ends_with(['(', '[', '{']) || !opens_list(enclosing) {
    return false;
  }

  let bracket = before.len() - 1;

  match (enclosing, preceding) {
    (NodeRef::Try(_), Some(head)) => {
      source[head.span().end..bracket].trim() == "catch"
    }
    (
      NodeRef::Call(_) | NodeRef::Match(_) | NodeRef::ImplDecl(_),
      Some(head),
    ) => source[head.span().end..bracket]
      .trim_matches(|c: char| c.is_whitespace() || c == ')')
      .is_empty(),
    (
      NodeRef::Call(_)
      | NodeRef::Match(_)
      | NodeRef::ImplDecl(_)
      | NodeRef::Try(_),
      None,
    )
    | (_, Some(_)) => false,
    (_, None) => !opens_bracket(&source[enclosing.span().start..bracket]),
  }
}

fn opens_bracket(text: &str) -> bool {
  text.contains(['(', '[', '{'])
}

fn after_bracket(before: &str) -> bool {
  before.trim_end().ends_with(['{', '(', '['])
}

fn closes(mut rest: &str) -> bool {
  loop {
    rest = rest.trim_start();

    if !rest.starts_with("//") {
      return rest.starts_with([')', ']', '}']);
    }

    rest = rest.find(['\n', '\r']).map_or("", |i| &rest[i..]);
  }
}

fn column(source: &str, offset: usize) -> usize {
  let line_start = source[..offset].rfind(['\n', '\r']).map_or(0, |i| i + 1);

  offset - line_start
}

pub(super) fn has_line_break(text: &str) -> bool {
  text.contains(['\n', '\r'])
}

pub(super) fn line_breaks(text: &str) -> usize {
  text.matches('\n').count()
    + text
      .as_bytes()
      .windows(2)
      .filter(|w| w[0] == b'\r' && w[1] != b'\n')
      .count()
    + usize::from(text.ends_with('\r'))
}
