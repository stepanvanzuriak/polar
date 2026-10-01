use crate::syntax::ast::{
  Module,
  fields::{AsNode, Field, NodeRef},
};
use std::fmt::Write as _;

#[must_use]
pub fn dump_ast(module: &Module) -> String {
  dump_node(module.as_node())
}

#[must_use]
pub fn dump_ast_shape(module: &Module) -> String {
  dump_node_shape(module.as_node())
}

#[must_use]
pub fn dump_node(node: NodeRef<'_>) -> String {
  dump(node, true)
}

#[must_use]
pub fn dump_node_shape(node: NodeRef<'_>) -> String {
  dump(node, false)
}

fn dump(node: NodeRef<'_>, spans: bool) -> String {
  let mut out = String::new();

  write_node(&mut out, node, 0, spans);
  out.push('\n');

  out
}

fn write_node(out: &mut String, node: NodeRef<'_>, indent: usize, spans: bool) {
  let fields = node.fields();

  let _ = write!(out, "{:indent$}({}", "", node.kind());

  for (key, field) in &fields {
    match field {
      Field::OptName(Some(name)) if *key == "alias" || *key == "from" => {
        let _ = write!(out, " {key}={}", name.text);
      }
      Field::Name(name) | Field::OptName(Some(name)) => {
        let _ = write!(out, " {}", name.text);
      }
      Field::Names(names) => {
        for name in *names {
          let _ = write!(out, " {}", name.text);
        }
      }
      Field::OptNames(Some(names)) => {
        let names: Vec<&str> = names.iter().map(|n| n.text.as_str()).collect();
        let _ = write!(out, " {key}=[{}]", names.join(", "));
      }
      Field::Tag(tag) => {
        let _ = write!(out, " {tag}");
      }
      Field::Text(text) => {
        let _ = write!(out, " {key}={text:?}");
      }
      Field::Bool(value) => {
        let _ = write!(out, " {key}={value}");
      }
      _ => {}
    }
  }

  if spans {
    let span = node.span();
    let _ = write!(out, " {}..{}", span.start, span.end);
  }

  for (_, field) in &fields {
    match field {
      Field::Node(child) | Field::OptNode(Some(child)) => {
        out.push('\n');
        write_node(out, *child, indent + 2, spans);
      }
      Field::Nodes(children) => {
        for child in children {
          out.push('\n');
          write_node(out, *child, indent + 2, spans);
        }
      }
      _ => {}
    }
  }

  out.push(')');
}
