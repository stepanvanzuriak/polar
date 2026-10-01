use polar_compiler::{
  syntax::ast::TypeExpr,
  types::{
    store::Store,
    ty::{EffTail, Effects, Label, Row, TVar, Tail, Type},
  },
};
use std::sync::Arc;

use super::drive;

/// Remembers which letter maps to which placeholder, so that `a` means the same
/// placeholder everywhere in one test.
#[derive(Default)]
pub struct Names(Vec<(String, TVar)>);

impl Names {
  /// The placeholder called `name`, making a fresh one the first time.
  pub fn get(&mut self, name: &str, store: &mut Store) -> TVar {
    if let Some((_, var)) = self.0.iter().find(|(n, _)| n == name) {
      return *var;
    }

    let var = store.fresh();
    self.0.push((name.to_string(), var));
    var
  }
}

pub fn ty(src: &str, store: &mut Store, names: &mut Names) -> Type {
  let mut parsed = None;
  let diagnostics = drive(src, |parser| parsed = Some(parser.type_expr()));

  assert!(diagnostics.is_empty(), "bad type in test: {src}");

  convert(&parsed.unwrap(), store, names)
}

fn convert(expr: &TypeExpr, store: &mut Store, names: &mut Names) -> Type {
  match expr {
    TypeExpr::Ref(r) => Type::Con {
      name: Arc::from(r.name.text.as_str()),
      args: r.args.iter().map(|a| convert(a, store, names)).collect(),
    },
    TypeExpr::Var(v) => Type::Var(names.get(&v.name.text, store)),
    TypeExpr::Fn(f) => {
      let params = f.params.iter().map(|p| convert(p, store, names)).collect();
      let ret = convert(&f.ret, store, names);
      let effects = match &f.effects {
        None => Effects::pure(),
        Some(row) => {
          let labels = row
            .entries
            .iter()
            .map(|entry| match entry.args.first() {
              Some(TypeExpr::Ref(arg)) if entry.name.text == "Throws" => {
                Label::throws(&arg.name.text)
              }
              _ => Label::effect(&entry.name.text),
            })
            .collect();
          let tail = match &row.tail {
            Some(name) => EffTail::Open(names.get(&name.text, store)),
            None => EffTail::Closed,
          };

          Effects::new(labels, tail)
        }
      };

      Type::func_with(params, ret, effects)
    }
    TypeExpr::Record(r) => Type::Record(Row::new(
      r.fields
        .iter()
        .map(|f| {
          (Arc::from(f.name.text.as_str()), convert(&f.ty, store, names))
        })
        .collect(),
      match &r.tail {
        Some(name) => Tail::Open(names.get(&name.text, store)),
        None => Tail::anonymous(),
      },
    )),
    TypeExpr::Invalid(_) => panic!("invalid type in a test"),
  }
}
