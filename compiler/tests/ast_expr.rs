use polar_compiler::{
  shared::source::Span,
  syntax::ast::{
    Binary, BinaryOp, Block, BoolLit, Call, EffectRow, Else, Expr, ExprStmt,
    FieldAccess, FieldInit, FloatLit, FnDecl, If, IntLit, InvalidExpr, Lambda,
    LetStmt, Match, MatchArm, Name, PCtor, PField, PLit, PRecord, PVar,
    PWildcard, Param, PatLit, Pattern, Pipe, RecordLit, Stmt, StringInterp,
    StringLit, StringPart, StringText, TypeExpr, TypeRef, Unary, UnaryOp, Var,
    dump::dump_node_shape,
    fields::{AsNode, NodeRef, children},
  },
};
use std::{collections::BTreeSet, sync::Arc};

struct Src(&'static str);

impl Src {
  fn all(&self) -> Span {
    Span::new(Arc::from("test.px"), 0, self.0.len())
  }

  fn at(&self, needle: &str, nth: usize) -> Span {
    let (start, _) =
      self.0.match_indices(needle).nth(nth).unwrap_or_else(|| {
        panic!("no occurrence {nth} of {needle:?} in the fixture")
      });

    Span::new(Arc::from("test.px"), start, start + needle.len())
  }

  fn part(&self, needle: &str, nth: usize, skip: usize, len: usize) -> Span {
    let start = self.at(needle, nth).start + skip;

    Span::new(Arc::from("test.px"), start, start + len)
  }

  fn cover(&self, from: (&str, usize), to: (&str, usize)) -> Span {
    self.at(from.0, from.1).join(&self.at(to.0, to.1))
  }

  fn name(&self, needle: &str, nth: usize) -> Name {
    Name { text: needle.to_string(), span: self.at(needle, nth) }
  }

  fn var(&self, needle: &str, nth: usize) -> Expr {
    Expr::Var(Var { span: self.at(needle, nth), name: self.name(needle, nth) })
  }

  fn type_ref(&self, needle: &str, nth: usize) -> TypeRef {
    TypeRef {
      span: self.at(needle, nth),
      name: self.name(needle, nth),
      args: vec![],
    }
  }

  fn block(&self, needle: &str, nth: usize, result: Expr) -> Block {
    Block {
      span: self.at(needle, nth),
      stmts: vec![],
      result: Box::new(result),
    }
  }
}

fn text(span: Span, raw: &str) -> StringPart {
  StringPart::Text(StringText {
    span,
    raw: raw.to_string(),
    value: raw.to_string(),
  })
}

const EVERY_KIND: &str = r#"{
  let Some(alpha, -1): Tee = function(beta: Int) -> Int / {Db} { beta }
  let {kay: _, ..} = {..base, zed: 1.5e-3}
  gee(alpha) |> aitch
  if ex + bee { "s #{alpha} t" } else if cee { dee.eff } else { match pee { 1_000 => !true, wu => wu } }
}"#;

fn let_ctor(src: &Src) -> Stmt {
  let lambda = Lambda {
    span: src.cover(("function", 0), ("{ beta }", 0)),
    params: vec![Param {
      span: src.at("beta: Int", 0),
      name: src.name("beta", 0),
      pattern: None,
      ty: Some(TypeExpr::Ref(src.type_ref("Int", 0))),
    }],
    return_type: Some(TypeExpr::Ref(src.type_ref("Int", 1))),
    effects: Some(EffectRow {
      span: src.at("/ {Db}", 0),
      entries: vec![src.type_ref("Db", 0)],
      tail: None,
    }),
    body: Box::new(src.block("{ beta }", 0, src.var("beta", 1))),
  };

  Stmt::Let(LetStmt {
    span: src.cover(("let", 0), ("{ beta }", 0)),
    pattern: Pattern::Ctor(PCtor {
      span: src.at("Some(alpha, -1)", 0),
      name: src.name("Some", 0),
      args: vec![
        Pattern::Var(PVar {
          span: src.at("alpha", 0),
          name: src.name("alpha", 0),
        }),
        Pattern::Lit(PLit {
          span: src.at("-1", 0),
          lit: PatLit::Int(IntLit {
            span: src.at("1", 0),
            raw: "1".to_string(),
          }),
          negative: true,
        }),
      ],
    }),
    ty: Some(TypeExpr::Ref(src.type_ref("Tee", 0))),
    value: Expr::Lambda(Box::new(lambda)),
  })
}

