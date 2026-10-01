use proptest::{
  collection::vec, prelude::*, sample::select, strategy::BoxedStrategy,
};
use std::{collections::BTreeSet, sync::Arc};

#[derive(Debug, Clone, PartialEq)]
pub enum GenType {
  Int,
  Float,
  Bool,
  String,
  Record(Vec<(String, GenType)>),
  List(Box<GenType>),
  Fn(Vec<GenType>, Box<GenType>),
}

impl GenType {
  #[must_use]
  pub fn source(&self) -> String {
    match self {
      GenType::Int => "Int".to_string(),
      GenType::Float => "Float".to_string(),
      GenType::Bool => "Bool".to_string(),
      GenType::String => "String".to_string(),
      GenType::Record(fields) => {
        let fields: Vec<String> =
          fields.iter().map(|(n, t)| format!("{n}: {}", t.source())).collect();

        format!("{{ {} }}", fields.join(", "))
      }
      GenType::List(item) => format!("List<{}>", item.source()),
      GenType::Fn(params, ret) => {
        let params: Vec<String> = params.iter().map(GenType::source).collect();

        format!("function({}) -> {}", params.join(", "), ret.source())
      }
    }
  }
}

const PRIMITIVES: [GenType; 4] =
  [GenType::Int, GenType::Float, GenType::Bool, GenType::String];

fn primitive() -> BoxedStrategy<GenType> {
  select(PRIMITIVES.to_vec()).boxed()
}

fn record_of(inner: BoxedStrategy<GenType>) -> BoxedStrategy<GenType> {
  (vec(inner, 3), proptest::bits::u8::between(0, 3))
    .prop_filter("a record has at least one field", |(_, mask)| *mask != 0)
    .prop_map(|(types, mask)| {
      let fields = ["a", "b", "c"]
        .iter()
        .zip(types)
        .enumerate()
        .filter(|(i, _)| mask & (1 << i) != 0)
        .map(|(_, (n, t))| ((*n).to_string(), t))
        .collect();

      GenType::Record(fields)
    })
    .boxed()
}

