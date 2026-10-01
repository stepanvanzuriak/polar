use std::{
  collections::{BTreeMap, BTreeSet, HashSet},
  fmt::Write as _,
};

use crate::{
  CompileOptions,
  backend::codegen::emit::{Colour, colour},
  backend::js::names::js_ident,
  check::{Types, env::TypeDef},
  core::ir::{CDecl, CModule, DeclKind, EffectSlot},
  stdlib,
  types::{
    print::bare,
    ty::{Brand, Effects, Pred, Scheme, Tail, Type},
  },
};

const LETTERS: &[u8; 26] = b"abcdefghijklmnopqrstuvwxyz";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
  Sync,
  Async,
}

#[must_use]
pub fn emit(
  module: &CModule,
  types: &Types,
  options: &CompileOptions,
) -> String {
  let local: HashSet<String> =
    types.type_defs.iter().map(|(name, _)| bare(name).to_string()).collect();
  let mut printer =
    Printer { options, local, imports: BTreeMap::new(), used: BTreeSet::new() };
  let mut body = String::new();

  for (name, def) in &types.type_defs {
    body.push_str(&printer.type_def(name, def));
  }

  for decl in module.decls.iter().filter(|d| d.exported) {
    if decl.sym.name.starts_with('$') {
      continue;
    }

    let Some(scheme) = types.scheme(&decl.sym.name) else { continue };

    match decl.kind {
      DeclKind::Constant => {
        printer.used.clear();

        let ty = printer.ty(&scheme.ty, Mode::Sync);

        let _ = writeln!(
          body,
          "export declare const {}: {ty};",
          js_ident(&decl.sym.name)
        );
      }
      DeclKind::Function => {
        body.push_str(&printer.function(decl, scheme, types));
      }
    }
  }

  let mut out = String::new();

  for (specifier, names) in &printer.imports {
    let names: Vec<&str> = names.iter().map(String::as_str).collect();

    let _ = writeln!(
      out,
      "import type {{ {} }} from \"{specifier}\";",
      names.join(", ")
    );
  }

  if !out.is_empty() && !body.is_empty() {
    out.push('\n');
  }

  out.push_str(&body);
  out
}

struct Printer<'a> {
  options: &'a CompileOptions,
  local: HashSet<String>,
  imports: BTreeMap<String, BTreeSet<String>>,
  used: BTreeSet<u32>,
}

