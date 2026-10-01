use crate::{
  shared::modules::ModuleSource,
  shared::source::Span,
  stdlib::{self, Interface, TraitSig},
  syntax::ast::{
    Bound, Builtin, CtorDecl, Decl, Expr, FieldType, FnDecl, ImplDecl, Module,
    Name, Pattern, TypeBody, TypeDecl, TypeExpr, TypeRef, TypeVar,
  },
  syntax::builder::{Gen, Piece},
  syntax::expand::insert_decls,
};

#[derive(Clone)]
enum Shape<'m> {
  Variant(&'m [CtorDecl]),
  Record(&'m [FieldType]),
}

struct Target<'m> {
  decl: &'m TypeDecl,
  shape: Shape<'m>,
}

struct UserTrait {
  qualifier: Option<String>,
  name: String,
  method: String,
  cases: Vec<String>,
}

enum Derivable {
  Eq(String),
  Show(String),
  Json(String),
  User(UserTrait),
}

pub fn expand(
  module: &mut Module,
  modules: &[ModuleSource],
  builder: &mut Gen,
) {
  let local_traits: Vec<String> = decls(module)
    .filter_map(|d| match d {
      Decl::Trait(t) => Some(t.name.text.clone()),
      _ => None,
    })
    .collect();
  let imports = Imports::of(module, modules);
  let mut impls = Vec::new();

  for decl in decls(module) {
    let Decl::Type(ty) = decl else { continue };
    let Some(derive) = &ty.derive else { continue };
    let shape = match &ty.body {
      TypeBody::Variants(v) => Shape::Variant(&v.ctors),
      TypeBody::Alias(TypeExpr::Record(r))
        if ty.params.is_empty() && r.tail.is_none() =>
      {
        Shape::Record(&r.fields)
      }
      TypeBody::Alias(_) => continue,
    };
    let target = Target { decl: ty, shape };

    for name in &derive.names {
      let Some(kind) = derivable(&name.text, module, &local_traits, &imports)
      else {
        continue;
      };

      builder.set_real(&name.span);

      if let Some(imp) = builder.derive(&target, name, &kind) {
        impls.push(Decl::Impl(imp));
      }
    }
  }

  if !impls.is_empty() {
    insert_decls(module, Builtin::Impls, impls, &builder.file);
  }
}

fn decls(module: &Module) -> impl Iterator<Item = &Decl> {
  module.zones.iter().flat_map(|z| &z.decls)
}

struct Imports {
  json: Option<String>,
  prelude: String,
  traits: Vec<(String, TraitSig)>,
}

impl Imports {
  fn of(module: &Module, modules: &[ModuleSource]) -> Self {
    let mut json = None;
    let mut prelude = "Prelude".to_string();
    let mut traits = Vec::new();

    for decl in decls(module) {
      let Decl::Import(import) = decl else { continue };
      let path: Vec<&str> =
        import.path.iter().map(|n| n.text.as_str()).collect();
      let local = import
        .alias
        .as_ref()
        .or(import.path.last())
        .map(|n| n.text.clone())
        .unwrap_or_default();
      let interface = match path.as_slice() {
        ["Std", "Json"] => {
          json = Some(local.clone());
          continue;
        }
        ["Std", "Prelude"] => {
          prelude.clone_from(&local);
          continue;
        }
        ["Std", name] => stdlib::interface(name),
        _ => user_interface(&path.join("."), modules),
      };

      for sig in interface.map(|i| i.traits).unwrap_or_default() {
        traits.push((local.clone(), sig));
      }
    }

    Self { json, prelude, traits }
  }
}

fn user_interface(path: &str, modules: &[ModuleSource]) -> Option<Interface> {
  let module = modules.iter().find(|m| m.path == path)?;
  let file = crate::shared::modules::file(path);

  stdlib::user_interface(path, &file, &module.source, modules)
    .ok()
    .map(|(_, interface)| interface)
}

