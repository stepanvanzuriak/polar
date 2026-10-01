use polar_compiler::{
  shared::source::Span,
  syntax::ast::{
    Block, CtorDecl, Decl, EffectRow, ExportDecl, Expr, FieldType, FnDecl,
    FnType, Import, InvalidExpr, InvalidType, Module, Name, Param, RecordType,
    TypeBody, TypeDecl, TypeExpr, TypeRef, TypeVar, VariantBody, Zone,
    ZoneKind,
    fields::{AsNode, Fields, NodeRef, children},
  },
};
use std::sync::Arc;

const SRC: &str = "uses\n  net/http as web\ntypes\n  Id<k> = Int\nfunctions\n  run(x: Int, y) -> Bool / {Db} {}\nexports\n  run\n";

fn file() -> Arc<str> {
  Arc::from("test.px")
}

fn sp(needle: &str, nth: usize) -> Span {
  let (start, _) = SRC
    .match_indices(needle)
    .nth(nth)
    .unwrap_or_else(|| panic!("no occurrence {nth} of {needle:?} in fixture"));

  Span::new(file(), start, start + needle.len())
}

fn cover(from: (&str, usize), to: (&str, usize)) -> Span {
  let start = sp(from.0, from.1).start;
  let end = sp(to.0, to.1).end;

  Span::new(file(), start, end)
}

fn name(text: &str, nth: usize) -> Name {
  Name { text: text.to_string(), span: sp(text, nth) }
}

fn type_ref(text: &str, nth: usize) -> TypeExpr {
  TypeExpr::Ref(TypeRef {
    span: sp(text, nth),
    name: name(text, nth),
    args: vec![],
  })
}

fn fixture() -> Module {
  let uses = Zone {
    span: cover(("uses", 0), ("web", 0)),
    kind: ZoneKind::USES,
    decls: vec![Decl::Import(Import {
      methods: None,
      span: cover(("net", 0), ("web", 0)),
      path: vec![name("net", 0), name("http", 0)],
      alias: Some(name("web", 0)),
    })],
  };

  let types = Zone {
    span: cover(("types", 0), ("Int", 0)),
    kind: ZoneKind::TYPES,
    decls: vec![Decl::Type(TypeDecl {
      span: cover(("Id", 0), ("Int", 0)),
      docs: vec![],
      name: name("Id", 0),
      params: vec![name("k", 0)],
      body: TypeBody::Alias(type_ref("Int", 0)),
      derive: None,
    })],
  };

  let functions = Zone {
    span: cover(("functions", 0), ("{}", 0)),
    kind: ZoneKind::FUNCTIONS,
    decls: vec![Decl::Fn(FnDecl {
      bounds: vec![],
      span: cover(("run", 0), ("{}", 0)),
      docs: vec![],
      name: name("run", 0),
      params: vec![
        Param {
          span: cover(("x", 0), ("Int", 1)),
          name: name("x", 0),
          pattern: None,
          ty: Some(type_ref("Int", 1)),
        },
        Param { span: sp("y", 1), name: name("y", 1), pattern: None, ty: None },
      ],
      return_type: Some(type_ref("Bool", 0)),
      effects: Some(EffectRow {
        span: sp("{Db}", 0),
        entries: vec![TypeRef {
          span: sp("Db", 0),
          name: name("Db", 0),
          args: vec![],
        }],
        tail: None,
      }),
      body: Block {
        span: sp("{}", 0),
        stmts: vec![],
        result: Box::new(Expr::Invalid(InvalidExpr { span: sp("{}", 0) })),
      },
    })],
  };

  let exports = Zone {
    span: cover(("exports", 0), ("run", 1)),
    kind: ZoneKind::EXPORTS,
    decls: vec![Decl::Export(ExportDecl {
      methods: None,
      span: sp("run", 1),
      name: name("run", 1),
    })],
  };

  Module {
    span: Span::new(file(), 0, SRC.len()),
    name: None,
    zones: vec![uses, types, functions, exports],
  }
}

fn kinds(node: NodeRef<'_>) -> Vec<&'static str> {
  children(node).iter().map(NodeRef::kind).collect()
}

fn walk<'a>(node: NodeRef<'a>, out: &mut Vec<NodeRef<'a>>) {
  out.push(node);

  for child in children(node) {
    walk(child, out);
  }
}

fn all_nodes(module: &Module) -> Vec<NodeRef<'_>> {
  let mut out = Vec::new();
  walk(module.as_node(), &mut out);
  out
}

mod children_of {
  use super::*;

  #[test]
  fn children_of_module() {
    let module = fixture();

    assert_eq!(kinds(module.as_node()), ["Zone", "Zone", "Zone", "Zone"]);
  }

  #[test]
  fn children_of_zone() {
    let module = fixture();
    let functions = &module.zones[2];

    assert_eq!(kinds(functions.as_node()), ["FnDecl"]);
  }

  #[test]
  fn children_of_fn_decl() {
    let module = fixture();
    let Decl::Fn(f) = &module.zones[2].decls[0] else {
      panic!("expected a function")
    };

    assert_eq!(
      kinds(f.as_node()),
      ["Param", "Param", "TypeRef", "EffectRow", "Block"]
    );
  }

  #[test]
  fn children_skips_names() {
    let outer = TypeRef {
      span: sp("Id<k> = Int", 0),
      name: name("Id", 0),
      args: vec![type_ref("Int", 0)],
    };

    assert_eq!(kinds(outer.as_node()), ["TypeRef"]);
  }