impl Printer<'_> {
  fn type_def(&mut self, name: &str, def: &TypeDef) -> String {
    let name = bare(name);

    match def {
      TypeDef::Variant { arity, ctors, .. } if ctors.is_empty() => format!(
        "export type {name}{} = {{ readonly $opaque: \"{name}\" }};\n\n",
        params(*arity)
      ),
      TypeDef::Variant { arity, ctors, .. } => {
        let cases: Vec<String> = ctors
          .iter()
          .map(|(ctor, scheme)| {
            let mut fields = vec![format!("$: \"{ctor}\"")];

            if let Type::Fn { params, .. } = &scheme.ty {
              for (i, param) in params.iter().enumerate() {
                fields.push(format!("_{i}: {}", self.ty(param, Mode::Sync)));
              }
            }

            format!("{{ {} }}", fields.join("; "))
          })
          .collect();

        format!(
          "export type {name}{} =\n  | {};\n\n",
          params(*arity),
          cases.join("\n  | ")
        )
      }
      TypeDef::Alias { arity, body, .. } => {
        let body = match body {
          Type::Record(row) => self.fields(&row.fields, Mode::Sync),
          other => self.ty(other, Mode::Sync),
        };

        format!("export type {name}{} = {body};\n\n", params(*arity))
      }
    }
  }

  fn function(
    &mut self,
    decl: &CDecl,
    scheme: &Scheme,
    types: &Types,
  ) -> String {
    let name = js_ident(&decl.sym.name).into_owned();
    let signature = |printer: &mut Self, mode: Mode, promise: bool| {
      printer.used.clear();

      let text = printer.signature(decl, scheme, types, mode, promise);
      let generics = printer.generics();

      (generics, text)
    };
    let declare = |name: &str, (generics, text): (String, String)| {
      format!("export declare function {name}{generics}{text};\n")
    };

    match colour(decl.effects.as_ref(), &[]) {
      Colour::Sync => declare(&name, signature(self, Mode::Sync, false)) + "\n",
      Colour::Async => {
        declare(&name, signature(self, Mode::Async, true)) + "\n"
      }
      Colour::Poly => {
        let sync = signature(self, Mode::Sync, false);
        let asynchronous = signature(self, Mode::Async, true);

        declare(&name, sync.clone())
          + &declare(&format!("{name}$sync"), sync)
          + &declare(&format!("{name}$async"), asynchronous)
          + "\n"
      }
    }
  }

  fn signature(
    &mut self,
    decl: &CDecl,
    scheme: &Scheme,
    types: &Types,
    mode: Mode,
    promise: bool,
  ) -> String {
    let Type::Fn { params, ret, .. } = &scheme.ty else {
      return format!("(): {}", self.ty(&scheme.ty, mode));
    };
    let mut types: Vec<String> =
      scheme.preds.iter().map(|pred| self.dictionary(pred, types)).collect();

    types.extend(params.iter().map(|p| self.ty(p, mode)));
    let names = decl.params.iter().map(|p| js_ident(&p.name).into_owned());
    let list: Vec<String> =
      names.zip(types).map(|(name, ty)| format!("{name}: {ty}")).collect();
    let ret = self.ty(ret, mode);
    let ret = if promise { format!("Promise<{ret}>") } else { ret };

    format!("({}): {ret}", list.join(", "))
  }

  fn dictionary(&mut self, pred: &Pred, types: &Types) -> String {
    let Some(methods) = trait_methods(&pred.trait_path, types) else {
      return "unknown".to_string();
    };
    let fields: Vec<String> = methods
      .iter()
      .map(|(name, scheme)| {
        let ty = with_param(&scheme.ty, &pred.ty);

        format!("{name}: {}", self.ty(&ty, Mode::Sync))
      })
      .collect();

    format!("{{ {} }}", fields.join("; "))
  }

  fn generics(&self) -> String {
    if self.used.is_empty() {
      return String::new();
    }

    let names: Vec<String> = self.used.iter().map(|&i| letter(i)).collect();

    format!("<{}>", names.join(", "))
  }

  fn ty(&mut self, ty: &Type, mode: Mode) -> String {
    match ty {
      Type::Var(_) => "unknown".to_string(),
      Type::Gen(i) => {
        self.used.insert(*i);
        letter(*i)
      }
      Type::Rigid { name, .. } => name.to_string(),
      Type::Con { name, args } => self.con(name, args, mode),
      Type::Fn { params, ret, effects } => {
        let params: Vec<String> = params
          .iter()
          .enumerate()
          .map(|(i, p)| format!("p{i}: {}", self.ty(p, mode)))
          .collect();
        let ret = self.ty(ret, mode);
        let ret = match (suspends(effects), mode) {
          (Some(true), _) => format!("Promise<{ret}>"),
          (None, Mode::Async) => format!("{ret} | Promise<{ret}>"),
          (Some(false), _) | (None, Mode::Sync) => ret,
        };

        format!("({}) => {ret}", params.join(", "))
      }
      Type::Record(row) => match &row.tail {
        Tail::Closed(Brand::Named(name)) => self.reference(name, &[], mode),
        Tail::Closed(_) if row.fields.is_empty() => {
          "Record<string, never>".to_string()
        }
        Tail::Closed(_) | Tail::Open(_) | Tail::Rigid(_) => {
          self.fields(&row.fields, mode)
        }
        Tail::Gen(i) => {
          self.used.insert(*i);

          let rest = letter(*i);

          if row.fields.is_empty() {
            rest
          } else {
            format!("{} & {rest}", self.fields(&row.fields, mode))
          }
        }
      },
    }
  }

  fn fields(
    &mut self,
    fields: &[(std::sync::Arc<str>, Type)],
    mode: Mode,
  ) -> String {
    if fields.is_empty() {
      return "Record<string, never>".to_string();
    }

    let fields: Vec<String> = fields
      .iter()
      .map(|(name, ty)| format!("{name}: {}", self.ty(ty, mode)))
      .collect();

    format!("{{ {} }}", fields.join("; "))
  }

  fn con(&mut self, name: &str, args: &[Type], mode: Mode) -> String {
    match name {
      "Int" | "Float" => "number".to_string(),
      "String" => "string".to_string(),
      "Bool" => "boolean".to_string(),
      _ => self.reference(name, args, mode),
    }
  }

  fn reference(&mut self, name: &str, args: &[Type], mode: Mode) -> String {
    let short = bare(name).to_string();
    let module = name.rsplit_once('.').map(|(module, _)| module);
    let resolved = match module {
      None => Some(short.clone()),
      Some(_) if self.local.contains(&short) => None,
      Some(module) => self.specifier(module).map(|specifier| {
        self.imports.entry(specifier).or_default().insert(short.clone());
        short.clone()
      }),
    };
    let Some(resolved) = resolved else {
      return "unknown".to_string();
    };

    if args.is_empty() {
      resolved
    } else {
      let args: Vec<String> = args.iter().map(|a| self.ty(a, mode)).collect();

      format!("{resolved}<{}>", args.join(", "))
    }
  }

  fn specifier(&self, module: &str) -> Option<String> {
    if let Some(std) = module.strip_prefix("Std.") {
      return Some(stdlib::specifier(&self.options.runtime, std));
    }

    self
      .options
      .modules
      .iter()
      .find(|m| m.path == module)
      .map(|m| m.specifier.clone())
  }
}