fn derivable(
  name: &str,
  module: &Module,
  local_traits: &[String],
  imports: &Imports,
) -> Option<Derivable> {
  if let Some(t) = decls(module).find_map(|d| match d {
    Decl::Trait(t) if t.name.text == name => Some(t),
    _ => None,
  }) {
    let recipe = t.recipe.as_ref()?;
    let method = t.methods.first()?;

    return Some(Derivable::User(UserTrait {
      qualifier: None,
      name: name.to_string(),
      method: method.name.text.clone(),
      cases: recipe.cases.iter().map(|c| c.name.text.clone()).collect(),
    }));
  }

  if let Some((local, sig)) =
    imports.traits.iter().find(|(_, t)| t.name == name)
  {
    if !sig.derivable() {
      return None;
    }

    return Some(Derivable::User(UserTrait {
      qualifier: Some(local.clone()),
      name: name.to_string(),
      method: sig.methods.first()?.0.clone(),
      cases: sig.recipe.iter().map(|(c, _)| c.clone()).collect(),
    }));
  }

  let prelude = module.name.as_ref().is_none_or(|n| n.text != "Prelude")
    && !local_traits.iter().any(|t| t == name);

  match name {
    "Eq" if prelude => Some(Derivable::Eq(imports.prelude.clone())),
    "Show" if prelude => Some(Derivable::Show(imports.prelude.clone())),
    "Json" => imports.json.clone().map(Derivable::Json),
    _ => None,
  }
}

impl Gen {
  fn use_site(
    &mut self,
    owner: &str,
    trait_name: &str,
    real: &Span,
    callee: Expr,
  ) -> Expr {
    let Expr::Field(field) = &callee else { return callee };
    let span = &field.field.span;

    self.expansion.derived.insert(
      (span.start, span.end),
      (owner.to_string(), trait_name.to_string()),
    );
    let real = self.resolve(real);

    self.expansion.real.insert(span.start, real);
    callee
  }

  fn derive(
    &mut self,
    target: &Target<'_>,
    name: &Name,
    kind: &Derivable,
  ) -> Option<ImplDecl> {
    let (trait_name, methods) = match kind {
      Derivable::Eq(prelude) => {
        ("Eq".to_string(), vec![self.eq(target, prelude)])
      }
      Derivable::Show(prelude) => {
        ("Show".to_string(), vec![self.show(target, prelude)])
      }
      Derivable::Json(module) => (
        "Json".to_string(),
        vec![self.encoder(target, module), self.decoder(target, module)],
      ),
      Derivable::User(user) => {
        let wanted = match target.shape {
          Shape::Record(_) => "record",
          Shape::Variant(_) => "variant",
        };

        if !user.cases.iter().any(|c| c == wanted) {
          return None;
        }

        (user.name.clone(), vec![self.recipe(target, user)])
      }
    };
    let decl = target.decl;
    let args = decl
      .params
      .iter()
      .map(|p| {
        TypeExpr::Var(TypeVar { span: self.span(), name: self.name(&p.text) })
      })
      .collect();
    let bounds = decl
      .params
      .iter()
      .map(|p| Bound {
        span: self.span(),
        trait_name: self.name(&trait_name),
        var: self.name(&p.text),
      })
      .collect();
    let trait_name = Name { text: trait_name, span: self.span() };
    let target_ref =
      TypeRef { span: self.span(), name: self.name(&decl.name.text), args };
    let _ = name;

    Some(ImplDecl {
      span: self.span(),
      docs: Vec::new(),
      trait_name,
      target: target_ref,
      bounds,
      methods,
    })
  }

