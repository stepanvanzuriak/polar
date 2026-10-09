use std::sync::Arc;

use crate::{
  check::Checker,
  core::lower::Resolved,
  syntax::ast::{Name, PatLit, Pattern},
  types::ty::{Row, Tail, Type},
};

impl<'a> Checker<'a> {
  pub(crate) fn check_pattern(
    &mut self,
    pattern: &'a Pattern,
    expected: &Type,
  ) {
    match pattern {
      Pattern::Wildcard(_) | Pattern::Invalid(_) => {}
      Pattern::Or(or) => {
        if let Some((first, rest)) = or.alternatives.split_first() {
          self.check_pattern(first, expected);
          self.alternatives += 1;

          for alternative in rest {
            self.check_pattern(alternative, expected);
          }

          self.alternatives -= 1;
        }
      }
      Pattern::Var(var) => self.bind_mono(&var.name.span, expected.clone()),
      Pattern::Lit(lit) => {
        let found = match &lit.lit {
          PatLit::Int(_) => Type::int(),
          PatLit::Float(_) => Type::float(),
          PatLit::Bool(_) => Type::bool(),
          PatLit::String(_) => Type::string(),
        };

        self.expect(expected, &found, &lit.span);
      }
      Pattern::Ctor(ctor) => {
        let scheme = match self.resolved(&ctor.name.span) {
          Some(Resolved::Ctor(id)) => self.ctor_scheme(id),
          _ => None,
        };
        let Some(scheme) = scheme else {
          for arg in &ctor.args {
            let fresh = self.fresh();

            self.check_pattern(arg, &fresh);
          }

          return;
        };

        let (params, result) = match self.instantiate(&scheme) {
          Type::Fn { params, ret, .. } => (params, *ret),
          other => (Vec::new(), other),
        };
        let fits = self.expect(expected, &result, &ctor.span);

        for (i, arg) in ctor.args.iter().enumerate() {
          let ty = match params.get(i) {
            Some(param) if fits => param.clone(),
            _ => self.fresh(),
          };

          self.check_pattern(arg, &ty);
        }
      }
      Pattern::Record(record) => {
        let names: Vec<&Name> = record.fields.iter().map(|f| &f.name).collect();

        self.duplicate_fields(&names);

        let mut fields: Vec<(Arc<str>, Type)> = Vec::new();

        for field in &record.fields {
          if !fields.iter().any(|(n, _)| **n == *field.name.text) {
            let fresh = self.fresh();

            fields.push((Arc::from(field.name.text.as_str()), fresh));
          }
        }

        let tail = if record.open {
          Tail::Open(self.fresh_var())
        } else {
          Tail::Closed(self.store.fresh_brand())
        };
        let shape = Type::Record(Row::new(fields.clone(), tail));
        let fits = self.expect(expected, &shape, &record.span);

        for field in &record.fields {
          let ty = match fields.iter().find(|(n, _)| **n == *field.name.text) {
            Some((_, ty)) if fits => ty.clone(),
            _ => self.fresh(),
          };

          self.check_pattern(&field.pattern, &ty);
        }
      }
      Pattern::List(list) => {
        let (elem, list_ty) = self.list_types(&list.span);
        let fits = self.expect(expected, &list_ty, &list.span);

        for item in &list.items {
          let ty = if fits { elem.clone() } else { self.fresh() };

          self.check_pattern(item, &ty);
        }

        if let Some(tail) = &list.tail {
          let ty = if fits { list_ty.clone() } else { self.fresh() };

          self.check_pattern(tail, &ty);
        }
      }
    }
  }
}