pub fn gen_type() -> BoxedStrategy<GenType> {
  primitive()
    .prop_recursive(3, 12, 3, |inner| {
      prop_oneof![
        record_of(inner.clone()),
        inner.clone().prop_map(|t| GenType::List(Box::new(t))),
        (vec(inner.clone(), 1..=2), inner)
          .prop_map(|(ps, r)| GenType::Fn(ps, Box::new(r))),
      ]
    })
    .boxed()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Ctx {
  Free,
  Forced,
  Annotated,
}

#[derive(Debug, Clone)]
pub enum Node {
  Text(String),
  Seq(Vec<Node>),
  Kind(&'static str, Box<Node>),
  Int { value: u32, ctx: Ctx },
  Record { fields: Vec<(String, Node)>, ctx: Ctx },
  Call { callee: String, args: Vec<Node> },
}

fn text(s: impl Into<String>) -> Node {
  Node::Text(s.into())
}

fn seq(nodes: Vec<Node>) -> Node {
  Node::Seq(nodes)
}

fn kind(name: &'static str, node: Node) -> Node {
  Node::Kind(name, Box::new(node))
}

fn paren(node: Node) -> Node {
  seq(vec![text("("), node, text(")")])
}

impl Node {
  #[must_use]
  pub fn render(&self) -> String {
    let mut out = String::new();

    self.write(&mut out);
    out
  }

  fn write(&self, out: &mut String) {
    match self {
      Node::Text(s) => out.push_str(s),
      Node::Seq(nodes) => nodes.iter().for_each(|n| n.write(out)),
      Node::Kind(_, node) => node.write(out),
      Node::Int { value, .. } => out.push_str(&value.to_string()),
      Node::Record { fields, .. } if fields.is_empty() => out.push_str("{}"),
      Node::Record { fields, .. } => {
        out.push_str("{ ");

        for (i, (name, value)) in fields.iter().enumerate() {
          if i > 0 {
            out.push_str(", ");
          }

          out.push_str(name);
          out.push_str(": ");
          value.write(out);
        }

        out.push_str(" }");
      }
      Node::Call { callee, args } => {
        out.push_str(callee);
        out.push('(');

        for (i, arg) in args.iter().enumerate() {
          if i > 0 {
            out.push_str(", ");
          }

          arg.write(out);
        }

        out.push(')');
      }
    }
  }

  pub fn kinds(&self, out: &mut BTreeSet<&'static str>) {
    match self {
      Node::Text(_) | Node::Int { .. } => {}
      Node::Seq(nodes) | Node::Call { args: nodes, .. } => {
        for node in nodes {
          node.kinds(out);
        }
      }
      Node::Kind(name, node) => {
        out.insert(name);
        node.kinds(out);
      }
      Node::Record { fields, .. } => {
        for (_, node) in fields {
          node.kinds(out);
        }
      }
    }
  }

  fn is_site(&self, mutation: Mutation) -> bool {
    match (self, mutation) {
      (Node::Int { ctx, .. }, Mutation::IntToString | Mutation::IntToFloat) => {
        *ctx != Ctx::Free
      }
      (Node::Record { fields, ctx }, Mutation::DropField) => {
        *ctx == Ctx::Annotated && !fields.is_empty()
      }
      (Node::Call { .. }, Mutation::ExtraArgument) => true,
      _ => false,
    }
  }

  fn count(&self, mutation: Mutation) -> usize {
    let own = usize::from(self.is_site(mutation));

    own
      + match self {
        Node::Text(_) | Node::Int { .. } => 0,
        Node::Seq(nodes) => nodes.iter().map(|n| n.count(mutation)).sum(),
        Node::Kind(_, node) => node.count(mutation),
        Node::Record { fields, .. } => {
          fields.iter().map(|(_, n)| n.count(mutation)).sum()
        }
        Node::Call { args, .. } => args.iter().map(|n| n.count(mutation)).sum(),
      }
  }

  fn mutate(&self, mutation: Mutation, index: &mut usize) -> Node {
    if self.is_site(mutation) {
      if *index == 0 {
        *index = usize::MAX;

        return match (self, mutation) {
          (_, Mutation::IntToString) => text("\"x\""),
          (_, Mutation::IntToFloat) => text("1.5"),
          (Node::Record { fields, ctx }, Mutation::DropField) => {
            Node::Record { fields: fields[1..].to_vec(), ctx: *ctx }
          }
          (Node::Call { callee, args }, Mutation::ExtraArgument) => {
            let mut args = args.clone();

            args.push(text("1"));
            Node::Call { callee: callee.clone(), args }
          }
          _ => self.clone(),
        };
      }

      *index = index.saturating_sub(1);
    }

    match self {
      Node::Text(_) | Node::Int { .. } => self.clone(),
      Node::Seq(nodes) => {
        Node::Seq(nodes.iter().map(|n| n.mutate(mutation, index)).collect())
      }
      Node::Kind(name, node) => {
        Node::Kind(name, Box::new(node.mutate(mutation, index)))
      }
      Node::Record { fields, ctx } => Node::Record {
        fields: fields
          .iter()
          .map(|(f, n)| (f.clone(), n.mutate(mutation, index)))
          .collect(),
        ctx: *ctx,
      },
      Node::Call { callee, args } => Node::Call {
        callee: callee.clone(),
        args: args.iter().map(|n| n.mutate(mutation, index)).collect(),
      },
    }
  }
}

#[derive(Debug, Clone)]
struct Var {
  name: String,
  ty: GenType,
  open: bool,
}

type Scope = Arc<Vec<Var>>;

fn with(scope: &Scope, name: &str, ty: &GenType) -> Scope {
  let mut vars = (**scope).clone();

  vars.push(Var { name: name.to_string(), ty: ty.clone(), open: false });
  Arc::new(vars)
}

fn lazy(f: impl Fn() -> BoxedStrategy<Node> + 'static) -> BoxedStrategy<Node> {
  Just(()).prop_flat_map(move |()| f()).boxed()
}

fn all(strategies: Vec<BoxedStrategy<Node>>) -> BoxedStrategy<Vec<Node>> {
  strategies.into_iter().fold(Just(Vec::new()).boxed(), |acc, s| {
    (acc, s)
      .prop_map(|(mut nodes, n)| {
        nodes.push(n);
        nodes
      })
      .boxed()
  })
}

const WORDS: &[&str] = &["", "a", "polar", "Hello", "x y"];

fn literal(
  ty: &GenType,
  scope: &Scope,
  depth: u32,
  ctx: Ctx,
  inline: bool,
) -> BoxedStrategy<Node> {
  let d = depth.saturating_sub(1);

  match ty {
    GenType::Int => (0u32..100)
      .prop_map(move |value| kind("int", Node::Int { value, ctx }))
      .boxed(),
    GenType::Float => (0u32..100, 0u32..10)
      .prop_map(|(a, b)| kind("float", text(format!("{a}.{b}"))))
      .boxed(),
    GenType::Bool => {
      select(vec!["true", "false"]).prop_map(|b| kind("bool", text(b))).boxed()
    }
    GenType::String => select(WORDS.to_vec())
      .prop_map(|w| kind("string", text(format!("\"{w}\""))))
      .boxed(),
    GenType::Record(fields) => {
      let names: Vec<String> = fields.iter().map(|(n, _)| n.clone()).collect();
      let values = all(
        fields.iter().map(|(_, t)| expr(t, scope, d, ctx, inline)).collect(),
      );

      values
        .prop_map(move |values| {
          kind(
            "record",
            Node::Record {
              fields: names.iter().cloned().zip(values).collect(),
              ctx,
            },
          )
        })
        .boxed()
    }
    GenType::List(item) => items(item, scope, d, ctx, inline, 0..=2),
    GenType::Fn(params, ret) => lambda(params, ret, scope, d, inline),
  }
}

fn items(
  item: &GenType,
  scope: &Scope,
  depth: u32,
  ctx: Ctx,
  inline: bool,
  count: std::ops::RangeInclusive<usize>,
) -> BoxedStrategy<Node> {
  let (item, scope) = (item.clone(), scope.clone());

  count
    .prop_flat_map(move |n| {
      let each = if n >= 2 { Ctx::Forced } else { ctx.min(Ctx::Forced) };

      vec(expr(&item, &scope, depth, each, inline), n)
    })
    .prop_map(|items| kind("list", list(items, None)))
    .boxed()
}

fn list(items: Vec<Node>, tail: Option<Node>) -> Node {
  let mut nodes = vec![text("[")];

  for (i, item) in items.into_iter().enumerate() {
    if i > 0 {
      nodes.push(text(", "));
    }

    nodes.push(item);
  }

  if let Some(tail) = tail {
    if nodes.len() > 1 {
      nodes.push(text(", "));
    }

    nodes.push(text(".."));
    nodes.push(tail);
  }

  nodes.push(text("]"));
  seq(nodes)
}

fn lambda(
  params: &[GenType],
  ret: &GenType,
  scope: &Scope,
  depth: u32,
  inline: bool,
) -> BoxedStrategy<Node> {
  let names: Vec<String> =
    (0..params.len()).map(|i| format!("p{}_{i}", scope.len())).collect();
  let mut inner = scope.clone();

  for (name, ty) in names.iter().zip(params) {
    inner = with(&inner, name, ty);
  }

  let header: Vec<String> = names
    .iter()
    .zip(params)
    .map(|(n, t)| format!("{n}: {}", t.source()))
    .collect();
  let head = format!("function({}) -> {} {{ ", header.join(", "), ret.source());

  expr(ret, &inner, depth, Ctx::Annotated, inline)
    .prop_map(move |body| {
      kind("lambda", seq(vec![text(head.clone()), body, text(" }")]))
    })
    .boxed()
}

fn vars_of<'s>(scope: &'s Scope, ty: &GenType) -> Vec<&'s Var> {
  scope.iter().filter(|v| !v.open && v.ty == *ty).collect()
}