  fn eq(&mut self, target: &Target<'_>, prelude: &str) -> FnDecl {
    let owner = target.decl.name.text.clone();
    let body = match &target.shape {
      Shape::Record(fields) => {
        let checks = fields
          .iter()
          .map(|f| {
            self.at(&f.span, |g| {
              let callee = g.qualified(prelude, "eq");
              let callee = g.use_site(&owner, "Eq", &f.span, callee);
              let x = g.var("$x");
              let y = g.var("$y");
              let left = g.field(x, &f.name.text);
              let right = g.field(y, &f.name.text);

              g.call(callee, vec![left, right])
            })
          })
          .collect();

        self.all(checks)
      }
      Shape::Variant(ctors) => {
        let mut arms = Vec::new();

        for ctor in *ctors {
          let lefts: Vec<String> =
            (0..ctor.args.len()).map(|i| format!("$a{i}")).collect();
          let rights: Vec<String> =
            (0..ctor.args.len()).map(|i| format!("$b{i}")).collect();
          let l_args = lefts.iter().map(|n| self.p_var(n)).collect();
          let r_args = rights.iter().map(|n| self.p_var(n)).collect();
          let l = self.p_ctor(&ctor.name.text, l_args);
          let r = self.p_ctor(&ctor.name.text, r_args);
          let pattern = self.p_record(vec![("l", l), ("r", r)]);
          let checks = ctor
            .args
            .iter()
            .enumerate()
            .map(|(i, arg)| {
              self.at(arg.span(), |g| {
                let callee = g.qualified(prelude, "eq");
                let callee = g.use_site(&owner, "Eq", arg.span(), callee);
                let left = g.var(&lefts[i]);
                let right = g.var(&rights[i]);

                g.call(callee, vec![left, right])
              })
            })
            .collect();
          let body = self.all(checks);

          arms.push((pattern, body));
        }

        if ctors.len() > 1 {
          let wildcard = self.p_wildcard();
          let no = self.boolean(false);

          arms.push((wildcard, no));
        }

        let x = self.var("$x");
        let y = self.var("$y");
        let pair = self.record(vec![("l", x), ("r", y)]);

        self.match_expr(pair, arms)
      }
    };

    self.method("eq", &["$x", "$y"], body)
  }

  fn shown(
    &mut self,
    prelude: &str,
    owner: &str,
    ty: &TypeExpr,
    real: &Span,
    value: Expr,
  ) -> Vec<Piece> {
    self.at(real, |g| {
      let callee = g.qualified(prelude, "show");
      let callee = g.use_site(owner, "Show", real, callee);
      let call = g.call(callee, vec![value]);
      let quoted = matches!(ty, TypeExpr::Ref(r) if r.name.text == "String" && r.args.is_empty());

      if quoted {
        vec![Piece::Text("\"".to_string()), Piece::Expr(call), Piece::Text("\"".to_string())]
      } else {
        vec![Piece::Expr(call)]
      }
    })
  }

  fn show(&mut self, target: &Target<'_>, prelude: &str) -> FnDecl {
    let owner = target.decl.name.text.clone();
    let body = match &target.shape {
      Shape::Record([]) => self.text("{}"),
      Shape::Record(fields) => {
        let mut pieces = vec![Piece::Text("{ ".to_string())];

        for (i, f) in fields.iter().enumerate() {
          let sep = if i == 0 { "" } else { ", " };

          pieces.push(Piece::Text(format!("{sep}{}: ", f.name.text)));

          let value = self.var("$value");
          let value = self.field(value, &f.name.text);

          pieces.extend(self.shown(prelude, &owner, &f.ty, &f.span, value));
        }

        pieces.push(Piece::Text(" }".to_string()));
        self.string(pieces)
      }
      Shape::Variant(ctors) => {
        let mut arms = Vec::new();

        for ctor in *ctors {
          let names: Vec<String> =
            (0..ctor.args.len()).map(|i| format!("$a{i}")).collect();
          let args = names.iter().map(|n| self.p_var(n)).collect();
          let pattern = self.p_ctor(&ctor.name.text, args);
          let body = if ctor.args.is_empty() {
            self.text(&ctor.name.text)
          } else {
            let mut pieces = vec![Piece::Text(format!("{}(", ctor.name.text))];

            for (i, arg) in ctor.args.iter().enumerate() {
              if i > 0 {
                pieces.push(Piece::Text(", ".to_string()));
              }

              let value = self.var(&names[i]);

              pieces.extend(self.shown(
                prelude,
                &owner,
                arg,
                arg.span(),
                value,
              ));
            }

            pieces.push(Piece::Text(")".to_string()));
            self.string(pieces)
          };

          arms.push((pattern, body));
        }

        let value = self.var("$value");

        self.match_expr(value, arms)
      }
    };

    self.method("show", &["$value"], body)
  }

  fn json_call(
    &mut self,
    module: &str,
    member: &str,
    owner: &str,
    real: &Span,
  ) -> Expr {
    self.at(real, |g| {
      let callee = g.qualified(module, member);

      g.use_site(owner, "Json", real, callee)
    })
  }

