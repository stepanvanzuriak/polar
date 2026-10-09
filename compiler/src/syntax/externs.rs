use crate::syntax::ast::{Block, Call, Decl, Expr, FnDecl, Module, Name, Var};

pub const PREFIX: &str = "$extern$";

pub fn wrap_exported(module: &mut Module) {
  let decls: Vec<&Decl> = module.zones.iter().flat_map(|z| &z.decls).collect();
  let exported: Vec<String> = decls
    .iter()
    .filter_map(|d| match d {
      Decl::Export(e) if e.methods.is_none() => Some(e.name.text.clone()),
      _ => None,
    })
    .collect();
  let defined: Vec<String> = decls
    .iter()
    .filter_map(|d| match d {
      Decl::Fn(f) => Some(f.name.text.clone()),
      Decl::Const(c) => Some(c.name.text.clone()),
      _ => None,
    })
    .collect();

  for zone in &mut module.zones {
    let mut wrappers = Vec::new();

    for decl in &mut zone.decls {
      let Decl::Extern(e) = decl else { continue };
      let name = e.name.clone();

      if !exported.contains(&name.text) || defined.contains(&name.text) {
        continue;
      }

      e.name.text = format!("{PREFIX}{}", name.text);

      let var = |name: &Name| {
        Expr::Var(Var { span: name.span.clone(), name: name.clone() })
      };
      let call = Expr::Call(Call {
        span: e.span.clone(),
        callee: Box::new(var(&e.name)),
        args: e.params.iter().map(|p| var(&p.name)).collect(),
      });

      wrappers.push(Decl::Fn(FnDecl {
        span: e.span.clone(),
        docs: e.docs.clone(),
        name,
        params: e
          .params
          .iter()
          .map(|p| crate::syntax::ast::Param { pattern: None, ..p.clone() })
          .collect(),
        return_type: Some(e.return_type.clone()),
        effects: e.effects.clone(),
        bounds: Vec::new(),
        body: Block {
          span: e.span.clone(),
          stmts: Vec::new(),
          result: Box::new(call),
        },
      }));
    }

    zone.decls.extend(wrappers);
  }
}

#[must_use]
pub fn source_name(name: &str) -> &str {
  name.strip_prefix(PREFIX).unwrap_or(name)
}