fn let_record(src: &Src) -> Stmt {
  Stmt::Let(LetStmt {
    span: src.cover(("let", 1), ("1.5e-3}", 0)),
    pattern: Pattern::Record(PRecord {
      span: src.at("{kay: _, ..}", 0),
      fields: vec![PField {
        span: src.at("kay: _", 0),
        name: src.name("kay", 0),
        pattern: Pattern::Wildcard(PWildcard { span: src.at("_", 0) }),
      }],
      open: true,
    }),
    ty: None,
    value: Expr::Record(RecordLit {
      span: src.at("{..base, zed: 1.5e-3}", 0),
      spread: Some(Box::new(src.var("base", 0))),
      fields: vec![FieldInit {
        span: src.at("zed: 1.5e-3", 0),
        name: src.name("zed", 0),
        value: Expr::Float(FloatLit {
          span: src.at("1.5e-3", 0),
          raw: "1.5e-3".to_string(),
        }),
      }],
    }),
  })
}

fn pipe_stmt(src: &Src) -> Stmt {
  Stmt::Expr(ExprStmt {
    span: src.at("gee(alpha) |> aitch", 0),
    expr: Expr::Pipe(Pipe {
      span: src.at("gee(alpha) |> aitch", 0),
      left: Box::new(Expr::Call(Call {
        span: src.at("gee(alpha)", 0),
        callee: Box::new(src.var("gee", 0)),
        args: vec![src.var("alpha", 1)],
      })),
      right: Box::new(src.var("aitch", 0)),
    }),
  })
}

