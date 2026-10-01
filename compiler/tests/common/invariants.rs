use polar_compiler::syntax::ast::{
  Module, Name,
  fields::{AsNode, Field, NodeRef, children},
};

pub fn check_invariants(src: &str, module: &Module) {
  check_node(src, module.as_node());
}

fn check_name(src: &str, parent: NodeRef<'_>, name: &Name) {
  let (start, end) = (name.span.start, name.span.end);
  let outer = parent.span();

  if polar_compiler::syntax::params::is_synthetic(&name.text) {
    assert!(outer.start <= start && end <= outer.end);
    return;
  }

  assert_eq!(&src[start..end], name.text, "a `Name` must slice to its text");
  assert!(
    outer.start <= start && end <= outer.end,
    "name `{}` at {start}..{end} lies outside its {} at {}..{}",
    name.text,
    parent.kind(),
    outer.start,
    outer.end
  );
}

fn check_node(src: &str, node: NodeRef<'_>) {
  let span = node.span();

  assert!(
    span.start <= span.end && span.end <= src.len(),
    "{} has an out-of-range span {}..{}",
    node.kind(),
    span.start,
    span.end
  );

  let mut unordered = None;

  for (key, field) in node.fields() {
    match field {
      Field::Name(name) | Field::OptName(Some(name)) => {
        check_name(src, node, name);
      }
      Field::Names(names) | Field::OptNames(Some(names)) => {
        for name in names {
          check_name(src, node, name);
        }
      }
      Field::Text(raw) if key == "raw" => {
        assert_eq!(
          &src[span.start..span.end],
          raw,
          "a {} must slice back to its `raw` text",
          node.kind()
        );
      }
      Field::OptNode(Some(spread)) if key == "spread" => {
        unordered = Some(spread.span().clone());
      }
      _ => {}
    }
  }

  let kids = children(node);
  let ordered: Vec<_> =
    kids.iter().filter(|kid| Some(kid.span()) != unordered.as_ref()).collect();

  for pair in ordered.windows(2) {
    let (a, b) = (pair[0].span(), pair[1].span());

    assert!(
      a.end <= b.start,
      "siblings {} {}..{} and {} {}..{} overlap or are out of order",
      pair[0].kind(),
      a.start,
      a.end,
      pair[1].kind(),
      b.start,
      b.end
    );
  }

  for kid in kids {
    let inner = kid.span();

    assert!(
      span.start <= inner.start && inner.end <= span.end,
      "{} {}..{} lies outside its parent {} {}..{}",
      kid.kind(),
      inner.start,
      inner.end,
      node.kind(),
      span.start,
      span.end
    );

    check_node(src, kid);
  }
}
