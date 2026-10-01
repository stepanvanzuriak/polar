use crate::{
  backend::js::{
    ast::{
      ArrowBody, ArrowFn, Expr, FunctionDecl, Ident, Item, LiteralValue,
      MemberProp, ObjectProp, Program, Stmt, Switch, TemplateLit, Unary,
      UnaryOp, VarDecl, VarKind,
    },
    json,
    names::js_ident,
    precedence::{self, Prec},
  },
  shared::ice::ice,
  shared::source::Span,
};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PrintResult {
  pub code: String,
  pub mappings: Vec<Mapping>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mapping {
  pub gen_line: usize,
  pub gen_column: usize,
  pub origin: Option<Span>,
  pub name: Option<String>,
}

#[must_use]
pub fn print_program(program: &Program) -> PrintResult {
  let mut p = Printer::default();

  for (i, item) in program.items.iter().enumerate() {
    let both_imports =
      i > 0 && is_import(item) && is_import(&program.items[i - 1]);

    if i > 0 && !both_imports {
      p.newline();
    }

    p.item(item);
  }

  PrintResult { code: p.out, mappings: p.mappings }
}

#[must_use]
pub fn print_expr(e: &Expr) -> String {
  let mut p = Printer::default();

  p.expr(e, Prec::Assign);
  p.out
}

#[must_use]
pub fn print_stmt(s: &Stmt) -> String {
  let mut p = Printer::default();

  p.stmt(s);
  p.out
}

fn is_import(item: &Item) -> bool {
  matches!(item, Item::ImportNamespace(_) | Item::ImportNamed(_))
}

#[derive(Default)]
struct Printer {
  out: String,
  mappings: Vec<Mapping>,
  line: usize,
  column: usize,
  indent: usize,
  mapped: bool,
}

impl Printer {
  fn push(&mut self, s: &str) {
    for c in s.chars() {
      if c == '\n' {
        self.line += 1;
        self.column = 0;
        self.mapped = false;
      } else {
        self.column += c.len_utf16();
      }
    }

    self.out.push_str(s);
  }

  fn newline(&mut self) {
    self.push("\n");
  }

  fn line_start(&mut self) {
    let pad = "  ".repeat(self.indent);

    self.push(&pad);
  }

  fn start(&mut self, origin: Option<&Span>, name: Option<String>) {
    match origin {
      Some(origin) => {
        self.mappings.push(Mapping {
          gen_line: self.line,
          gen_column: self.column,
          origin: Some(origin.clone()),
          name,
        });
        self.mapped = true;
      }
      None if self.mapped => {
        self.mappings.push(Mapping {
          gen_line: self.line,
          gen_column: self.column,
          origin: None,
          name: None,
        });
        self.mapped = false;
      }
      None => {}
    }
  }

  fn item(&mut self, item: &Item) {
    match item {
      Item::ImportNamespace(import) => {
        self.start(import.origin.as_ref(), None);
        self.push("import * as ");
        self.ident(&import.local);
        self.push(" from ");
        self.push(&json::string(&import.source));
        self.push(";\n");
      }
      Item::ImportNamed(import) => {
        self.start(import.origin.as_ref(), None);
        self.push("import { ");

        for (i, (export, local)) in import.names.iter().enumerate() {
          if i > 0 {
            self.push(", ");
          }

          if *export != local.name {
            self.push(export);
            self.push(" as ");
          }

          self.ident(local);
        }

        self.push(" } from ");
        self.push(&json::string(&import.source));
        self.push(";\n");
      }
      Item::Function(f) => self.function(f),
      Item::Stmt(s) => self.stmt(s),
    }
  }

  fn function(&mut self, f: &FunctionDecl) {
    self.line_start();
    self.start(f.origin.as_ref(), None);

    if f.exported {
      self.push("export ");
    }
    if f.is_async {
      self.push("async ");
    }

    self.push("function ");
    self.ident(&f.name);
    self.params(&f.params);
    self.push(" ");
    self.body(&f.body);
    self.newline();
  }

  fn params(&mut self, params: &[Ident]) {
    self.push("(");

    for (i, param) in params.iter().enumerate() {
      if i > 0 {
        self.push(", ");
      }
      self.ident(param);
    }

    self.push(")");
  }

  fn body(&mut self, body: &[Stmt]) {
    self.push("{\n");
    self.indent += 1;

    for s in body {
      self.stmt(s);
    }

    self.indent -= 1;
    self.line_start();
    self.push("}");
  }

  fn stmt(&mut self, s: &Stmt) {
    self.line_start();
    self.start(s.origin(), None);

    match s {
      Stmt::Var(v) => self.var(v),
      Stmt::Expr(e) => {
        if starts_ambiguously(&e.expr) {
          self.push("(");
          self.expr(&e.expr, Prec::Assign);
          self.push(")");
        } else {
          self.expr(&e.expr, Prec::Assign);
        }
        self.push(";\n");
      }
      Stmt::Return(r) => {
        match &r.arg {
          Some(arg) => {
            self.push("return ");
            self.expr(arg, Prec::Assign);
          }
          None => self.push("return"),
        }
        self.push(";\n");
      }
      Stmt::If(i) => {
        self.if_chain(&i.test, &i.consequent, i.alternate.as_deref());
        self.newline();
      }
      Stmt::Block(b) => {
        self.body(&b.body);
        self.newline();
      }
      Stmt::Throw(t) => {
        self.push("throw ");
        self.expr(&t.arg, Prec::Assign);
        self.push(";\n");
      }
      Stmt::Switch(sw) => self.switch(sw),
      Stmt::Break(_) => self.push("break;\n"),
      Stmt::Continue(_) => self.push("continue;\n"),
      Stmt::While(w) => {
        self.push("while (");
        self.expr(&w.test, Prec::Assign);
        self.push(") ");
        self.body(&w.body);
        self.newline();
      }
      Stmt::Try(t) => {
        self.push("try ");
        self.body(&t.block);
        self.push(" catch (");
        self.ident(&t.param);
        self.push(") ");
        self.body(&t.handler);
        self.newline();
      }
    }
  }

  fn var(&mut self, v: &VarDecl) {
    if v.exported {
      self.push("export ");
    }

    self.push(match v.kind {
      VarKind::Const => "const ",
      VarKind::Let => "let ",
    });
    self.ident(&v.name);

    if let Some(init) = &v.init {
      self.push(" = ");
      self.expr(init, Prec::Assign);
    }

    self.push(";\n");
  }

  fn if_chain(
    &mut self,
    test: &Expr,
    consequent: &[Stmt],
    alternate: Option<&[Stmt]>,
  ) {
    self.push("if (");
    self.expr(test, Prec::Assign);
    self.push(") ");
    self.body(consequent);

    match alternate {
      Some([Stmt::If(inner)]) => {
        self.push(" else ");
        self.start(inner.origin.as_ref(), None);
        self.if_chain(
          &inner.test,
          &inner.consequent,
          inner.alternate.as_deref(),
        );
      }
      Some([]) | None => {}
      Some(alternate) => {
        self.push(" else ");
        self.body(alternate);
      }
    }
  }

  fn switch(&mut self, sw: &Switch) {
    self.push("switch (");
    self.expr(&sw.discriminant, Prec::Assign);
    self.push(") {\n");
    self.indent += 1;

    for case in &sw.cases {
      self.line_start();
      self.start(case.origin.as_ref(), None);
      self.push("case ");
      self.literal(&case.test.value);
      self.push(":\n");
      self.case_body(&case.body);
    }

    if let Some(default) = &sw.default {
      self.line_start();
      self.push("default:\n");
      self.case_body(default);
    }

    self.indent -= 1;
    self.line_start();
    self.push("}\n");
  }

  fn case_body(&mut self, body: &[Stmt]) {
    self.indent += 1;

    for s in body {
      self.stmt(s);
    }

    self.indent -= 1;
  }

  fn expr(&mut self, e: &Expr, min: Prec) {
    if precedence::of(e) < min {
      self.push("(");
      self.bare(e);
      self.push(")");
    } else {
      self.bare(e);
    }
  }

  fn bare(&mut self, e: &Expr) {
    match e {
      Expr::Ident(id) => self.ident(id),
      Expr::Literal(lit) => {
        self.start(lit.origin.as_ref(), None);
        self.literal(&lit.value);
      }
      Expr::TemplateLit(t) => self.template(t),
      Expr::ArrowFn(a) => self.arrow(a),
      Expr::Call(c) => {
        self.start(c.origin.as_ref(), None);
        self.callee(&c.callee);
        self.push("(");

        for (i, arg) in c.args.iter().enumerate() {
          if i > 0 {
            self.push(", ");
          }
          self.expr(arg, Prec::Assign);
        }

        self.push(")");
      }
      Expr::Member(m) => {
        self.start(m.origin.as_ref(), None);
        self.callee(&m.object);

        match &m.property {
          MemberProp::Static(name) => {
            self.push(".");
            self.start(name.origin.as_ref(), None);
            self.push(&name.name);
          }
          MemberProp::Computed(index) => {
            self.push("[");
            self.expr(index, Prec::Assign);
            self.push("]");
          }
        }
      }
      Expr::ObjectLit(o) => {
        self.start(o.origin.as_ref(), None);
        self.object(&o.props);
      }
      Expr::ArrayLit(a) => {
        self.start(a.origin.as_ref(), None);
        self.push("[");

        for (i, element) in a.elements.iter().enumerate() {
          if i > 0 {
            self.push(", ");
          }
          self.expr(element, Prec::Assign);
        }

        self.push("]");
      }
      Expr::Binary(b) => {
        let prec = precedence::binary(b.op);

        self.start(b.origin.as_ref(), None);
        self.expr(&b.left, prec);
        self.push(" ");
        self.push(b.op.as_str());
        self.push(" ");
        self.expr(&b.right, prec.tighter());
      }
      Expr::Logical(l) => {
        let prec = precedence::logical(l.op);

        self.start(l.origin.as_ref(), None);
        self.expr(&l.left, prec);
        self.push(" ");
        self.push(l.op.as_str());
        self.push(" ");
        self.expr(&l.right, prec.tighter());
      }
      Expr::Unary(u) => self.unary(u),
      Expr::Conditional(c) => {
        self.start(c.origin.as_ref(), None);
        self.expr(&c.test, Prec::Or);
        self.push(" ? ");
        self.expr(&c.consequent, Prec::Assign);
        self.push(" : ");
        self.expr(&c.alternate, Prec::Assign);
      }
      Expr::Assign(a) => {
        self.start(a.origin.as_ref(), None);
        self.ident(&a.target);
        self.push(" = ");
        self.expr(&a.value, Prec::Assign);
      }
      Expr::Await(a) => {
        self.start(a.origin.as_ref(), None);
        self.push("await ");
        self.expr(&a.arg, Prec::Unary);
      }
    }
  }

  fn template(&mut self, t: &TemplateLit) {
    self.start(t.origin.as_ref(), None);
    self.push("`");

    for (i, quasi) in t.quasis.iter().enumerate() {
      let mut text = String::new();

      json::write_template_text(&mut text, quasi);
      self.push(&text);

      if let Some(e) = t.exprs.get(i) {
        self.push("${");
        self.expr(e, Prec::Assign);
        self.push("}");
      }
    }

    self.push("`");
  }

  fn arrow(&mut self, a: &ArrowFn) {
    self.start(a.origin.as_ref(), None);

    if a.is_async {
      self.push("async ");
    }

    self.params(&a.params);
    self.push(" => ");

    match &a.body {
      ArrowBody::Expr(body) if matches!(**body, Expr::ObjectLit(_)) => {
        self.push("(");
        self.bare(body);
        self.push(")");
      }
      ArrowBody::Expr(body) => self.expr(body, Prec::Assign),
      ArrowBody::Block(body) => self.body(body),
    }
  }

  fn unary(&mut self, u: &Unary) {
    self.start(u.origin.as_ref(), None);
    self.push(u.op.as_str());

    match &*u.arg {
      Expr::Await(_) => {
        self.push("(");
        self.bare(&u.arg);
        self.push(")");
      }
      Expr::Unary(inner)
        if u.op == UnaryOp::Negate && inner.op == UnaryOp::Negate =>
      {
        self.push(" ");
        self.bare(&u.arg);
      }
      arg => self.expr(arg, Prec::Unary),
    }
  }

  fn callee(&mut self, e: &Expr) {
    if let Expr::Literal(lit) = e
      && matches!(lit.value, LiteralValue::Number(_))
    {
      self.push("(");
      self.bare(e);
      self.push(")");
    } else {
      self.expr(e, Prec::Call);
    }
  }

  fn ident(&mut self, id: &Ident) {
    let name = js_ident(&id.name);
    let original = match (&id.original_name, &name) {
      (Some(original), _) => Some(original.clone()),
      (None, std::borrow::Cow::Owned(_)) => Some(id.name.clone()),
      (None, std::borrow::Cow::Borrowed(_)) => None,
    };

    self.start(id.origin.as_ref(), original);
    self.push(&name);
  }

  fn literal(&mut self, value: &LiteralValue) {
    match value {
      LiteralValue::String(s) => self.push(&json::string(s)),
      LiteralValue::Number(n) => self.push(&number_to_string(*n)),
      LiteralValue::Bool(true) => self.push("true"),
      LiteralValue::Bool(false) => self.push("false"),
      LiteralValue::Null => self.push("null"),
    }
  }

  fn object(&mut self, props: &[ObjectProp]) {
    if props.is_empty() {
      return self.push("{}");
    }

    let one_line = props.len() <= 3
      && props.iter().all(|p| !contains_object_or_fn(prop_value(p)));

    if one_line {
      self.push("{ ");

      for (i, p) in props.iter().enumerate() {
        if i > 0 {
          self.push(", ");
        }
        self.prop(p);
      }

      self.push(" }");
    } else {
      self.push("{\n");
      self.indent += 1;

      for p in props {
        self.line_start();
        self.prop(p);
        self.push(",\n");
      }

      self.indent -= 1;
      self.line_start();
      self.push("}");
    }
  }

  fn prop(&mut self, p: &ObjectProp) {
    match p {
      ObjectProp::KeyValue(kv) => {
        self.start(kv.origin.as_ref(), None);
        self.start(kv.key.origin.as_ref(), None);
        self.push(&kv.key.name);
        self.push(": ");
        self.expr(&kv.value, Prec::Assign);
      }
      ObjectProp::Spread(s) => {
        self.start(s.origin.as_ref(), None);
        self.push("...");
        self.expr(&s.arg, Prec::Assign);
      }
    }
  }
}

fn prop_value(p: &ObjectProp) -> &Expr {
  match p {
    ObjectProp::KeyValue(kv) => &kv.value,
    ObjectProp::Spread(s) => &s.arg,
  }
}

fn contains_object_or_fn(e: &Expr) -> bool {
  match e {
    Expr::ObjectLit(_) | Expr::ArrowFn(_) => true,
    Expr::Ident(_) | Expr::Literal(_) => false,
    Expr::TemplateLit(t) => t.exprs.iter().any(contains_object_or_fn),
    Expr::Call(c) => {
      contains_object_or_fn(&c.callee)
        || c.args.iter().any(contains_object_or_fn)
    }
    Expr::Member(m) => {
      contains_object_or_fn(&m.object)
        || match &m.property {
          MemberProp::Static(_) => false,
          MemberProp::Computed(index) => contains_object_or_fn(index),
        }
    }
    Expr::ArrayLit(a) => a.elements.iter().any(contains_object_or_fn),
    Expr::Binary(b) => {
      contains_object_or_fn(&b.left) || contains_object_or_fn(&b.right)
    }
    Expr::Logical(l) => {
      contains_object_or_fn(&l.left) || contains_object_or_fn(&l.right)
    }
    Expr::Unary(u) => contains_object_or_fn(&u.arg),
    Expr::Conditional(c) => {
      contains_object_or_fn(&c.test)
        || contains_object_or_fn(&c.consequent)
        || contains_object_or_fn(&c.alternate)
    }
    Expr::Assign(a) => contains_object_or_fn(&a.value),
    Expr::Await(a) => contains_object_or_fn(&a.arg),
  }
}

fn starts_ambiguously(e: &Expr) -> bool {
  match e {
    Expr::ObjectLit(_) => true,
    Expr::Member(m) => starts_ambiguously(&m.object),
    Expr::Call(c) => starts_ambiguously(&c.callee),
    Expr::Binary(b) => starts_ambiguously(&b.left),
    Expr::Logical(l) => starts_ambiguously(&l.left),
    Expr::Conditional(c) => starts_ambiguously(&c.test),
    Expr::Ident(_)
    | Expr::Literal(_)
    | Expr::TemplateLit(_)
    | Expr::ArrowFn(_)
    | Expr::ArrayLit(_)
    | Expr::Unary(_)
    | Expr::Assign(_)
    | Expr::Await(_) => false,
  }
}

#[must_use]
pub fn number_to_string(n: f64) -> String {
  if !n.is_finite() || n < 0.0 || (n == 0.0 && n.is_sign_negative()) {
    ice(format!("unprintable number literal {n}"), None);
  }

  if n == 0.0 {
    return "0".to_string();
  }

  let sci = format!("{n:e}");
  let (mantissa, exponent) = sci.split_once('e').unwrap_or((&sci, "0"));
  let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
  let exponent: i64 = exponent.parse().unwrap_or(0);
  let k = i64::try_from(digits.len()).unwrap_or(i64::MAX);
  let point = exponent + 1;

  if k <= point && point <= 21 {
    let zeros = usize::try_from(point - k).unwrap_or(0);

    format!("{digits}{}", "0".repeat(zeros))
  } else if 0 < point && point <= 21 {
    let (int, frac) = digits.split_at(usize::try_from(point).unwrap_or(0));

    format!("{int}.{frac}")
  } else if -6 < point && point <= 0 {
    let zeros = usize::try_from(-point).unwrap_or(0);

    format!("0.{}{digits}", "0".repeat(zeros))
  } else {
    let sign = if point - 1 < 0 { '-' } else { '+' };
    let e = (point - 1).abs();
    let (first, rest) = digits.split_at(1);

    if rest.is_empty() {
      format!("{first}e{sign}{e}")
    } else {
      format!("{first}.{rest}e{sign}{e}")
    }
  }
}