type Options = Vec<(u32, BoxedStrategy<Node>)>;

fn expr(
  ty: &GenType,
  scope: &Scope,
  depth: u32,
  ctx: Ctx,
  inline: bool,
) -> BoxedStrategy<Node> {
  let mut options: Options = vec![(3, literal(ty, scope, depth, ctx, inline))];

  from_scope(ty, scope, depth, inline, &mut options);

  if depth > 0 {
    let d = depth - 1;

    options.push((1, if_expr(ty, scope, d, inline)));

    if !inline {
      options.push((1, let_block(ty, scope, d, ctx)));
      options.push((1, lambda_call(ty, scope, d)));
    }

    options.extend(specific(ty, scope, d, ctx, inline));
  }

  prop::strategy::Union::new_weighted(options).boxed()
}

fn from_scope(
  ty: &GenType,
  scope: &Scope,
  depth: u32,
  inline: bool,
  options: &mut Options,
) {
  let names: Vec<String> =
    vars_of(scope, ty).iter().map(|v| v.name.clone()).collect();

  if !names.is_empty() {
    options.push((4, select(names).prop_map(|n| kind("var", text(n))).boxed()));
  }

  let accesses: Vec<String> = scope
    .iter()
    .filter_map(|v| match &v.ty {
      GenType::Record(fields) => fields
        .iter()
        .find(|(_, t)| t == ty)
        .map(|(f, _)| format!("{}.{f}", v.name)),
      _ => None,
    })
    .collect();

  if !accesses.is_empty() {
    options
      .push((3, select(accesses).prop_map(|a| kind("field", text(a))).boxed()));
  }

  let callees: Vec<(String, Vec<GenType>)> = scope
    .iter()
    .filter_map(|v| match &v.ty {
      GenType::Fn(params, ret) if **ret == *ty => {
        Some((v.name.clone(), params.clone()))
      }
      _ => None,
    })
    .collect();

  if !callees.is_empty() && depth > 0 {
    let scope = scope.clone();

    options.push((
      2,
      select(callees)
        .prop_flat_map(move |(callee, params)| {
          all(
            params
              .iter()
              .map(|p| expr(p, &scope, depth - 1, Ctx::Annotated, inline))
              .collect(),
          )
          .prop_map(move |args| {
            kind("call", Node::Call { callee: callee.clone(), args })
          })
        })
        .boxed(),
    ));
  }
}