  fn encoder(&mut self, target: &Target<'_>, module: &str) -> FnDecl {
    let owner = target.decl.name.text.clone();
    let body = match &target.shape {
      Shape::Record(fields) => {
        let mut entries = Vec::new();

        for f in *fields {
          let callee = self.json_call(module, "to_json", &owner, &f.span);
          let value = self.var("$value");
          let access = self.field(value, &f.name.text);
          let encoded = self.call(callee, vec![access]);
          let key = self.text(&f.name.text);

          entries.push(self.record(vec![("key", key), ("value", encoded)]));
        }

        let list = self.std_list(entries);
        let object = self.qualified(module, "object");

        self.call(object, vec![list])
      }
      Shape::Variant(ctors) => {
        let mut arms = Vec::new();

        for ctor in *ctors {
          let names: Vec<String> =
            (0..ctor.args.len()).map(|i| format!("$a{i}")).collect();
          let args = names.iter().map(|n| self.p_var(n)).collect();
          let pattern = self.p_ctor(&ctor.name.text, args);
          let mut values = Vec::new();

          for (i, arg) in ctor.args.iter().enumerate() {
            let callee = self.json_call(module, "to_json", &owner, arg.span());
            let value = self.var(&names[i]);

            values.push(self.call(callee, vec![value]));
          }

          let tag = self.text(&ctor.name.text);
          let list = self.std_list(values);
          let tagged = self.qualified(module, "tagged");

          arms.push((pattern, self.call(tagged, vec![tag, list])));
        }

        let value = self.var("$value");

        self.match_expr(value, arms)
      }
    };

    self.method("to_json", &["$value"], body)
  }

  fn attempt(&mut self, scrutinee: Expr, ok: Pattern, body: Expr) -> Expr {
    let error = self.p_var("$error");
    let failed = self.p_std_ctor("Err", vec![error]);
    let error = self.var("$error");
    let forwarded = self.std_ctor_call("Err", error);
    let succeeded = self.p_std_ctor("Ok", vec![ok]);

    self.match_expr(scrutinee, vec![(failed, forwarded), (succeeded, body)])
  }

  fn decode_fields(
    &mut self,
    module: &str,
    owner: &str,
    fields: &str,
    keys: &[(String, Span)],
    done: impl FnOnce(&mut Self, Vec<String>) -> Expr,
  ) -> Expr {
    let names: Vec<String> =
      (0..keys.len()).map(|i| format!("$f{i}")).collect();
    let mut body = done(self, names.clone());

    for (i, (key, real)) in keys.iter().enumerate().rev() {
      let callee = self.json_call(module, "field", owner, real);
      let fields = self.var(fields);
      let path = self.var("$path");
      let key = self.text(key);
      let decoded = self.call(callee, vec![fields, path, key]);
      let bound = self.p_var(&names[i]);

      body = self.attempt(decoded, bound, body);
    }

    body
  }

