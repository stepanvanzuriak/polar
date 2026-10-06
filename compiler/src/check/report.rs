use std::sync::Arc;

use crate::{
  core::scope::suggest,
  shared::codes::DiagnosticCode::{
    AnnotationTooGeneral, ArgumentCount, ExtraField, InfiniteType,
    MissingField, NotAFunction, NotNumeric, TypeMismatch,
  },
  shared::diagnostic::{Diagnostic, Label},
  shared::source::Span,
  types::{
    print::{Printer, bare},
    store::Store,
    ty::{Constraint, Tail, Type},
    unify::UnifyError,
  },
};

#[derive(Debug, Clone)]
pub enum Because {
  Nothing,
  Annotation(Span),
  Parameter(Span),
  Branch(Span, Type),
  Access,
  Trait(Span, Type),
}

pub struct Report<'e> {
  pub error: &'e UnifyError,
  pub expected: &'e Type,
  pub found: &'e Type,
  pub span: Span,
  pub because: Because,
  pub int_literal: Option<String>,
}

#[must_use]
pub fn report(error: &UnifyError, span: Span, store: &Store) -> Diagnostic {
  let (expected, found) = match error {
    UnifyError::Mismatch { expected, found } => {
      (expected.clone(), found.clone())
    }
    UnifyError::RigidEscape { rigid, other } => (rigid.clone(), other.clone()),
    UnifyError::Occurs { var, ty } => (Type::Var(*var), ty.clone()),
    UnifyError::MissingField { record, .. }
    | UnifyError::ExtraField { record, .. } => (record.clone(), record.clone()),
    UnifyError::NotNumeric { found, .. } => (found.clone(), found.clone()),
    UnifyError::ParamCount { .. }
    | UnifyError::MissingEffect { .. }
    | UnifyError::ExtraEffect { .. }
    | UnifyError::EffectMismatch { .. }
    | UnifyError::EffectEscape { .. } => (Type::unit(), Type::unit()),
  };

  Report {
    error,
    expected: &store.zonk(&expected),
    found: &store.zonk(&found),
    span,
    because: Because::Nothing,
    int_literal: None,
  }
  .build(store)
}

