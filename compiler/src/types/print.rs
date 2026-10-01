use std::{
  collections::{BTreeMap, BTreeSet},
  fmt::Write as _,
  sync::Arc,
};

use crate::types::{
  store::Store,
  ty::{EffTail, Effects, Label, Row, Scheme, TVar, Tail, Type},
};

#[must_use]
pub fn print(ty: &Type, store: &Store) -> String {
  let ty = store.zonk(ty);

  Printer::new(&[&ty], false).print(&ty)
}

#[must_use]
pub fn print_all(types: &[&Type], store: &Store, aliases: bool) -> Vec<String> {
  let zonked: Vec<Type> = types.iter().map(|t| store.zonk(t)).collect();
  let refs: Vec<&Type> = zonked.iter().collect();
  let mut printer = Printer::new(&refs, aliases);

  zonked.iter().map(|t| printer.print(t)).collect()
}

#[must_use]
pub fn print_scheme(scheme: &Scheme) -> String {
  let mut all: Vec<&Type> = vec![&scheme.ty];

  all.extend(scheme.preds.iter().map(|p| &p.ty));

  let mut printer = Printer::new(&all, false);
  let mut out = printer.print(&scheme.ty);

  if scheme.preds.is_empty() {
    return out;
  }

  let mut full: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();

  for pred in &scheme.preds {
    full.entry(bare(&pred.trait_path)).or_default().insert(&pred.trait_path);
  }

  let preds: Vec<String> = scheme
    .preds
    .iter()
    .map(|p| {
      let name = bare(&p.trait_path);
      let name = if full[name].len() > 1 {
        p.trait_path.to_string()
      } else {
        name.to_string()
      };

      format!("{name}<{}>", printer.print(&p.ty))
    })
    .collect();

  out.push_str(" where ");
  out.push_str(&preds.join(", "));
  out
}

#[must_use]
pub fn bare(name: &str) -> &str {
  name.rsplit('.').next().unwrap_or(name)
}

const LETTERS: &[u8; 25] = b"abcdfghijklmnopqrstuvwxyz";

#[derive(Clone, Copy, PartialEq)]
enum Key {
  Var(TVar),
  Gen(u32),
}

pub struct Printer {
  names: Vec<(Key, String)>,
  row_names: Vec<Key>,
  effect_names: Vec<Key>,
  reserved: BTreeSet<String>,
  qualified: BTreeSet<Arc<str>>,
  aliases: bool,
}

impl Printer {
  #[must_use]
  pub fn new(types: &[&Type], aliases: bool) -> Self {
    let mut reserved = BTreeSet::new();
    let mut full: BTreeMap<String, BTreeSet<Arc<str>>> = BTreeMap::new();

    for ty in types {
      scan(ty, &mut reserved, &mut full);
    }

    let qualified =
      full.into_values().filter(|names| names.len() > 1).flatten().collect();

    Self {
      names: Vec::new(),
      row_names: Vec::new(),
      effect_names: Vec::new(),
      reserved,
      qualified,
      aliases,
    }
  }

  pub fn print(&mut self, ty: &Type) -> String {
    let mut out = String::new();

    self.ty(ty, &mut out);
    out
  }

  fn ty(&mut self, ty: &Type, out: &mut String) {
    match ty {
      Type::Var(v) => self.letter(Key::Var(*v), out),
      Type::Gen(i) => self.letter(Key::Gen(*i), out),
      Type::Rigid { name, .. } => out.push_str(name),
      Type::Con { name, args } => {
        self.name(name, out);

        if !args.is_empty() {
          out.push('<');
          self.list(args, out);
          out.push('>');
        }
      }
      Type::Fn { params, ret, effects } => {
        out.push_str("function(");
        self.list(params, out);
        out.push_str(") -> ");

        let captures = !effects.is_pure()
          && matches!(&**ret, Type::Fn { effects: inner, .. } if inner.is_pure());

        if captures {
          out.push('(');
          self.ty(ret, out);
          out.push(')');
        } else {
          self.ty(ret, out);
        }

        if !effects.is_pure() {
          out.push_str(" / ");
          self.effects(effects, out);
        }
      }
      Type::Record(row) => self.row(row, out),
    }
  }