fn if_expr(
  ty: &GenType,
  scope: &Scope,
  d: u32,
  inline: bool,
) -> BoxedStrategy<Node> {
  let (ty, scope) = (ty.clone(), scope.clone());

  lazy(move || {
    (
      expr(&GenType::Bool, &scope, d, Ctx::Forced, inline),
      expr(&ty, &scope, d, Ctx::Forced, inline),
      expr(&ty, &scope, d, Ctx::Forced, inline),
    )
      .prop_map(|(c, t, e)| {
        kind(
          "if",
          seq(vec![
            text("if "),
            paren(c),
            text(" { "),
            t,
            text(" } else { "),
            e,
            text(" }"),
          ]),
        )
      })
      .boxed()
  })
}

fn let_block(
  ty: &GenType,
  scope: &Scope,
  d: u32,
  ctx: Ctx,
) -> BoxedStrategy<Node> {
  let (outer_ty, outer_scope) = (ty.clone(), scope.clone());

  lazy(move || {
    let (ty, scope) = (outer_ty.clone(), outer_scope.clone());

    primitive()
      .prop_flat_map(move |value_ty| {
        let name = format!("v{}", scope.len());
        let inner = with(&scope, &name, &value_ty);

        (
          expr(&value_ty, &scope, d, Ctx::Free, false),
          expr(&ty, &inner, d, ctx, false),
        )
          .prop_map(move |(value, body)| {
            kind(
              "let",
              seq(vec![
                text(format!("{{\nlet {name} = ")),
                value,
                text("\n"),
                body,
                text("\n}"),
              ]),
            )
          })
      })
      .boxed()
  })
}

