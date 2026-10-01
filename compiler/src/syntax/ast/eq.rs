use crate::syntax::ast::{
  Module, Name,
  fields::{AsNode, Field, NodeRef},
};

#[must_use]
pub fn ast_eq(a: &Module, b: &Module) -> bool {
  ast_diff(a, b).is_none()
}

#[must_use]
pub fn ast_diff(a: &Module, b: &Module) -> Option<String> {
  let mut work = vec![(String::new(), a.as_node(), b.as_node())];

  while let Some((path, a, b)) = work.pop() {
    if a.kind() != b.kind() {
      return Some(format!("{}: {} vs {}", display(&path), a.kind(), b.kind()));
    }

    let mut kids = Vec::new();

    for ((key, fa), (_, fb)) in a.fields().into_iter().zip(b.fields()) {
      let here = join(&path, key);

      if let Some(diff) = compare(&here, fa, fb, &mut kids) {
        return Some(diff);
      }
    }

    work.extend(kids.into_iter().rev());
  }

  None
}

type Pending<'a> = Vec<(String, NodeRef<'a>, NodeRef<'a>)>;

fn compare<'a>(
  path: &str,
  a: Field<'a>,
  b: Field<'a>,
  kids: &mut Pending<'a>,
) -> Option<String> {
  match (a, b) {
    (Field::Span(_), Field::Span(_)) | (Field::Spans(_), Field::Spans(_)) => {
      None
    }
    (Field::Name(a), Field::Name(b)) => names(path, a, b),
    (Field::OptName(a), Field::OptName(b)) => match (a, b) {
      (Some(a), Some(b)) => names(path, a, b),
      (None, None) => None,
      (a, b) => Some(format!(
        "{path}: {} vs {}",
        a.map_or("none".into(), |n| format!("{:?}", n.text)),
        b.map_or("none".into(), |n| format!("{:?}", n.text)),
      )),
    },
    (Field::OptNames(a), Field::OptNames(b)) => match (a, b) {
      (Some(a), Some(b)) => name_lists(path, a, b),
      (None, None) => None,
      (a, b) => Some(format!(
        "{path}: {} vs {}",
        if a.is_some() { "a list" } else { "none" },
        if b.is_some() { "a list" } else { "none" },
      )),
    },
    (Field::Names(a), Field::Names(b)) => name_lists(path, a, b),
    (Field::Text(a), Field::Text(b)) => {
      (a != b).then(|| format!("{path}: {a:?} vs {b:?}"))
    }
    (Field::Bool(a), Field::Bool(b)) => {
      (a != b).then(|| format!("{path}: {a} vs {b}"))
    }
    (Field::Tag(a), Field::Tag(b)) => {
      (a != b).then(|| format!("{path}: `{a}` vs `{b}`"))
    }
    (Field::Node(a), Field::Node(b)) => {
      kids.push((path.to_string(), a, b));
      None
    }
    (Field::OptNode(a), Field::OptNode(b)) => match (a, b) {
      (Some(a), Some(b)) => {
        kids.push((path.to_string(), a, b));
        None
      }
      (None, None) => None,
      (a, b) => Some(format!(
        "{path}: {} vs {}",
        a.map_or("none", |n| n.kind()),
        b.map_or("none", |n| n.kind()),
      )),
    },
    (Field::Nodes(a), Field::Nodes(b)) => {
      if a.len() != b.len() {
        return Some(format!("{path}: {} items vs {}", a.len(), b.len()));
      }

      kids.extend(
        a.into_iter()
          .zip(b)
          .enumerate()
          .map(|(i, (a, b))| (format!("{path}[{i}]"), a, b)),
      );

      None
    }
    _ => Some(format!("{path}: members of different shapes")),
  }
}

fn name_lists(path: &str, a: &[Name], b: &[Name]) -> Option<String> {
  if a.len() != b.len() {
    return Some(format!("{path}: {} names vs {}", a.len(), b.len()));
  }

  a.iter()
    .zip(b)
    .enumerate()
    .find_map(|(i, (a, b))| names(&format!("{path}[{i}]"), a, b))
}

fn names(path: &str, a: &Name, b: &Name) -> Option<String> {
  (a.text != b.text).then(|| format!("{path}: {:?} vs {:?}", a.text, b.text))
}

fn join(path: &str, key: &str) -> String {
  if path.is_empty() { key.to_string() } else { format!("{path}.{key}") }
}

fn display(path: &str) -> &str {
  if path.is_empty() { "module" } else { path }
}