fn if_expr(src: &Src) -> Expr {
  let end = ("wu } }", 0);

  let string = Expr::String(StringLit {
    span: src.at(r#""s #{alpha} t""#, 0),
    parts: vec![
      text(src.part("s #", 0, 0, 2), "s "),
      StringPart::Interp(StringInterp {
        span: src.at("#{alpha}", 0),
        expr: Box::new(src.var("alpha", 2)),
      }),
      text(src.part(" t\"", 0, 0, 2), " t"),
    ],
  });

  let arms = vec![
    MatchArm {
      span: src.at("1_000 => !true", 0),
      rows: vec![vec![Pattern::Lit(PLit {
        span: src.at("1_000", 0),
        lit: PatLit::Int(IntLit {
          span: src.at("1_000", 0),
          raw: "1_000".to_string(),
        }),
        negative: false,
      })]],
      guard: None,
      body: Expr::Unary(Unary {
        span: src.at("!true", 0),
        op: UnaryOp::Not,
        operand: Box::new(Expr::Bool(BoolLit {
          span: src.at("true", 0),
          value: true,
        })),
      }),
    },
    MatchArm {
      span: src.at("wu => wu", 0),
      rows: vec![vec![Pattern::Var(PVar {
        span: src.at("wu", 0),
        name: src.name("wu", 0),
      })]],
      guard: None,
      body: src.var("wu", 1),
    },
  ];

  let matched = Expr::Match(Match {
    span: src.at("match pee { 1_000 => !true, wu => wu }", 0),
    subjects: vec![src.var("pee", 0)],
    arms,
  });

  let else_if = If {
    span: src.cover(("if cee", 0), end),
    cond: Box::new(src.var("cee", 0)),
    then_branch: Box::new(src.block(
      "{ dee.eff }",
      0,
      Expr::Field(FieldAccess {
        span: src.at("dee.eff", 0),
        target: Box::new(src.var("dee", 0)),
        field: src.name("eff", 0),
      }),
    )),
    else_branch: Some(Box::new(Else::Block(src.block(
      "{ match pee { 1_000 => !true, wu => wu } }",
      0,
      matched,
    )))),
  };

  Expr::If(If {
    span: src.cover(("if ex", 0), end),
    cond: Box::new(Expr::Binary(Binary {
      span: src.at("ex + bee", 0),
      op: BinaryOp::Add,
      op_span: src.at("+", 0),
      left: Box::new(src.var("ex", 0)),
      right: Box::new(src.var("bee", 0)),
    })),
    then_branch: Box::new(src.block(r#"{ "s #{alpha} t" }"#, 0, string)),
    else_branch: Some(Box::new(Else::If(else_if))),
  })
}

fn every_kind() -> Block {
  let src = Src(EVERY_KIND);

  Block {
    span: src.all(),
    stmts: vec![let_ctor(&src), let_record(&src), pipe_stmt(&src)],
    result: Box::new(if_expr(&src)),
  }
}

fn walk<'a>(node: NodeRef<'a>, out: &mut Vec<NodeRef<'a>>) {
  out.push(node);

  for child in children(node) {
    walk(child, out);
  }
}

fn all_nodes(block: &Block) -> Vec<NodeRef<'_>> {
  let mut out = Vec::new();
  walk(block.as_node(), &mut out);
  out
}

#[test]
fn skipped_body_is_gone() {
  fn body_is_a_block(f: &FnDecl) -> &Block {
    &f.body
  }

  let _ = body_is_a_block;
}

#[test]
fn noderef_covers_new_kinds() {
  let block = every_kind();
  let kinds: BTreeSet<_> =
    all_nodes(&block).iter().map(NodeRef::kind).collect();

  for kind in [
    "IntLit",
    "FloatLit",
    "BoolLit",
    "StringLit",
    "StringText",
    "StringInterp",
    "Var",
    "FieldAccess",
    "Call",
    "Pipe",
    "Binary",
    "Unary",
    "RecordLit",
    "FieldInit",
    "Lambda",
    "Block",
    "LetStmt",
    "ExprStmt",
    "If",
    "Match",
    "MatchArm",
    "PWildcard",
    "PVar",
    "PLit",
    "PCtor",
    "PRecord",
    "PField",
  ] {
    assert!(kinds.contains(kind), "{kind} is not reachable as a NodeRef");
  }
}

#[test]
fn invalid_nodes_have_kinds() {
  let at = Span::empty(Arc::from("test.px"), 0);
  let expr = Expr::Invalid(InvalidExpr { span: at.clone() });
  let pattern =
    Pattern::Invalid(polar_compiler::syntax::ast::InvalidPattern { span: at });

  assert_eq!(expr.as_node().kind(), "InvalidExpr");
  assert_eq!(pattern.as_node().kind(), "InvalidPattern");
}

#[test]
fn plain_string_has_one_part() {
  let src = Src(r#""abc""#);
  let lit =
    StringLit { span: src.all(), parts: vec![text(src.at("abc", 0), "abc")] };

  assert_eq!(
    dump_node_shape(lit.as_node()),
    "(StringLit\n  (StringText raw=\"abc\" value=\"abc\"))\n"
  );
}

#[test]
fn empty_string_has_no_parts() {
  let lit = StringLit { span: Src(r#""""#).all(), parts: vec![] };

  assert!(children(lit.as_node()).is_empty());
  assert_eq!(dump_node_shape(lit.as_node()), "(StringLit)\n");
}

#[test]
fn interpolated_string_is_one_node() {
  let block = every_kind();
  let Expr::If(outer) = &*block.result else { panic!("expected an if") };
  let Expr::String(lit) = &*outer.then_branch.result else {
    panic!("expected a string")
  };

  let kinds: Vec<_> =
    children(lit.as_node()).iter().map(NodeRef::kind).collect();

  assert_eq!(kinds, ["StringText", "StringInterp", "StringText"]);
}

#[test]
fn interp_span_covers_delimiters() {
  let src = Src(r##""#{x}""##);
  let interp =
    StringInterp { span: src.at("#{x}", 0), expr: Box::new(src.var("x", 0)) };

  assert_eq!(&src.0[interp.span.start..interp.span.end], "#{x}");
}

#[test]
fn literals_keep_raw() {
  let src = Src("1_000 1.5e-3");
  let int = IntLit { span: src.at("1_000", 0), raw: "1_000".to_string() };
  let float = FloatLit { span: src.at("1.5e-3", 0), raw: "1.5e-3".to_string() };

  assert_eq!(dump_node_shape(int.as_node()), "(IntLit raw=\"1_000\")\n");
  assert_eq!(dump_node_shape(float.as_node()), "(FloatLit raw=\"1.5e-3\")\n");
}

#[test]
fn binary_op_span_is_the_operator() {
  let src = Src("a + b");
  let binary = Binary {
    span: src.all(),
    op: BinaryOp::Add,
    op_span: src.at("+", 0),
    left: Box::new(src.var("a", 0)),
    right: Box::new(src.var("b", 0)),
  };

  assert_eq!(&src.0[binary.op_span.start..binary.op_span.end], "+");
  assert_eq!(&src.0[binary.span.start..binary.span.end], "a + b");
  assert_eq!(
    dump_node_shape(binary.as_node()),
    "(Binary +\n  (Var a)\n  (Var b))\n"
  );
}

#[test]
fn else_if_is_typed() {
  let block = every_kind();
  let Expr::If(outer) = &*block.result else { panic!("expected an if") };
  let Some(Else::If(inner)) = outer.else_branch.as_deref() else {
    panic!("expected `else if`")
  };

  assert!(matches!(inner.else_branch.as_deref(), Some(Else::Block(_))));
}

#[test]
fn children_are_ordered() {
  let block = every_kind();

  for node in all_nodes(&block) {
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

#[test]
fn parent_covers_children() {
  let block = every_kind();

  for node in all_nodes(&block) {
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