fn lambda_call(ty: &GenType, scope: &Scope, d: u32) -> BoxedStrategy<Node> {
  let (outer_ty, outer_scope) = (ty.clone(), scope.clone());

  lazy(move || {
    let (ty, scope) = (outer_ty.clone(), outer_scope.clone());

    primitive()
      .prop_flat_map(move |param| {
        let name = format!("f{}", scope.len());
        let fn_ty = GenType::Fn(vec![param.clone()], Box::new(ty.clone()));
        let inner = with(&scope, &name, &fn_ty);

        (
          lambda(std::slice::from_ref(&param), &ty, &scope, d, false),
          expr(&param, &inner, d, Ctx::Annotated, false),
        )
          .prop_map(move |(lam, arg)| {
            seq(vec![
              text(format!("{{\nlet {name} = ")),
              lam,
              text("\n"),
              kind(
                "call",
                Node::Call { callee: name.clone(), args: vec![arg] },
              ),
              text("\n}"),
            ])
          })
      })
      .boxed()
  })
}

fn binary(
  name: &'static str,
  op: &'static str,
  left: BoxedStrategy<Node>,
  right: BoxedStrategy<Node>,
) -> BoxedStrategy<Node> {
  (left, right)
    .prop_map(move |(l, r)| {
      kind(name, seq(vec![paren(l), text(format!(" {op} ")), paren(r)]))
    })
    .boxed()
}

fn builtin(
  name: &'static str,
  args: Vec<BoxedStrategy<Node>>,
) -> BoxedStrategy<Node> {
  all(args)
    .prop_map(move |args| {
      let mut nodes = vec![text(format!("{name}("))];

      for (i, arg) in args.into_iter().enumerate() {
        if i > 0 {
          nodes.push(text(", "));
        }

        nodes.push(arg);
      }

      nodes.push(text(")"));
      kind("builtin", seq(nodes))
    })
    .boxed()
}

fn specific(
  ty: &GenType,
  scope: &Scope,
  d: u32,
  ctx: Ctx,
  inline: bool,
) -> Options {
  match ty {
    GenType::Int | GenType::Float => numeric(ty, scope, d, ctx, inline),
    GenType::Bool => boolean(scope, d, inline),
    GenType::String => strings(scope, d, inline),
    GenType::Record(fields) => update(ty, fields, scope, d, ctx, inline),
    GenType::List(item) => lists(ty, item, scope, d, ctx, inline),
    GenType::Fn(params, ret) => {
      vec![(2, lambda(params, ret, scope, d, inline))]
    }
  }
}

fn numeric(
  ty: &GenType,
  scope: &Scope,
  d: u32,
  ctx: Ctx,
  inline: bool,
) -> Options {
  let sub = |c: Ctx| expr(ty, scope, d, c, inline);
  let name = if *ty == GenType::Int { "arith_int" } else { "arith_float" };
  let mut out: Options = ["+", "-", "*", "/", "%"]
    .into_iter()
    .map(|op| (1, binary(name, op, sub(Ctx::Forced), sub(Ctx::Forced))))
    .collect();

  out.push((
    1,
    sub(ctx)
      .prop_map(move |e| kind(name, paren(seq(vec![text("-"), paren(e)]))))
      .boxed(),
  ));
  out.push((
    1,
    if *ty == GenType::Int {
      builtin(
        "String.length",
        vec![expr(&GenType::String, scope, d, Ctx::Annotated, inline)],
      )
    } else {
      builtin("Float.abs", vec![sub(Ctx::Annotated)])
    },
  ));
  out
}

fn boolean(scope: &Scope, d: u32, inline: bool) -> Options {
  let sub = |t: &GenType| expr(t, scope, d, Ctx::Forced, inline);
  let mut out = Options::new();

  for t in [GenType::Int, GenType::Float, GenType::String] {
    for op in ["<", "<=", ">", ">="] {
      out.push((1, binary("compare", op, sub(&t), sub(&t))));
    }
  }

  for t in [GenType::Int, GenType::String, GenType::Bool] {
    for op in ["==", "!="] {
      out.push((1, binary("equality", op, sub(&t), sub(&t))));
    }
  }

  for op in ["&&", "||"] {
    out
      .push((2, binary("logic", op, sub(&GenType::Bool), sub(&GenType::Bool))));
  }

  out.push((
    1,
    sub(&GenType::Bool)
      .prop_map(|e| kind("logic", seq(vec![text("!"), paren(e)])))
      .boxed(),
  ));

  let string = || expr(&GenType::String, scope, d, Ctx::Annotated, inline);

  out.push((1, builtin("String.contains", vec![string(), string()])));
  out
}