impl Report<'_> {
  #[must_use]
  pub fn build(self, store: &Store) -> Diagnostic {
    let expected = self.expected.clone();
    let found = self.found.clone();
    let branch = match &self.because {
      Because::Branch(_, ty) | Because::Trait(_, ty) => Some(store.zonk(ty)),
      _ => None,
    };
    let inner: Vec<Type> = match self.error {
      UnifyError::Mismatch { expected, found } => {
        vec![store.zonk(expected), store.zonk(found)]
      }
      UnifyError::Occurs { var, ty } => {
        vec![store.zonk(&Type::Var(*var)), store.zonk(ty)]
      }
      UnifyError::MissingField { record, .. }
      | UnifyError::ExtraField { record, .. } => vec![store.zonk(record)],
      UnifyError::NotNumeric { found, .. } => vec![store.zonk(found)],
      UnifyError::RigidEscape { rigid, other } => {
        vec![store.zonk(rigid), store.zonk(other)]
      }
      UnifyError::ParamCount { .. }
      | UnifyError::MissingEffect { .. }
      | UnifyError::ExtraEffect { .. }
      | UnifyError::EffectMismatch { .. }
      | UnifyError::EffectEscape { .. } => Vec::new(),
    };
    let mut all: Vec<&Type> = vec![&expected, &found];

    all.extend(inner.iter());
    all.extend(branch.iter());

    let mut printer = Printer::new(&all, true);

    match self.error {
      UnifyError::Mismatch { .. }
      | UnifyError::MissingEffect { .. }
      | UnifyError::ExtraEffect { .. }
      | UnifyError::EffectMismatch { .. }
      | UnifyError::EffectEscape { .. } => {
        self.mismatch(&expected, &found, branch.as_ref(), &mut printer)
      }
      UnifyError::ParamCount { expected: e, found: f } => {
        self.param_count(*e, *f, &expected, &found, &mut printer)
      }
      UnifyError::Occurs { .. } => {
        let label = format!(
          "this would make `{}` equal to `{}`",
          printer.print(&inner[0]),
          printer.print(&inner[1])
        );

        Diagnostic::error(
          InfiniteType,
          "infinite type",
          Label::new(self.span.clone()).with_message(label),
        )
        .with_help("this usually means an argument is in the wrong position")
      }
      UnifyError::MissingField { label, record } => {
        self.missing(label, record, &expected, &found, &mut printer)
      }
      UnifyError::ExtraField { label, .. } => {
        self.extra(label, &expected, &mut printer)
      }
      UnifyError::NotNumeric { constraint, .. } => {
        not_numeric(*constraint, &printer.print(&inner[0]), &self.span)
      }
      UnifyError::RigidEscape { .. } => {
        self.rigid(&inner[0], &inner[1], &mut printer)
      }
    }
  }

  fn param_count(
    &self,
    e: usize,
    f: usize,
    expected: &Type,
    found: &Type,
    printer: &mut Printer,
  ) -> Diagnostic {
    let diagnostic = Diagnostic::error(
      ArgumentCount,
      format!(
        "expected a function that takes {}, found one that takes {f}",
        arguments(e)
      ),
      Label::new(self.span.clone()).with_message(format!(
        "expected `{}`, found `{}`",
        printer.print(expected),
        printer.print(found)
      )),
    );

    self.secondary(diagnostic, printer, None)
  }

  fn extra(
    &self,
    label: &Arc<str>,
    expected: &Type,
    printer: &mut Printer,
  ) -> Diagnostic {
    let owner = match expected {
      Type::Record(_) => format!("the type `{}`", printer.print(expected)),
      _ => "the expected record type".to_string(),
    };
    let diagnostic = Diagnostic::error(
      ExtraField,
      format!("unexpected field `{label}`"),
      Label::new(self.span.clone())
        .with_message(format!("{owner} has no field `{label}`")),
    )
    .with_note(
      "this record type is closed; write `| rest` in the annotation to \
       accept extra fields",
    );

    self.secondary(diagnostic, printer, None)
  }

  fn rigid(
    &self,
    rigid: &Type,
    other: &Type,
    printer: &mut Printer,
  ) -> Diagnostic {
    let rigid = printer.print(rigid);
    let other = printer.print(other);
    let diagnostic = Diagnostic::error(
      AnnotationTooGeneral,
      format!("the annotation `{rigid}` is too general"),
      Label::new(self.span.clone()).with_message(format!(
        "this is `{other}`, so the function only works for `{other}`"
      )),
    )
    .with_help(format!(
      "replace `{rigid}` with `{other}`, or remove the annotation"
    ));

    self.secondary(diagnostic, printer, None)
  }

  fn mismatch(
    &self,
    expected: &Type,
    found: &Type,
    branch: Option<&Type>,
    printer: &mut Printer,
  ) -> Diagnostic {
    let label = format!(
      "expected `{}`, found `{}`",
      printer.print(expected),
      printer.print(found)
    );
    let mut diagnostic = Diagnostic::error(
      TypeMismatch,
      "mismatched types",
      Label::new(self.span.clone()).with_message(label),
    );

    if let Some(note) = difference(expected, found, printer) {
      diagnostic = diagnostic.with_note(note);
    }

    if let Some(raw) = &self.int_literal
      && is_con(expected, "Float")
      && is_con(found, "Int")
    {
      diagnostic = diagnostic
        .with_help(format!("write `{}.0` for a Float", raw.replace('_', "")));
    }

    self.secondary(diagnostic, printer, branch)
  }

  fn missing(
    &self,
    label: &Arc<str>,
    record: &Type,
    expected: &Type,
    found: &Type,
    printer: &mut Printer,
  ) -> Diagnostic {
    let mut labels: Vec<Arc<str>> = match (expected, found) {
      (Type::Record(e), Type::Record(f)) => e
        .fields
        .iter()
        .filter(|(n, _)| !f.fields.iter().any(|(m, _)| m == n))
        .map(|(n, _)| n.clone())
        .collect(),
      _ => Vec::new(),
    };

    if !labels.contains(label) {
      labels = vec![label.clone()];
    }

    labels.sort();

    let names = join_names(&labels);
    let message = if labels.len() == 1 {
      format!("missing field {names}")
    } else {
      format!("missing fields {names}")
    };
    let field = if labels.len() == 1 { "field" } else { "fields" };
    let primary = match self.because {
      Because::Access => format!(
        "the type `{}` has no {field} {names}",
        printer.print(&strip(found))
      ),
      _ => format!("this record has no {field} {names}"),
    };
    let mut diagnostic = Diagnostic::error(
      MissingField,
      message,
      Label::new(self.span.clone()).with_message(primary),
    );

    let have: Vec<&str> = match record {
      Type::Record(row) => row.fields.iter().map(|(n, _)| &**n).collect(),
      _ => Vec::new(),
    };

    if let Some(near) = suggest(&labels[0], have) {
      let help = match &self.because {
        Because::Access => format!("did you mean `{near}`?"),
        _ => format!("the record has `{near}`; did you mean `{}`?", labels[0]),
      };

      diagnostic = diagnostic.with_help(help);
    }

    match &self.because {
      Because::Parameter(span) | Because::Annotation(span) => diagnostic
        .with_secondary(Label::new(span.clone()).with_message(format!(
          "{names} {} required here",
          if labels.len() == 1 { "is" } else { "are" }
        ))),
      Because::Nothing
      | Because::Branch(..)
      | Because::Access
      | Because::Trait(..) => diagnostic,
    }
  }

  fn secondary(
    &self,
    diagnostic: Diagnostic,
    printer: &mut Printer,
    branch: Option<&Type>,
  ) -> Diagnostic {
    match &self.because {
      Because::Annotation(span) | Because::Parameter(span) => diagnostic
        .with_secondary(
          Label::new(span.clone()).with_message("expected because of this"),
        ),
      Because::Branch(span, _) => match branch {
        Some(ty) => diagnostic.with_secondary(
          Label::new(span.clone())
            .with_message(format!("this branch is `{}`", printer.print(ty))),
        ),
        None => diagnostic,
      },
      Because::Trait(span, _) => match branch {
        Some(ty) => diagnostic.with_secondary(
          Label::new(span.clone())
            .with_message(format!("the trait says `{}`", printer.print(ty))),
        ),
        None => diagnostic,
      },
      Because::Nothing | Because::Access => diagnostic,
    }
  }
}