  fn decoder(&mut self, target: &Target<'_>, module: &str) -> FnDecl {
    let owner = target.decl.name.text.clone();
    let body = match &target.shape {
      Shape::Record(fields) => {
        let keys: Vec<(String, Span)> = fields
          .iter()
          .map(|f| (f.name.text.clone(), f.span.clone()))
          .collect();
        let names: Vec<String> =
          fields.iter().map(|f| f.name.text.clone()).collect();
        let inner =
          self.decode_fields(module, &owner, "$fields", &keys, |g, vars| {
            let values = names
              .iter()
              .zip(&vars)
              .map(|(n, v)| (n.as_str(), g.var(v)))
              .collect();
            let record = g.record(values);

            g.std_ctor_call("Ok", record)
          });
        let callee = self.qualified(module, "object_fields");
        let value = self.var("$value");
        let path = self.var("$path");
        let decoded = self.call(callee, vec![value, path]);
        let bound = self.p_var("$fields");

        self.attempt(decoded, bound, inner)
      }
      Shape::Variant(ctors) => {
        let expected =
          ctors.iter().map(|c| c.name.text.as_str()).collect::<Vec<_>>();
        let expected = match expected.as_slice() {
          [] => String::new(),
          [one] => (*one).to_string(),
          [init @ .., last] => format!("{} or {last}", init.join(", ")),
        };
        let mut arms = Vec::new();

        for ctor in *ctors {
          let pattern = self.p_string(&ctor.name.text);
          let keys: Vec<(String, Span)> = ctor
            .args
            .iter()
            .enumerate()
            .map(|(i, a)| (format!("_{i}"), a.span().clone()))
            .collect();
          let ctor_name = ctor.name.text.clone();
          let body =
            self.decode_fields(module, &owner, "$fields", &keys, |g, vars| {
              let value = if vars.is_empty() {
                g.var(&ctor_name)
              } else {
                let callee = g.var(&ctor_name);
                let args = vars.iter().map(|v| g.var(v)).collect();

                g.call(callee, args)
              };

              g.std_ctor_call("Ok", value)
            });

          arms.push((pattern, body));
        }

        let wildcard = self.p_wildcard();
        let unknown = self.qualified(module, "unknown_variant");
        let path = self.var("$path");
        let expected = self.text(&expected);
        let tag = self.var("$tag");

        arms.push((wildcard, self.call(unknown, vec![path, expected, tag])));

        let tag = self.var("$tag");
        let matched = self.match_expr(tag, arms);
        let callee = self.qualified(module, "variant");
        let value = self.var("$value");
        let path = self.var("$path");
        let decoded = self.call(callee, vec![value, path]);
        let tag = self.p_var("$tag");
        let fields = self.p_var("$fields");
        let bound = self.p_record(vec![("tag", tag), ("fields", fields)]);

        self.attempt(decoded, bound, matched)
      }
    };

    self.method("from_json", &["$value", "$path"], body)
  }

  fn recipe_ref(&mut self, user: &UserTrait, case: &str) -> Expr {
    let member = stdlib::recipe_fn(&user.name, case);

    match &user.qualifier {
      Some(module) => self.qualified(module, &member),
      None => self.var(&member),
    }
  }

  fn user_method(
    &mut self,
    user: &UserTrait,
    owner: &str,
    real: &Span,
  ) -> Expr {
    self.at(real, |g| {
      if let Some(module) = &user.qualifier {
        let callee = g.qualified(module, &user.method);

        g.use_site(owner, &user.name, real, callee)
      } else {
        let callee = g.var(&user.method);

        if let Expr::Var(v) = &callee {
          let span = &v.name.span;

          g.expansion.derived.insert(
            (span.start, span.end),
            (owner.to_string(), user.name.clone()),
          );
        }

        callee
      }
    })
  }

  fn recipe(&mut self, target: &Target<'_>, user: &UserTrait) -> FnDecl {
    let owner = target.decl.name.text.clone();
    let body = match &target.shape {
      Shape::Record(fields) => {
        let mut entries = Vec::new();

        for f in *fields {
          let callee = self.user_method(user, &owner, &f.span);
          let value = self.var("$value");
          let access = self.field(value, &f.name.text);
          let converted = self.call(callee, vec![access]);
          let name = self.text(&f.name.text);

          entries.push(self.record(vec![("name", name), ("value", converted)]));
        }

        let list = self.std_list(entries);
        let case = self.recipe_ref(user, "record");

        self.call(case, vec![list])
      }
      Shape::Variant(ctors) => {
        let mut arms = Vec::new();

        for ctor in *ctors {
          let names: Vec<String> =
            (0..ctor.args.len()).map(|i| format!("$a{i}")).collect();
          let args = names.iter().map(|n| self.p_var(n)).collect();
          let pattern = self.p_ctor(&ctor.name.text, args);
          let mut values = Vec::new();

          for (i, arg) in ctor.args.iter().enumerate() {
            let callee = self.user_method(user, &owner, arg.span());
            let value = self.var(&names[i]);

            values.push(self.call(callee, vec![value]));
          }

          let name = self.text(&ctor.name.text);
          let list = self.std_list(values);
          let case = self.recipe_ref(user, "variant");

          arms.push((pattern, self.call(case, vec![name, list])));
        }

        let value = self.var("$value");

        self.match_expr(value, arms)
      }
    };

    self.method(&user.method, &["$value"], body)
  }
}