fn strings(scope: &Scope, d: u32, inline: bool) -> Options {
  let inner_scope = scope.clone();
  let interp = select(PRIMITIVES.to_vec())
    .prop_flat_map(move |t| {
      let depth = if t == GenType::String { 0 } else { d };

      expr(&t, &inner_scope, depth, Ctx::Free, true)
    })
    .prop_map(|e| kind("interp", seq(vec![text("\"#{"), e, text("}\"")])))
    .boxed();

  vec![
    (3, interp),
    (
      1,
      builtin(
        "String.uppercase",
        vec![expr(&GenType::String, scope, d, Ctx::Annotated, inline)],
      ),
    ),
    (
      1,
      builtin(
        "Int.to_string",
        vec![expr(&GenType::Int, scope, d, Ctx::Annotated, inline)],
      ),
    ),
  ]
}

fn update(
  ty: &GenType,
  fields: &[(String, GenType)],
  scope: &Scope,
  d: u32,
  ctx: Ctx,
  inline: bool,
) -> Options {
  let bases: Vec<String> =
    vars_of(scope, ty).iter().map(|v| v.name.clone()).collect();

  if bases.is_empty() {
    return Options::new();
  }

  let (fields, scope) = (fields.to_vec(), scope.clone());

  vec![(
    3,
    (select(bases), select(fields))
      .prop_flat_map(move |(base, (field, field_ty))| {
        expr(&field_ty, &scope, d, ctx, inline).prop_map(move |value| {
          kind(
            "update",
            seq(vec![
              text(format!("{{ ..{base}, {field}: ")),
              value,
              text(" }"),
            ]),
          )
        })
      })
      .boxed(),
  )]
}

fn lists(
  ty: &GenType,
  item: &GenType,
  scope: &Scope,
  d: u32,
  ctx: Ctx,
  inline: bool,
) -> Options {
  let tails: Vec<String> =
    vars_of(scope, ty).iter().map(|v| v.name.clone()).collect();
  let mut out: Options = vec![(2, items(item, scope, d, ctx, inline, 1..=3))];

  if !tails.is_empty() {
    out.push((
      2,
      (expr(item, scope, d, Ctx::Forced, inline), select(tails))
        .prop_map(|(head, tail)| {
          kind("list", list(vec![head], Some(text(tail))))
        })
        .boxed(),
    ));
  }

  out
}

#[derive(Debug, Clone)]
pub struct Param {
  pub name: String,
  pub ty: GenType,
  pub open: bool,
}

impl Param {
  fn source(&self) -> String {
    match (&self.ty, self.open) {
      (GenType::Record(fields), true) => {
        let fields: Vec<String> =
          fields.iter().map(|(n, t)| format!("{n}: {}", t.source())).collect();

        format!("{{ {} | r }}", fields.join(", "))
      }
      (ty, _) => ty.source(),
    }
  }
}

#[derive(Debug, Clone)]
pub struct Program {
  pub params: Vec<Param>,
  pub ret: GenType,
  pub body: Node,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mutation {
  IntToString,
  IntToFloat,
  DropField,
  ExtraArgument,
  ReturnAnnotation,
}

pub const MUTATIONS: [Mutation; 5] = [
  Mutation::IntToString,
  Mutation::IntToFloat,
  Mutation::DropField,
  Mutation::ExtraArgument,
  Mutation::ReturnAnnotation,
];

impl Mutation {
  #[must_use]
  pub fn expected(self) -> &'static [&'static str] {
    match self {
      Mutation::IntToString => &["POLAR0501", "POLAR0512"],
      Mutation::IntToFloat | Mutation::ReturnAnnotation => &["POLAR0501"],
      Mutation::DropField => &["POLAR0502"],
      Mutation::ExtraArgument => &["POLAR0508"],
    }
  }
}

