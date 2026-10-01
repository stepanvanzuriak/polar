use proptest::{collection::vec, prelude::*, strategy::BoxedStrategy};
use std::{collections::BTreeSet, fmt::Write as _};

pub const EFFECTS: [(&str, &str, i64); 3] =
  [("A", "a", 1), ("B", "b", 2), ("C", "c", 3)];

const PREAMBLE: &str = "module Gen

hosts
  H1
  H2

effects
  A in H1 {
    a(x: Int) -> Int
  }

  B in H2 {
    b(x: Int) -> Int
  }

  C in H1 {
    c(x: Int) -> Int
  }

binds
  A in H1 {
    a(x) {
      x + 1
    }
  }

  B in H2 {
    b(x) {
      x + 2
    }
  }

  C in H1 {
    c(x) {
      x + 3
    }
  }

  C in H2 {
    c(x) {
      x + 3
    }
  }
";

#[derive(Debug, Clone)]
pub enum E {
  Lit(i64),
  Param,
  Add(Box<E>, Box<E>),
  Sub(Box<E>, Box<E>),
  Op(usize, Box<E>),
  Call(usize, Box<E>),
  Seq(Box<E>, Box<E>),
}

impl E {
  fn render(&self, out: &mut String) {
    match self {
      E::Lit(n) => out.push_str(&n.to_string()),
      E::Param => out.push('x'),
      E::Add(a, b) | E::Sub(a, b) => {
        out.push('(');
        a.render(out);
        out.push_str(if matches!(self, E::Add(..)) { " + " } else { " - " });
        b.render(out);
        out.push(')');
      }
      E::Op(k, e) => {
        let (effect, op, _) = EFFECTS[*k];

        let _ = write!(out, "{effect}.{op}(");
        e.render(out);
        out.push(')');
      }
      E::Call(j, e) => {
        let _ = write!(out, "g{j}(");
        e.render(out);
        out.push(')');
      }
      E::Seq(first, then) => {
        out.push_str("{\n      let _ = ");
        first.render(out);
        out.push_str("\n      ");
        then.render(out);
        out.push_str("\n    }");
      }
    }
  }

  fn own_labels(&self, out: &mut BTreeSet<usize>, calls: &mut BTreeSet<usize>) {
    match self {
      E::Lit(_) | E::Param => {}
      E::Add(a, b) | E::Sub(a, b) | E::Seq(a, b) => {
        a.own_labels(out, calls);
        b.own_labels(out, calls);
      }
      E::Op(k, e) => {
        out.insert(*k);
        e.own_labels(out, calls);
      }
      E::Call(j, e) => {
        calls.insert(*j);
        e.own_labels(out, calls);
      }
    }
  }

  fn repair(
    self,
    allowed: &BTreeSet<usize>,
    callable: &dyn Fn(usize) -> bool,
  ) -> E {
    let fix = |e: Box<E>| Box::new(e.repair(allowed, callable));

    match self {
      E::Lit(n) => E::Lit(n),
      E::Param => E::Param,
      E::Add(a, b) => E::Add(fix(a), fix(b)),
      E::Sub(a, b) => E::Sub(fix(a), fix(b)),
      E::Seq(a, b) => E::Seq(fix(a), fix(b)),
      E::Op(k, e) if allowed.contains(&k) => E::Op(k, fix(e)),
      E::Op(_, e) if allowed.contains(&2) => E::Op(2, fix(e)),
      E::Call(j, e) if callable(j) => E::Call(j, fix(e)),
      E::Op(_, e) | E::Call(_, e) => e.repair(allowed, callable),
    }
  }

  fn eval(&self, x: i64, fns: &[Function]) -> i64 {
    match self {
      E::Lit(n) => *n,
      E::Param => x,
      E::Add(a, b) => a.eval(x, fns) + b.eval(x, fns),
      E::Sub(a, b) => a.eval(x, fns) - b.eval(x, fns),
      E::Op(k, e) => e.eval(x, fns) + EFFECTS[*k].2,
      E::Call(j, e) => fns[*j].body.eval(e.eval(x, fns), fns),
      E::Seq(_, then) => then.eval(x, fns),
    }
  }
}

#[derive(Debug, Clone)]
pub struct Function {
  pub declared: bool,
  pub row: BTreeSet<usize>,
  pub body: E,
}

#[derive(Debug, Clone)]
pub struct Program {
  pub fns: Vec<Function>,
  pub main: Vec<(usize, i64)>,
}

fn names(labels: &BTreeSet<usize>) -> Vec<&'static str> {
  labels.iter().map(|&k| EFFECTS[k].0).collect()
}

fn compatible(labels: &BTreeSet<usize>) -> bool {
  !(labels.contains(&0) && labels.contains(&1))
}

fn signature(i: usize, f: &Function) -> String {
  if !f.declared {
    return format!("g{i}(x: Int)");
  }

  if f.row.is_empty() {
    format!("g{i}(x: Int) -> Int")
  } else {
    format!("g{i}(x: Int) -> Int / {{{}}}", names(&f.row).join(", "))
  }
}

impl Program {
  #[must_use]
  pub fn source(&self) -> String {
    self.render(&self.fns)
  }