fn not_numeric(constraint: Constraint, found: &str, span: &Span) -> Diagnostic {
  let message = match constraint {
    Constraint::Ord => {
      format!("comparison needs `Int`, `Float` or `String`, found `{found}`")
    }
    Constraint::Num | Constraint::None => {
      format!("arithmetic needs `Int` or `Float`, found `{found}`")
    }
  };

  Diagnostic::error(
    NotNumeric,
    message,
    Label::new(span.clone()).with_message(format!("this is `{found}`")),
  )
}

#[must_use]
pub fn not_a_function(ty: &Type, span: &Span, store: &Store) -> Diagnostic {
  let ty = store.zonk(ty);
  let printed = Printer::new(&[&ty], true).print(&ty);

  Diagnostic::error(
    NotAFunction,
    "this is not a function",
    Label::new(span.clone()).with_message(format!("it has type `{printed}`")),
  )
}

#[must_use]
pub fn argument_count(
  expected: usize,
  found: usize,
  func: &Type,
  span: &Span,
  store: &Store,
) -> Diagnostic {
  let func = store.zonk(func);
  let printed = Printer::new(&[&func], true).print(&func);
  let were = if found == 1 { "was" } else { "were" };

  Diagnostic::error(
    ArgumentCount,
    format!(
      "this function takes {} but {found} {were} given",
      arguments(expected)
    ),
    Label::new(span.clone()),
  )
  .with_note(format!("the function has type `{printed}`"))
}