  pub fn effects(&mut self, effects: &Effects, out: &mut String) {
    out.push('{');

    for (i, label) in effects.labels.iter().enumerate() {
      if i > 0 {
        out.push_str(", ");
      }

      self.label(label, out);
    }

    let key = match effects.tail {
      EffTail::Closed => None,
      EffTail::Open(v) | EffTail::Rigid(v) => Some(Key::Var(v)),
      EffTail::Gen(i) => Some(Key::Gen(i)),
    };

    if let Some(key) = key {
      if !effects.labels.is_empty() {
        out.push(' ');
      }

      match index_of(&mut self.effect_names, key) {
        0 => out.push_str("| e"),
        n => {
          let _ = write!(out, "| e{}", n + 1);
        }
      }
    }

    out.push('}');
  }

  pub fn label(&mut self, label: &Label, out: &mut String) {
    out.push_str(&label.name);

    if let Some(arg) = &label.arg {
      out.push('<');
      self.name(arg, out);
      out.push('>');
    }
  }

  fn letter(&mut self, key: Key, out: &mut String) {
    if let Some((_, name)) = self.names.iter().find(|(k, _)| *k == key) {
      out.push_str(name);
      return;
    }

    let mut i = self.names.len();
    let name = loop {
      let candidate = letter_name(i);

      if !self.reserved.contains(&candidate)
        && !self.names.iter().any(|(_, n)| *n == candidate)
      {
        break candidate;
      }

      i += 1;
    };

    out.push_str(&name);
    self.names.push((key, name));
  }

  fn row(&mut self, row: &Row, out: &mut String) {
    if self.aliases
      && let Some(brand) = row.brand()
    {
      self.name(brand, out);
      return;
    }

    self.fields(row, out);

    if let Some(brand) = row.brand() {
      out.push_str(" as ");
      self.name(brand, out);
    }
  }

  fn name(&self, name: &Arc<str>, out: &mut String) {
    if self.qualified.contains(name) {
      out.push_str(name);
    } else {
      out.push_str(bare(name));
    }
  }

  fn fields(&mut self, row: &Row, out: &mut String) {
    let mut fields: Vec<_> = row.fields.iter().collect();
    fields.sort_by(|a, b| a.0.cmp(&b.0));

    if fields.is_empty() && row.tail.is_closed() {
      out.push_str("{}");
      return;
    }

    out.push_str("{ ");

    for (i, (name, ty)) in fields.iter().enumerate() {
      if i > 0 {
        out.push_str(", ");
      }

      out.push_str(name);
      out.push_str(": ");
      self.ty(ty, out);
    }

    let key = match row.tail {
      Tail::Closed(_) => None,
      Tail::Open(v) | Tail::Rigid(v) => Some(Key::Var(v)),
      Tail::Gen(i) => Some(Key::Gen(i)),
    };

    if let Some(key) = key {
      if !fields.is_empty() {
        out.push(' ');
      }

      match index_of(&mut self.row_names, key) {
        0 => out.push_str("| r"),
        n => {
          let _ = write!(out, "| r{n}");
        }
      }
    }

    out.push_str(" }");
  }

  fn list(&mut self, types: &[Type], out: &mut String) {
    for (i, ty) in types.iter().enumerate() {
      if i > 0 {
        out.push_str(", ");
      }

      self.ty(ty, out);
    }
  }
}

fn letter_name(i: usize) -> String {
  let letter = char::from(LETTERS[i % LETTERS.len()]);

  match i / LETTERS.len() {
    0 => letter.to_string(),
    n => format!("{letter}{n}"),
  }
}

fn scan(
  ty: &Type,
  reserved: &mut BTreeSet<String>,
  full: &mut BTreeMap<String, BTreeSet<Arc<str>>>,
) {
  match ty {
    Type::Var(_) | Type::Gen(_) => {}
    Type::Rigid { name, .. } => {
      reserved.insert(name.to_string());
    }
    Type::Con { name, args } => {
      full.entry(bare(name).to_string()).or_default().insert(name.clone());

      for arg in args {
        scan(arg, reserved, full);
      }
    }
    Type::Fn { params, ret, effects } => {
      for param in params {
        scan(param, reserved, full);
      }

      scan(ret, reserved, full);

      for arg in effects.labels.iter().filter_map(|l| l.arg.as_ref()) {
        full.entry(bare(arg).to_string()).or_default().insert(arg.clone());
      }
    }
    Type::Record(row) => {
      if let Some(brand) = row.brand() {
        full.entry(bare(brand).to_string()).or_default().insert(brand.clone());
      }

      for (_, field) in &row.fields {
        scan(field, reserved, full);
      }
    }
  }
}

fn index_of(seen: &mut Vec<Key>, key: Key) -> usize {
  if let Some(i) = seen.iter().position(|&k| k == key) {
    return i;
  }

  seen.push(key);
  seen.len() - 1
}
