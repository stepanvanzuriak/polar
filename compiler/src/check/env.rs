use std::{
  collections::{BTreeMap, HashMap},
  sync::{Arc, OnceLock},
};

use crate::{
  backend::codegen::builtins::{BUILTINS, BuiltinInfo, std_owner},
  shared::diagnostic::DiagnosticBag,
  shared::ice::ice,
  shared::source::SourceFile,
  syntax::ast::{TypeDecl, TypeExpr},
  syntax::lexer::lex,
  syntax::parser::Parser,
  types::ty::{Row, Scheme, Tail, Type},
};

pub const PRIMITIVES: &[&str] = &["Bool", "Float", "Int", "String"];

#[derive(Debug, Clone, PartialEq)]
pub enum TypeDef {
  Variant { name: Arc<str>, arity: usize, ctors: Vec<(String, Scheme)> },
  Alias { name: Arc<str>, arity: usize, body: Type },
}

impl TypeDef {
  #[must_use]
  pub fn arity(&self) -> usize {
    match self {
      TypeDef::Variant { arity, .. } | TypeDef::Alias { arity, .. } => *arity,
    }
  }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ImplDef {
  pub trait_path: Arc<str>,
  pub target: Arc<str>,
  pub arity: usize,
  pub bounds: Vec<(Arc<str>, usize)>,
  pub module: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TraitDef {
  pub path: Arc<str>,
  pub methods: Vec<(String, Scheme)>,
  pub recipe: Vec<(String, Scheme)>,
}

#[derive(Debug, Clone)]
pub(crate) enum Entry<'a> {
  Prim,
  Def(TypeDef),
  Pending(&'a TypeDecl),
  Converting(&'a TypeDecl),
}

#[derive(Debug, Default)]
pub struct TypeEnv<'a> {
  pub(crate) entries: BTreeMap<String, Entry<'a>>,
  pub(crate) ctors: HashMap<String, Scheme>,
  pub(crate) variants: HashMap<Arc<str>, Vec<(String, Scheme)>>,
  pub(crate) traits: HashMap<Arc<str>, TraitDef>,
  pub(crate) impls: HashMap<(Arc<str>, Arc<str>), ImplDef>,
}

impl TypeEnv<'_> {
  pub(crate) fn with_primitives() -> Self {
    let mut env = TypeEnv::default();

    for name in PRIMITIVES {
      env.entries.insert((*name).to_string(), Entry::Prim);
    }

    env
  }

  pub(crate) fn add_def(&mut self, bare: &str, def: TypeDef) {
    if let TypeDef::Variant { name, ctors, .. } = &def {
      for (ctor, scheme) in ctors {
        self.ctors.insert(ctor.clone(), scheme.clone());
      }

      self.variants.insert(name.clone(), ctors.clone());
    }

    self.entries.insert(bare.to_string(), Entry::Def(def));
  }

  pub(crate) fn names(&self) -> impl Iterator<Item = &str> {
    self.entries.keys().map(String::as_str)
  }

  #[must_use]
  pub fn ctors_of(&self, name: &str) -> Option<&[(String, Scheme)]> {
    self.variants.get(name).map(Vec::as_slice)
  }

  #[must_use]
  pub fn ctor(&self, name: &str) -> Option<&Scheme> {
    self.ctors.get(name)
  }
}

#[must_use]
pub fn builtin_type(info: &BuiltinInfo) -> Type {
  static TYPES: OnceLock<Vec<Type>> = OnceLock::new();

  let types = TYPES.get_or_init(|| {
    BUILTINS
      .iter()
      .map(|b| match std_owner(b.module) {
        Some(_) => Type::unit(),
        None => parse_signature(b),
      })
      .collect()
  });
  let index = BUILTINS
    .iter()
    .position(|b| b.module == info.module && b.member == info.member)
    .unwrap_or_else(|| {
      ice(format!("no builtin `{}.{}`", info.module, info.member), None)
    });

  types[index].clone()
}

#[must_use]
pub fn parse_signature(info: &BuiltinInfo) -> Type {
  simple(&signature_expr(info), info)
}

#[must_use]
pub fn signature_expr(info: &BuiltinInfo) -> TypeExpr {
  let file = SourceFile::new("<builtin>", info.signature);
  let mut bag = DiagnosticBag::default();
  let lexed = lex(&file, &mut bag);
  let expr =
    Parser::new(&file, &lexed.tokens, &lexed.comments, &mut bag).type_expr();

  if bag.has_errors() {
    ice(
      format!(
        "the signature of `{}.{}` does not parse: {}",
        info.module, info.member, info.signature
      ),
      None,
    );
  }

  expr
}

fn simple(expr: &TypeExpr, info: &BuiltinInfo) -> Type {
  let bad = || -> ! {
    ice(
      format!(
        "the signature of `{}.{}` may only use primitives: {}",
        info.module, info.member, info.signature
      ),
      None,
    )
  };

  match expr {
    TypeExpr::Ref(r)
      if r.args.is_empty() && PRIMITIVES.contains(&&*r.name.text) =>
    {
      Type::con(&r.name.text)
    }
    TypeExpr::Ref(r) if r.args.len() == 1 => {
      let name = match r.name.text.as_str() {
        "Option" => crate::core::lower::STD_OPTION,
        "List" => "Std.List.List",
        _ => bad(),
      };

      Type::Con { name: name.into(), args: vec![simple(&r.args[0], info)] }
    }
    TypeExpr::Fn(f) => Type::func(
      f.params.iter().map(|p| simple(p, info)).collect(),
      simple(&f.ret, info),
    ),
    TypeExpr::Record(r) if r.tail.is_none() => Type::Record(Row::new(
      r.fields
        .iter()
        .map(|f| (f.name.text.as_str().into(), simple(&f.ty, info)))
        .collect(),
      Tail::anonymous(),
    )),
    TypeExpr::Ref(_)
    | TypeExpr::Var(_)
    | TypeExpr::Record(_)
    | TypeExpr::Invalid(_) => bad(),
  }
}