fn suspends(effects: &Effects) -> Option<bool> {
  match colour(Some(&EffectSlot(effects.clone())), &[]) {
    Colour::Sync => Some(false),
    Colour::Async => Some(true),
    Colour::Poly => None,
  }
}

fn trait_methods(path: &str, types: &Types) -> Option<Vec<(String, Scheme)>> {
  if let Some(def) = types.traits.iter().find(|t| &*t.path == path) {
    return Some(def.methods.clone());
  }

  let (module, name) = path.strip_prefix("Std.")?.rsplit_once('.')?;
  let interface = stdlib::interface(module)?;

  interface.traits.iter().find(|t| t.name == name).map(|t| t.schemes.clone())
}

fn with_param(ty: &Type, param: &Type) -> Type {
  match ty {
    Type::Gen(0) => param.clone(),
    Type::Gen(_) | Type::Var(_) => Type::Var(crate::types::ty::TVar(0)),
    Type::Rigid { .. } => ty.clone(),
    Type::Con { name, args } => Type::Con {
      name: name.clone(),
      args: args.iter().map(|a| with_param(a, param)).collect(),
    },
    Type::Fn { params, ret, effects } => Type::Fn {
      params: params.iter().map(|p| with_param(p, param)).collect(),
      ret: Box::new(with_param(ret, param)),
      effects: effects.clone(),
    },
    Type::Record(row) => Type::Record(crate::types::ty::Row {
      fields: row
        .fields
        .iter()
        .map(|(n, t)| (n.clone(), with_param(t, param)))
        .collect(),
      tail: row.tail.clone(),
    }),
  }
}

fn params(arity: usize) -> String {
  if arity == 0 {
    return String::new();
  }

  let names: Vec<String> =
    (0..arity).map(|i| letter(u32::try_from(i).unwrap_or(0))).collect();

  format!("<{}>", names.join(", "))
}

fn letter(i: u32) -> String {
  let index = i as usize;
  let base = char::from(LETTERS[index % LETTERS.len()]);

  match index / LETTERS.len() {
    0 => base.to_string(),
    n => format!("{base}{n}"),
  }
}