fn arguments(n: usize) -> String {
  if n == 1 { "1 argument".to_string() } else { format!("{n} arguments") }
}

fn join_names(labels: &[Arc<str>]) -> String {
  let quoted: Vec<String> = labels.iter().map(|l| format!("`{l}`")).collect();

  match quoted.as_slice() {
    [] => String::new(),
    [one] => one.clone(),
    [init @ .., last] => format!("{} and {last}", init.join(", ")),
  }
}

fn strip(ty: &Type) -> Type {
  match ty {
    Type::Record(row) => {
      let mut row = row.clone();

      if row.brand().is_some() {
        row.tail = Tail::anonymous();
      }
      Type::Record(row)
    }
    other => other.clone(),
  }
}

fn is_con(ty: &Type, name: &str) -> bool {
  matches!(ty, Type::Con { name: n, args } if &**n == name && args.is_empty())
}

fn compatible(a: &Type, b: &Type) -> bool {
  match (a, b) {
    (Type::Var(_), _) | (_, Type::Var(_)) => true,
    (Type::Con { name: n1, args: a1 }, Type::Con { name: n2, args: a2 }) => {
      n1 == n2
        && a1.len() == a2.len()
        && a1.iter().zip(a2).all(|(x, y)| compatible(x, y))
    }
    (
      Type::Fn { params: p1, ret: r1, .. },
      Type::Fn { params: p2, ret: r2, .. },
    ) => {
      p1.len() == p2.len()
        && p1.iter().zip(p2).all(|(x, y)| compatible(x, y))
        && compatible(r1, r2)
    }
    (Type::Record(e), Type::Record(f)) => {
      e.fields.len() == f.fields.len()
        && e.fields.iter().all(|(n, t)| {
          f.fields.iter().any(|(m, u)| m == n && compatible(t, u))
        })
    }
    _ => a == b,
  }
}

fn difference(
  expected: &Type,
  found: &Type,
  printer: &mut Printer,
) -> Option<String> {
  let differ = |printer: &mut Printer, what: String, e: &Type, f: &Type| {
    difference(e, f, printer).or_else(|| {
      Some(format!(
        "{what} differs: expected `{}`, found `{}`",
        printer.print(e),
        printer.print(f)
      ))
    })
  };

  match (expected, found) {
    (Type::Con { name: n1, args: a1 }, Type::Con { name: n2, args: a2 })
      if n1 == n2 && a1.len() == a2.len() =>
    {
      let i = a1.iter().zip(a2).position(|(e, f)| !compatible(e, f))?;

      differ(
        printer,
        format!("argument {} of `{}`", i + 1, bare(n1)),
        &a1[i],
        &a2[i],
      )
    }
    (
      Type::Fn { params: p1, ret: r1, .. },
      Type::Fn { params: p2, ret: r2, .. },
    ) if p1.len() == p2.len() => {
      match p1.iter().zip(p2).position(|(e, f)| !compatible(e, f)) {
        Some(i) => {
          differ(printer, format!("parameter {}", i + 1), &p1[i], &p2[i])
        }
        None if !compatible(r1, r2) => {
          differ(printer, "the return type".to_string(), r1, r2)
        }
        None => None,
      }
    }
    (Type::Record(e), Type::Record(f)) => {
      let (name, e, f) = e.fields.iter().find_map(|(n, t)| {
        f.fields
          .iter()
          .find(|(m, u)| m == n && !compatible(t, u))
          .map(|(_, u)| (n.clone(), t, u))
      })?;

      differ(printer, format!("field `{name}`"), e, f)
    }
    _ => None,
  }
}