  #[test]
  fn param_without_annotation() {
    let module = fixture();
    let Decl::Fn(f) = &module.zones[2].decls[0] else {
      panic!("expected a function")
    };
    let y = &f.params[1];

    assert!(y.ty.is_none());
    assert!(kinds(y.as_node()).is_empty());
  }

  #[test]
  fn children_are_ordered_over_fixtures() {
    let module = fixture();

    for node in all_nodes(&module) {
      let spans: Vec<_> =
        children(node).iter().map(|c| c.span().clone()).collect();

      for pair in spans.windows(2) {
        assert!(
          pair[0].end <= pair[1].start,
          "{} children out of order: {:?} then {:?}",
          node.kind(),
          pair[0],
          pair[1]
        );
      }
    }
  }
}

mod fields {
  use super::*;

  fn keys<T: Fields>(node: &T) -> Vec<&'static str> {
    node.fields().iter().map(|(k, _)| *k).collect()
  }

  #[test]
  fn field_keys_are_member_names() {
    let module = fixture();
    let Decl::Type(t) = &module.zones[1].decls[0] else {
      panic!("expected a type")
    };

    assert_eq!(keys(t), ["span", "docs", "name", "params", "body", "derive"]);
  }

  #[test]
  fn field_keys_cover_every_member_of_fn_decl() {
    let module = fixture();
    let Decl::Fn(f) = &module.zones[2].decls[0] else {
      panic!("expected a function")
    };

    assert_eq!(
      keys(f),
      [
        "span",
        "docs",
        "name",
        "params",
        "return_type",
        "effects",
        "bounds",
        "body"
      ]
    );
  }

  #[test]
  fn enum_fields_delegate_to_the_variant() {
    let alias = TypeBody::Alias(type_ref("Int", 0));

    assert_eq!(keys(&alias), ["span", "name", "args"]);
  }
}

mod spans {
  use super::*;

  #[test]
  fn every_node_carries_a_span() {
    let module = fixture();

    for node in all_nodes(&module) {
      assert!(
        node.span().start <= node.span().end,
        "{} has a reversed span",
        node.kind()
      );
    }
  }

  #[test]
  fn enum_span_delegates() {
    let r = TypeRef { span: sp("Int", 0), name: name("Int", 0), args: vec![] };
    let v = TypeVar { span: sp("k", 0), name: name("k", 0) };
    let f = FnType {
      span: sp("Bool", 0),
      params: vec![],
      ret: Box::new(type_ref("Int", 1)),
      effects: None,
    };
    let rec = RecordType {
      span: sp("{Db}", 0),
      fields: vec![FieldType {
        span: sp("Db", 0),
        name: name("Db", 0),
        ty: type_ref("Int", 1),
      }],
      tail: None,
    };
    let i = InvalidType { span: sp("{}", 0) };

    assert_eq!(TypeExpr::Ref(r.clone()).span(), &r.span);
    assert_eq!(TypeExpr::Var(v.clone()).span(), &v.span);
    assert_eq!(TypeExpr::Fn(f.clone()).span(), &f.span);
    assert_eq!(TypeExpr::Record(rec.clone()).span(), &rec.span);
    assert_eq!(TypeExpr::Invalid(i.clone()).span(), &i.span);
  }

  #[test]
  fn decl_and_expr_spans_delegate() {
    let e =
      ExportDecl { methods: None, span: sp("run", 1), name: name("run", 1) };
    let bad = InvalidExpr { span: sp("{}", 0) };
    let variants = VariantBody {
      span: sp("Int", 0),
      ctors: vec![CtorDecl {
        span: sp("Int", 0),
        name: name("Int", 0),
        args: vec![],
      }],
    };

    assert_eq!(Decl::Export(e.clone()).span(), &e.span);
    assert_eq!(Expr::Invalid(bad.clone()).span(), &bad.span);
    assert_eq!(TypeBody::Variants(variants.clone()).span(), &variants.span);
  }

  #[test]
  fn invalid_node_has_a_span() {
    let at = Span::new(file(), 7, 7);
    let invalid = InvalidType { span: at.clone() };

    assert_eq!(invalid.span, at);
    assert_eq!(invalid.span.start, invalid.span.end);
  }

  #[test]
  fn parent_covers_children() {
    let module = fixture();

    for node in all_nodes(&module) {
      let parent = node.span().clone();

      for child in children(node) {
        assert!(
          parent.start <= child.span().start && child.span().end <= parent.end,
          "{} {:?} does not cover {} {:?}",
          node.kind(),
          parent,
          child.kind(),
          child.span()
        );
      }
    }
  }

  #[test]
  fn module_span_covers_the_file() {
    let module = fixture();

    assert_eq!(module.span.start, 0);
    assert_eq!(module.span.end, SRC.len());
  }
}

mod ownership {
  use super::*;

  #[test]
  fn module_is_static() {
    fn assert_static<T: 'static>() {}

    assert_static::<Module>();
  }

  #[test]
  fn nodes_clone_and_compare() {
    let module = fixture();

    assert_eq!(module.clone(), module);
  }

  #[test]
  fn differing_nodes_compare_unequal() {
    let module = fixture();
    let mut other = module.clone();
    other.zones.pop();

    assert_ne!(other, module);
  }
}