pub const PRELUDE: &str = "types\n  List<a> = Nil | Cons(a, List<a>)\n\n";

impl Program {
  fn signature(&self, ret: &str) -> String {
    let params: Vec<String> = self
      .params
      .iter()
      .map(|p| format!("{}: {}", p.name, p.source()))
      .collect();

    format!("gen({}) -> {ret}", params.join(", "))
  }

  fn module(&self, ret: &str, body: &str, extra: &str) -> String {
    format!(
      "{PRELUDE}functions\n  {} {{\n{body}\n  }}\n{extra}",
      self.signature(ret)
    )
  }

  #[must_use]
  pub fn source(&self) -> String {
    self.module(&self.ret.source(), &self.body.render(), "")
  }

  #[must_use]
  pub fn expected_type(&self) -> String {
    let params: Vec<String> = self.params.iter().map(Param::source).collect();

    format!("function({}) -> {}", params.join(", "), self.ret.source())
  }

  #[must_use]
  pub fn kinds(&self) -> BTreeSet<&'static str> {
    let mut out = BTreeSet::new();

    self.body.kinds(&mut out);
    out
  }

  #[must_use]
  pub fn sites(&self, mutation: Mutation) -> usize {
    match mutation {
      Mutation::ReturnAnnotation => 1,
      _ => self.body.count(mutation),
    }
  }

  #[must_use]
  pub fn mutated(&self, mutation: Mutation, site: usize) -> String {
    if mutation == Mutation::ReturnAnnotation {
      let other = PRIMITIVES
        .iter()
        .find(|p| **p != self.ret)
        .map_or_else(|| "Int".to_string(), GenType::source);

      return self.module(&other, &self.body.render(), "");
    }

    let mut index = site;
    let body = self.body.mutate(mutation, &mut index);

    self.module(&self.ret.source(), &body.render(), "")
  }

  #[must_use]
  pub fn with_main(&self, args: &[Node]) -> String {
    let args: Vec<String> = args.iter().map(Node::render).collect();
    let main = format!(
      "\n  main() {{\n    let _ = gen({})\n    Log.info(\"ok\")\n  }}\n\nexports\n  main\n",
      args.join(", ")
    );

    self.module(&self.ret.source(), &self.body.render(), &main)
  }
}

fn param() -> BoxedStrategy<GenType> {
  gen_type()
}

pub fn program() -> BoxedStrategy<Program> {
  let open = proptest::option::of(record_of(primitive()));

  (vec(param(), 0..=3), open, gen_type(), any::<bool>())
    .prop_flat_map(|(types, open, ret, reuse)| {
      let ret = match types.iter().find(|t| matches!(t, GenType::Record(_))) {
        Some(record) if reuse => record.clone(),
        _ => ret,
      };
      let mut params: Vec<Param> = types
        .into_iter()
        .enumerate()
        .map(|(i, ty)| Param { name: format!("x{i}"), ty, open: false })
        .collect();

      if let Some(ty) = open {
        params.push(Param {
          name: format!("x{}", params.len()),
          ty,
          open: true,
        });
      }

      let scope: Scope = Arc::new(
        params
          .iter()
          .map(|p| Var { name: p.name.clone(), ty: p.ty.clone(), open: p.open })
          .collect(),
      );
      let body = expr(&ret, &scope, 3, Ctx::Annotated, false);

      (Just(params), Just(ret), body)
    })
    .prop_map(|(params, ret, body)| Program { params, ret, body })
    .boxed()
}

pub fn arguments(program: &Program) -> BoxedStrategy<Vec<Node>> {
  all(
    program
      .params
      .iter()
      .map(|p| {
        let value =
          literal(&p.ty, &Arc::new(Vec::new()), 1, Ctx::Annotated, true);

        if p.open {
          value
            .prop_map(|node| match node {
              Node::Kind(_, inner) => match *inner {
                Node::Record { mut fields, ctx } => {
                  fields.push(("z".to_string(), text("0")));
                  Node::Record { fields, ctx }
                }
                other => other,
              },
              other => other,
            })
            .boxed()
        } else {
          value
        }
      })
      .collect(),
  )
}
