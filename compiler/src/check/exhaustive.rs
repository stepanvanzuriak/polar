use std::sync::Arc;

use crate::{
  check::{Checker, convert::substitute},
  core::lower::Resolved,
  shared::codes::DiagnosticCode::{
    NonExhaustiveMatch, RefutableLet, UnreachableArm,
  },
  shared::diagnostic::{Diagnostic, Label},
  shared::source::Span,
  syntax::ast::{Expr, LetStmt, Match, MatchArm, PatLit, Pattern, StringPart},
  syntax::params,
  types::{print::bare, ty::Type},
};

pub(crate) enum Site<'a> {
  Match(&'a Match),
  Let(&'a LetStmt),
  Catch { arms: Vec<&'a MatchArm>, ty: Type, span: Span },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Pat {
  Wild,
  Ctor { name: Arc<str>, args: Vec<Pat>, list: bool },
  Bool(bool),
  Lit(String),
  Record(Vec<(Arc<str>, Pat)>),
}

#[derive(Debug, Clone, PartialEq)]
enum Head {
  Ctor(Arc<str>),
  Bool(bool),
  Lit(String),
  Record(Vec<Arc<str>>),
}

pub trait Signatures {
  fn ctors(&self, ty: &Type) -> Option<Vec<(Arc<str>, Vec<Type>)>>;

  fn is_list(&self, ty: &Type) -> bool;
}

#[must_use]
pub fn useful(
  rows: &[Vec<Pat>],
  new: &[Pat],
  types: &[Type],
  sigs: &dyn Signatures,
) -> Option<Vec<Pat>> {
  let Some((first, rest)) = new.split_first() else {
    return rows.is_empty().then(Vec::new);
  };
  let ty = types.first().cloned().unwrap_or(Type::Gen(0));
  let rest_types = types.get(1..).unwrap_or(&[]);

  if let Some(head) = head_of(first, &ty) {
    return try_head(rows, new, &head, &ty, rest_types, sigs);
  }

  let heads: Vec<Head> = rows
    .iter()
    .filter_map(|row| row.first().and_then(|p| head_of(p, &ty)))
    .fold(Vec::new(), |mut acc, h| {
      if !acc.contains(&h) {
        acc.push(h);
      }
      acc
    });
  let all = all_heads(&ty, sigs);

  if let Some(all) = &all
    && !all.is_empty()
    && all.iter().all(|h| heads.contains(h))
  {
    return all
      .iter()
      .find_map(|head| try_head(rows, new, head, &ty, rest_types, sigs));
  }

  let default: Vec<Vec<Pat>> = rows
    .iter()
    .filter(|row| row.first().is_some_and(|p| matches!(p, Pat::Wild)))
    .map(|row| row[1..].to_vec())
    .collect();
  let mut witness = useful(&default, rest, rest_types, sigs)?;
  let missing = match &all {
    Some(all) if !heads.is_empty() => all
      .iter()
      .find(|h| !heads.contains(h))
      .map_or(Pat::Wild, |h| wild_of(h, &ty, sigs)),
    _ => Pat::Wild,
  };

  witness.insert(0, missing);
  Some(witness)
}

fn try_head(
  rows: &[Vec<Pat>],
  new: &[Pat],
  head: &Head,
  ty: &Type,
  rest_types: &[Type],
  sigs: &dyn Signatures,
) -> Option<Vec<Pat>> {
  let args = arg_types(head, ty, sigs);
  let arity = args.len();
  let rows: Vec<Vec<Pat>> =
    rows.iter().filter_map(|row| specialise(row, head, arity)).collect();
  let new = specialise(new, head, arity)?;
  let mut types = args;

  types.extend(rest_types.iter().cloned());

  let mut witness = useful(&rows, &new, &types, sigs)?;
  let inner: Vec<Pat> = witness.drain(..arity).collect();

  witness.insert(0, rebuild(head, inner, sigs.is_list(ty)));
  Some(witness)
}

fn head_of(pat: &Pat, ty: &Type) -> Option<Head> {
  match pat {
    Pat::Wild => None,
    Pat::Ctor { name, .. } => Some(Head::Ctor(name.clone())),
    Pat::Bool(b) => Some(Head::Bool(*b)),
    Pat::Lit(s) => Some(Head::Lit(s.clone())),
    Pat::Record(fields) => {
      if let Type::Record(row) = ty {
        return Some(Head::Record(sorted_fields(row)));
      }

      let mut names: Vec<Arc<str>> =
        fields.iter().map(|(n, _)| n.clone()).collect();

      names.sort();
      Some(Head::Record(names))
    }
  }
}

fn sorted_fields(row: &crate::types::ty::Row) -> Vec<Arc<str>> {
  let mut names: Vec<Arc<str>> =
    row.fields.iter().map(|(n, _)| n.clone()).collect();

  names.sort();
  names
}

fn all_heads(ty: &Type, sigs: &dyn Signatures) -> Option<Vec<Head>> {
  match ty {
    Type::Con { name, args } if &**name == "Bool" && args.is_empty() => {
      Some(vec![Head::Bool(true), Head::Bool(false)])
    }
    Type::Record(row) => Some(vec![Head::Record(sorted_fields(row))]),
    _ => sigs.ctors(ty).map(|ctors| {
      ctors.into_iter().map(|(name, _)| Head::Ctor(name)).collect()
    }),
  }
}

fn arg_types(head: &Head, ty: &Type, sigs: &dyn Signatures) -> Vec<Type> {
  match head {
    Head::Bool(_) | Head::Lit(_) => Vec::new(),
    Head::Record(names) => names
      .iter()
      .map(|n| match ty {
        Type::Record(row) => row
          .fields
          .iter()
          .find(|(m, _)| m == n)
          .map_or(Type::Gen(0), |(_, t)| t.clone()),
        _ => Type::Gen(0),
      })
      .collect(),
    Head::Ctor(name) => sigs
      .ctors(ty)
      .and_then(|ctors| {
        ctors.into_iter().find(|(n, _)| n == name).map(|(_, args)| args)
      })
      .unwrap_or_default(),
  }
}

fn specialise(row: &[Pat], head: &Head, arity: usize) -> Option<Vec<Pat>> {
  let (first, rest) = row.split_first()?;
  let mut out: Vec<Pat> = match (first, head) {
    (Pat::Wild, _) => vec![Pat::Wild; arity],
    (Pat::Ctor { name, args, .. }, Head::Ctor(h)) if name == h => {
      let mut args = args.clone();

      args.resize(arity, Pat::Wild);
      args
    }
    (Pat::Bool(b), Head::Bool(h)) if b == h => Vec::new(),
    (Pat::Lit(s), Head::Lit(h)) if s == h => Vec::new(),
    (Pat::Record(fields), Head::Record(names)) => names
      .iter()
      .map(|n| {
        fields
          .iter()
          .find(|(m, _)| m == n)
          .map_or(Pat::Wild, |(_, p)| p.clone())
      })
      .collect(),
    _ => return None,
  };

  out.extend(rest.iter().cloned());
  Some(out)
}

fn rebuild(head: &Head, args: Vec<Pat>, list: bool) -> Pat {
  match head {
    Head::Ctor(name) => Pat::Ctor { name: name.clone(), args, list },
    Head::Bool(b) => Pat::Bool(*b),
    Head::Lit(s) => Pat::Lit(s.clone()),
    Head::Record(names) => {
      Pat::Record(names.iter().cloned().zip(args).collect())
    }
  }
}

fn wild_of(head: &Head, ty: &Type, sigs: &dyn Signatures) -> Pat {
  let arity = arg_types(head, ty, sigs).len();

  rebuild(head, vec![Pat::Wild; arity], sigs.is_list(ty))
}

#[must_use]
pub fn show(pat: &Pat) -> String {
  match pat {
    Pat::Wild => "_".to_string(),
    Pat::Bool(b) => b.to_string(),
    Pat::Lit(s) => s.clone(),
    Pat::Ctor { list: true, .. } => show_list(pat),
    Pat::Ctor { name, args, .. } if args.is_empty() => name.to_string(),
    Pat::Ctor { name, args, .. } => {
      let args: Vec<String> = args.iter().map(show).collect();

      format!("{name}({})", args.join(", "))
    }
    Pat::Record(fields) => {
      let shown: Vec<String> = fields
        .iter()
        .filter(|(_, p)| *p != Pat::Wild)
        .map(|(n, p)| format!("{n}: {}", show(p)))
        .collect();

      match (shown.is_empty(), shown.len() < fields.len()) {
        (true, _) => "_".to_string(),
        (false, true) => format!("{{ {}, .. }}", shown.join(", ")),
        (false, false) => format!("{{ {} }}", shown.join(", ")),
      }
    }
  }
}

fn show_list(pat: &Pat) -> String {
  let mut items = Vec::new();
  let mut rest = pat;

  loop {
    match rest {
      Pat::Ctor { list: true, args, .. } if args.len() == 2 => {
        items.push(show(&args[0]));
        rest = &args[1];
      }
      Pat::Ctor { list: true, args, .. } if args.is_empty() => {
        return format!("[{}]", items.join(", "));
      }
      other => {
        items.push(format!("..{}", show(other)));
        return format!("[{}]", items.join(", "));
      }
    }
  }
}

struct Env<'c, 'a> {
  checker: &'c Checker<'a>,
}

impl Signatures for Env<'_, '_> {
  fn ctors(&self, ty: &Type) -> Option<Vec<(Arc<str>, Vec<Type>)>> {
    let Type::Con { name, args } = ty else { return None };
    let ctors = self.checker.env.ctors_of(name)?;

    Some(
      ctors
        .iter()
        .map(|(ctor, scheme)| {
          let params = match substitute(&scheme.ty, args) {
            Type::Fn { params, .. } => params,
            _ => Vec::new(),
          };

          (Arc::from(ctor.as_str()), params)
        })
        .collect(),
    )
  }

  fn is_list(&self, ty: &Type) -> bool {
    let Type::Con { name, .. } = ty else { return false };

    bare(name) == "List"
      && self.ctors(ty).is_some_and(|ctors| {
        ctors.iter().any(|(n, a)| &**n == "Cons" && a.len() == 2)
          && ctors.iter().any(|(n, a)| &**n == "Nil" && a.is_empty())
      })
  }
}

impl Checker<'_> {
  pub(crate) fn exhaustiveness(&mut self) {
    if self.bag.has_errors() {
      return;
    }

    let sites = std::mem::take(&mut self.matches);
    let mut diagnostics = Vec::new();

    for site in &sites {
      match site {
        Site::Match(node) => self.check_match(node, &mut diagnostics),
        Site::Let(stmt) => self.check_let(stmt, &mut diagnostics),
        Site::Catch { arms, ty, span } => {
          let ty = self.store.zonk(ty);

          self.check_arms(
            arms,
            std::slice::from_ref(&ty),
            span,
            "catch",
            &mut diagnostics,
          );
        }
      }
    }

    for diagnostic in diagnostics {
      self.push(diagnostic);
    }
  }

  fn scrutinee_type(&self, span: &Span) -> Type {
    self
      .exprs
      .get(&(span.start, span.end))
      .map_or(Type::Gen(0), |t| self.store.zonk(t))
  }

  fn check_match(&self, node: &Match, out: &mut Vec<Diagnostic>) {
    let types: Vec<Type> = node
      .subjects
      .iter()
      .map(|subject| self.scrutinee_type(subject.span()))
      .collect();
    let end = node.subjects.last().map_or(node.span.end, |s| s.span().end);
    let span =
      Span { file: node.span.file.clone(), start: node.span.start, end };
    let arms: Vec<&MatchArm> = node.arms.iter().collect();

    self.check_arms(&arms, &types, &span, "match", out);
  }

  fn rows(&self, arm: &MatchArm) -> Vec<Vec<Pat>> {
    arm
      .rows
      .iter()
      .flat_map(|row| product(row.iter().map(|p| self.pats(p)).collect()))
      .collect()
  }

  fn check_arms(
    &self,
    arms: &[&MatchArm],
    types: &[Type],
    span: &Span,
    what: &str,
    out: &mut Vec<Diagnostic>,
  ) {
    let sigs = Env { checker: self };
    let rows: Vec<Vec<Vec<Pat>>> =
      arms.iter().map(|arm| self.rows(arm)).collect();
    let covering = |i: usize| -> Vec<Vec<Pat>> {
      (0..i)
        .filter(|&j| arms[j].guard.is_none())
        .flat_map(|j| rows[j].clone())
        .collect()
    };

    for (i, arm) in arms.iter().enumerate() {
      let earlier = covering(i);

      if rows[i].iter().any(|row| useful(&earlier, row, types, &sigs).is_some())
      {
        continue;
      }

      let mut diagnostic = Diagnostic::warning(
        UnreachableArm,
        "this arm can never run",
        Label::new(arm.span.clone())
          .with_message("earlier arms already match everything it would"),
      );
      let by = (0..i).filter(|&j| arms[j].guard.is_none()).find(|&j| {
        rows[i].iter().all(|row| useful(&rows[j], row, types, &sigs).is_none())
      });

      if let Some(j) = by {
        diagnostic = diagnostic.with_secondary(
          Label::new(arms[j].span.clone())
            .with_message("this arm already covers it"),
        );
      }

      out.push(diagnostic);
    }

    let all = covering(arms.len());
    let wild = vec![Pat::Wild; types.len()];

    if let Some(witness) = useful(&all, &wild, types, &sigs) {
      let shown: Vec<String> = witness.iter().map(show).collect();

      out.push(Diagnostic::error(
        NonExhaustiveMatch,
        format!("this `{what}` doesn't cover every case"),
        Label::new(span.clone())
          .with_message(format!("`{}` is not covered", shown.join(", "))),
      ));
    }
  }

  fn check_let(&self, stmt: &LetStmt, out: &mut Vec<Diagnostic>) {
    let ty = self.scrutinee_type(stmt.value.span());
    let sigs = Env { checker: self };
    let rows: Vec<Vec<Pat>> =
      self.pats(&stmt.pattern).into_iter().map(|p| vec![p]).collect();

    if let Some(witness) = useful(&rows, &[Pat::Wild], &[ty], &sigs) {
      let parameter = matches!(&stmt.value, Expr::Var(v) if params::is_synthetic(&v.name.text));
      let (message, help) = if parameter {
        (
          "this parameter pattern can fail, but a parameter has nowhere else to go",
          "take a plain parameter and `match` on it",
        )
      } else {
        (
          "this pattern can fail, but `let` has nowhere else to go",
          "use `match` to handle the other cases",
        )
      };

      out.push(
        Diagnostic::error(
          RefutableLet,
          message,
          Label::new(stmt.pattern.span().clone())
            .with_message(format!("`{}` is not covered", show(&witness[0]))),
        )
        .with_help(help),
      );
    }
  }

  fn pats(&self, pattern: &Pattern) -> Vec<Pat> {
    match pattern {
      Pattern::Or(or) => {
        or.alternatives.iter().flat_map(|p| self.pats(p)).collect()
      }
      Pattern::Ctor(ctor) => {
        let name = self.ctor_name(ctor);

        product(ctor.args.iter().map(|a| self.pats(a)).collect())
          .into_iter()
          .map(|args| Pat::Ctor { name: name.clone(), args, list: false })
          .collect()
      }
      Pattern::Record(record) => {
        let mut names: Vec<&str> = Vec::new();
        let mut columns: Vec<Vec<Pat>> = Vec::new();

        for field in &record.fields {
          if !names.contains(&field.name.text.as_str()) {
            names.push(&field.name.text);
            columns.push(self.pats(&field.pattern));
          }
        }

        product(columns)
          .into_iter()
          .map(|values| {
            let mut fields: Vec<(Arc<str>, Pat)> =
              names.iter().map(|n| Arc::from(*n)).zip(values).collect();

            fields.sort_by(|a, b| a.0.cmp(&b.0));
            Pat::Record(fields)
          })
          .collect()
      }
      Pattern::List(list) => {
        let (nil, cons) = self.list_ctors(list);
        let ends = match &list.tail {
          Some(tail) => self.pats(tail),
          None => {
            vec![Pat::Ctor { name: nil.clone(), args: Vec::new(), list: false }]
          }
        };
        let items: Vec<Vec<Pat>> =
          list.items.iter().map(|i| self.pats(i)).collect();

        ends
          .into_iter()
          .flat_map(|end| {
            let cons = cons.clone();

            product(items.clone()).into_iter().map(move |values| {
              values.into_iter().rev().fold(end.clone(), |acc, item| {
                Pat::Ctor {
                  name: cons.clone(),
                  args: vec![item, acc],
                  list: false,
                }
              })
            })
          })
          .collect()
      }
      _ => vec![Self::pat(pattern)],
    }
  }

  fn ctor_name(&self, ctor: &crate::syntax::ast::PCtor) -> Arc<str> {
    let name = match self.resolutions.get(&ctor.name.span) {
      Some(Resolved::Ctor(id)) => self
        .core
        .ctors
        .get(id.0 as usize)
        .map_or(ctor.name.text.clone(), |c| c.name.clone()),
      _ => ctor.name.text.clone(),
    };

    Arc::from(name.as_str())
  }

  fn list_ctors(
    &self,
    list: &crate::syntax::ast::PList,
  ) -> (Arc<str>, Arc<str>) {
    match self.resolutions.get(&list.span) {
      Some(Resolved::List { nil, cons }) => {
        let name = |id: crate::core::ir::CtorId| {
          self
            .core
            .ctors
            .get(id.0 as usize)
            .map_or_else(String::new, |c| c.name.clone())
        };

        (Arc::from(name(*nil).as_str()), Arc::from(name(*cons).as_str()))
      }
      _ => (Arc::from("Nil"), Arc::from("Cons")),
    }
  }

  fn pat(pattern: &Pattern) -> Pat {
    match pattern {
      Pattern::Lit(lit) => match &lit.lit {
        PatLit::Bool(b) => Pat::Bool(b.value),
        PatLit::Int(int) => Pat::Lit(number(&int.raw, lit.negative)),
        PatLit::Float(float) => Pat::Lit(number(&float.raw, lit.negative)),
        PatLit::String(s) => {
          let text: String = s
            .parts
            .iter()
            .map(|part| match part {
              StringPart::Text(text) => text.raw.as_str(),
              StringPart::Interp(_) => "",
            })
            .collect();

          Pat::Lit(format!("\"{text}\""))
        }
      },
      _ => Pat::Wild,
    }
  }
}

fn product(columns: Vec<Vec<Pat>>) -> Vec<Vec<Pat>> {
  columns.into_iter().fold(vec![Vec::new()], |rows, column| {
    rows
      .iter()
      .flat_map(|row| {
        column.iter().map(move |pat| {
          let mut row = row.clone();

          row.push(pat.clone());
          row
        })
      })
      .collect()
  })
}

fn number(raw: &str, negative: bool) -> String {
  let digits: String = raw.chars().filter(|&c| c != '_').collect();
  let value = digits.parse::<f64>().unwrap_or(0.0);

  if negative && value != 0.0 { format!("-{digits}") } else { digits }
}