  fn render(&self, fns: &[Function]) -> String {
    let mut out = format!("{PREAMBLE}\nfunctions\n");

    for (i, f) in fns.iter().enumerate() {
      let mut body = String::new();

      f.body.render(&mut body);
      let _ = write!(out, "  {} {{\n    {body}\n  }}\n\n", signature(i, f));
    }

    out.push_str("  main() {\n");

    for (j, arg) in &self.main {
      let _ = writeln!(out, "    Log.info(\"#{{g{j}({arg})}}\")");
    }

    out.push_str("    {}\n  }\n\nexports\n  main\n");
    out
  }

  #[must_use]
  pub fn expected_type(&self, i: usize) -> String {
    let row = &self.fns[i].row;

    if row.is_empty() {
      "function(Int) -> Int".to_string()
    } else {
      format!("function(Int) -> Int / {{{}}}", names(row).join(", "))
    }
  }

  #[must_use]
  pub fn main_labels(&self) -> BTreeSet<usize> {
    self
      .main
      .iter()
      .flat_map(|(j, _)| self.fns[*j].row.iter().copied())
      .collect()
  }

  #[must_use]
  pub fn hosts(&self) -> Vec<&'static str> {
    let labels = self.main_labels();
    let mut out = Vec::new();

    if !labels.contains(&1) {
      out.push("H1");
    }

    if !labels.contains(&0) {
      out.push("H2");
    }

    out
  }

  #[must_use]
  pub fn expected_output(&self) -> String {
    self.main.iter().fold(String::new(), |mut out, (j, arg)| {
      let _ = writeln!(out, "{}", self.fns[*j].body.eval(*arg, &self.fns));
      out
    })
  }

  #[must_use]
  pub fn drop_label_sites(&self) -> Vec<usize> {
    (0..self.fns.len())
      .filter(|&i| self.fns[i].declared && !self.fns[i].row.is_empty())
      .collect()
  }

  #[must_use]
  pub fn without_label(&self, i: usize) -> String {
    let mut fns = self.fns.clone();
    let first = *fns[i].row.iter().next().unwrap();

    fns[i].row.remove(&first);
    self.render(&fns)
  }

  #[must_use]
  pub fn with_disjoint_call(&self, i: usize) -> String {
    let mut fns = self.fns.clone();
    let body = std::mem::replace(&mut fns[i].body, E::Lit(0));

    fns[i].body = E::Seq(
      Box::new(E::Op(0, Box::new(E::Lit(0)))),
      Box::new(E::Seq(Box::new(E::Op(1, Box::new(E::Lit(0)))), Box::new(body))),
    );
    fns[i].row.insert(0);
    fns[i].row.insert(1);
    self.render(&fns)
  }
}

fn expr() -> BoxedStrategy<E> {
  let leaf = prop_oneof![(0i64..10).prop_map(E::Lit), Just(E::Param)];

  leaf
    .prop_recursive(4, 24, 2, |inner| {
      prop_oneof![
        1 => (inner.clone(), inner.clone())
          .prop_map(|(a, b)| E::Add(Box::new(a), Box::new(b))),
        1 => (inner.clone(), inner.clone())
          .prop_map(|(a, b)| E::Sub(Box::new(a), Box::new(b))),
        3 => (0usize..3, inner.clone()).prop_map(|(k, e)| E::Op(k, Box::new(e))),
        2 => (0usize..4, inner.clone()).prop_map(|(j, e)| E::Call(j, Box::new(e))),
        1 => (inner.clone(), inner)
          .prop_map(|(a, b)| E::Seq(Box::new(a), Box::new(b))),
      ]
    })
    .boxed()
}

const ALLOWED: [&[usize]; 4] = [&[0, 2], &[1, 2], &[2], &[]];

pub fn program() -> BoxedStrategy<Program> {
  let function = (0usize..4, any::<bool>(), expr());

  (vec(function, 1..=4), vec((0usize..4, 0i64..10), 1..=3))
    .prop_map(|(raw, calls)| {
      let mut fns: Vec<Function> = Vec::new();

      for (allowed, declared, body) in raw {
        let allowed: BTreeSet<usize> =
          ALLOWED[allowed].iter().copied().collect();
        let body = {
          let rows: Vec<BTreeSet<usize>> =
            fns.iter().map(|f| f.row.clone()).collect();
          let callable =
            |j: usize| j < rows.len() && rows[j].is_subset(&allowed);

          body.repair(&allowed, &callable)
        };
        let mut own = BTreeSet::new();
        let mut callees = BTreeSet::new();

        body.own_labels(&mut own, &mut callees);

        for j in callees {
          own.extend(fns[j].row.iter().copied());
        }

        fns.push(Function { declared, row: own, body });
      }

      let mut main: Vec<(usize, i64)> = Vec::new();
      let mut labels = BTreeSet::new();

      for (j, arg) in calls {
        let j = j % fns.len();
        let mut next = labels.clone();

        next.extend(fns[j].row.iter().copied());

        if compatible(&next) {
          labels = next;
          main.push((j, arg));
        }
      }

      Program { fns, main }
    })
    .boxed()
}
