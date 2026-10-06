use std::{cell::RefCell, collections::HashMap, sync::OnceLock};

use crate::{
  check::{
    self, Types,
    env::{ImplDef, TypeDef},
  },
  core::{
    inline::{InlineBody, exportable},
    lower::lower_expanded,
  },
  shared::diagnostic::DiagnosticBag,
  shared::modules::ModuleSource,
  shared::source::SourceFile,
  syntax::ast::{Decl, Module, TypeBody},
  syntax::lexer::lex,
  syntax::parser::parse,
  types::ty::{Scheme, Type},
};

pub struct StdModule {
  pub name: &'static str,
  pub source: &'static str,
  pub files: &'static [(&'static str, &'static str)],
}

pub const MODULES: &[StdModule] = &[
  StdModule {
    name: "Assert",
    source: include_str!("../../std/Assert.px"),
    files: &[],
  },
  StdModule {
    name: "Dom",
    source: include_str!("../../std/Dom.px"),
    files: &[("bindings/Dom.js", include_str!("../../std/bindings/Dom.js"))],
  },
  StdModule {
    name: "Fs",
    source: include_str!("../../std/Fs.px"),
    files: &[("bindings/Fs.js", include_str!("../../std/bindings/Fs.js"))],
  },
  StdModule {
    name: "Http",
    source: include_str!("../../std/Http.px"),
    files: &[],
  },
  StdModule { name: "Id", source: include_str!("../../std/Id.px"), files: &[] },
  StdModule {
    name: "Json",
    source: include_str!("../../std/Json.px"),
    files: &[],
  },
  StdModule {
    name: "List",
    source: include_str!("../../std/List.px"),
    files: &[],
  },
  StdModule {
    name: "Map",
    source: include_str!("../../std/Map.px"),
    files: &[],
  },
  StdModule {
    name: "Math",
    source: include_str!("../../std/Math.px"),
    files: &[],
  },
  StdModule {
    name: "Option",
    source: include_str!("../../std/Option.px"),
    files: &[],
  },
  StdModule {
    name: "Path",
    source: include_str!("../../std/Path.px"),
    files: &[],
  },
  StdModule {
    name: "Prelude",
    source: include_str!("../../std/Prelude.px"),
    files: &[],
  },
  StdModule {
    name: "Process",
    source: include_str!("../../std/Process.px"),
    files: &[(
      "bindings/Process.js",
      include_str!("../../std/bindings/Process.js"),
    )],
  },
  StdModule {
    name: "Ref",
    source: include_str!("../../std/Ref.px"),
    files: &[],
  },
  StdModule {
    name: "Regex",
    source: include_str!("../../std/Regex.px"),
    files: &[(
      "bindings/Regex.js",
      include_str!("../../std/bindings/Regex.js"),
    )],
  },
  StdModule {
    name: "Result",
    source: include_str!("../../std/Result.px"),
    files: &[],
  },
  StdModule {
    name: "Table",
    source: include_str!("../../std/Table.px"),
    files: &[],
  },
  StdModule {
    name: "Time",
    source: include_str!("../../std/Time.px"),
    files: &[("bindings/Time.js", include_str!("../../std/bindings/Time.js"))],
  },
  StdModule {
    name: "Url",
    source: include_str!("../../std/Url.px"),
    files: &[("bindings/Url.js", include_str!("../../std/bindings/Url.js"))],
  },
];

#[must_use]
pub fn module(name: &str) -> Option<&'static StdModule> {
  MODULES.iter().find(|m| m.name == name)
}

#[must_use]
pub fn filename(name: &str) -> String {
  format!("std/{name}.px")
}

#[must_use]
pub fn compile_options(name: &str, runtime: &str) -> crate::CompileOptions {
  let host = interface(name)
    .filter(|i| i.hosts.len() > 1)
    .and_then(|i| i.hosts.first().cloned());

  crate::CompileOptions {
    runtime: runtime.to_string(),
    host: host
      .map_or(crate::HostOption::Auto, |h| crate::HostOption::Fixed(Some(h))),
    ..crate::CompileOptions::default()
  }
}

#[must_use]
pub fn specifier(runtime: &str, name: &str) -> String {
  let dir = runtime.rsplit_once('/').map_or(".", |(dir, _)| dir);

  format!("{dir}/std/{name}.js")
}

#[derive(Debug, Clone, PartialEq)]
pub struct TraitSig {
  pub name: String,
  pub param: String,
  pub methods: Vec<(String, usize)>,
  pub schemes: Vec<(String, Scheme)>,
  pub recipe: Vec<(String, Scheme)>,
}

impl TraitSig {
  #[must_use]
  pub fn derivable(&self) -> bool {
    !self.recipe.is_empty()
  }

  #[must_use]
  pub fn has_method(&self, name: &str) -> bool {
    self.methods.iter().any(|(m, _)| m == name)
  }
}

#[derive(Debug, Clone, PartialEq)]
pub struct EffectSig {
  pub name: String,
  pub host: Option<String>,
  pub native: bool,
  pub ops: Vec<(String, usize)>,
  pub schemes: Vec<(String, Scheme)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Interface {
  pub functions: Vec<(String, usize)>,
  pub constants: Vec<String>,
  pub ctors: Vec<(String, usize, String)>,
  pub schemes: Vec<(String, Scheme)>,
  pub types: Vec<(String, TypeDef)>,
  pub traits: Vec<TraitSig>,
  pub impls: Vec<ImplDef>,
  pub hosts: Vec<String>,
  pub effects: Vec<EffectSig>,
  pub binds: Vec<(String, String)>,
  pub inline: Vec<InlineBody>,
}

impl Interface {
  #[must_use]
  pub fn function(&self, name: &str) -> Option<usize> {
    self.functions.iter().find(|(n, _)| n == name).map(|&(_, arity)| arity)
  }

  #[must_use]
  pub fn constant(&self, name: &str) -> bool {
    self.constants.iter().any(|n| n == name)
  }

  #[must_use]
  pub fn scheme(&self, name: &str) -> Option<&Scheme> {
    self.schemes.iter().find(|(n, _)| n == name).map(|(_, s)| s)
  }

  #[must_use]
  pub fn trait_sig(&self, name: &str) -> Option<&TraitSig> {
    self.traits.iter().find(|t| t.name == name)
  }

  #[must_use]
  pub fn method_owner(&self, method: &str) -> Option<&TraitSig> {
    self.traits.iter().find(|t| t.has_method(method))
  }
}

static STD: [OnceLock<Interface>; MODULES.len()] =
  [const { OnceLock::new() }; MODULES.len()];

#[must_use]
pub fn interface(name: &str) -> Option<Interface> {
  let index = MODULES.iter().position(|m| m.name == name)?;
  let std = &MODULES[index];
  let cell = &STD[index];

  if let Some(interface) = cell.get() {
    return Some(interface.clone());
  }

  let path = format!("Std.{name}");
  let computed = build(&path, &filename(name), std.source, &[])
    .map_or_else(|_| empty(), |(_, interface)| interface);

  Some(cell.get_or_init(|| computed).clone())
}

#[must_use]
pub fn interface_of(
  filename: &str,
  source: &str,
) -> (Option<String>, Interface) {
  let file = SourceFile::new(filename, source);
  let mut bag = DiagnosticBag::default();
  let module = parse(&file, &lex(&file, &mut bag), &mut bag);
  let path =
    module.name.as_ref().map_or("Module", |n| n.text.as_str()).to_string();

  build(&path, filename, source, &[]).unwrap_or_else(|_| {
    (module.name.as_ref().map(|n| n.text.clone()), shape(&module))
  })
}

#[must_use]
pub fn shape_of(filename: &str, source: &str) -> (Option<String>, Interface) {
  let file = SourceFile::new(filename, source);
  let mut bag = DiagnosticBag::default();
  let module = parse(&file, &lex(&file, &mut bag), &mut bag);

  (module.name.as_ref().map(|n| n.text.clone()), shape(&module))
}

type Cached = Result<(Option<String>, Interface), Vec<String>>;

thread_local! {
  static CHECKING: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
  static CACHE: RefCell<HashMap<(String, String), Cached>> =
    RefCell::new(HashMap::new());
}

pub(crate) fn reset_session() {
  CACHE.with(|cache| cache.borrow_mut().clear());
  CHECKING.with(|stack| stack.borrow_mut().clear());
}

pub(crate) fn within<T>(path: Option<&str>, f: impl FnOnce() -> T) -> T {
  let Some(path) = path else { return f() };

  CHECKING.with(|stack| stack.borrow_mut().push(path.to_string()));

  let out = f();

  CHECKING.with(|stack| stack.borrow_mut().pop());
  out
}

pub(crate) fn user_interface(
  path: &str,
  filename: &str,
  source: &str,
  modules: &[ModuleSource],
) -> Cached {
  let cycle = CHECKING.with(|stack| {
    let stack = stack.borrow();

    stack.iter().position(|p| p == path).map(|start| {
      let mut cycle: Vec<String> = stack[start..].to_vec();

      cycle.push(path.to_string());
      cycle
    })
  });

  if let Some(cycle) = cycle {
    return Err(cycle);
  }

  let key = (path.to_string(), source.to_string());

  if let Some(cached) = CACHE.with(|cache| cache.borrow().get(&key).cloned()) {
    return cached;
  }

  let result = within(Some(path), || build(path, filename, source, modules));

  CACHE.with(|cache| cache.borrow_mut().insert(key, result.clone()));
  result
}

fn build(
  path: &str,
  filename: &str,
  source: &str,
  modules: &[ModuleSource],
) -> Cached {
  let sets = modules
    .iter()
    .find(|m| m.path == path)
    .map(|m| m.plugins.clone())
    .unwrap_or_default();

  crate::syntax::plugins::within(&sets, || {
    build_within(path, filename, source, modules)
  })
}

fn build_within(
  path: &str,
  filename: &str,
  source: &str,
  modules: &[ModuleSource],
) -> Cached {
  let file = SourceFile::new(filename, source);
  let mut bag = DiagnosticBag::default();
  let mut module = parse(&file, &lex(&file, &mut bag), &mut bag);
  let header = module.name.as_ref().map(|n| n.text.clone());

  if bag.has_errors() {
    return Ok((header, shape(&module)));
  }

  let expansion =
    crate::syntax::expand::expand(&mut module, &file, modules, &mut bag);

  if bag.has_errors() {
    return Ok((header, shape(&module)));
  }

  let mut interface = shape(&module);
  let lowered = lower_expanded(&module, modules, &expansion, &mut bag);

  if let Some(cycle) = lowered.cycle {
    return Err(cycle);
  }

  if bag.has_errors() {
    return Ok((header, interface));
  }

  let types = check::check_as(&module, &lowered, Some(path), &mut bag);

  harvest(&mut interface, &types);

  let core = crate::core::annotate::annotate(lowered.core, &types);
  let core = crate::core::dictionaries::elaborate(core, &types, modules);

  interface.inline = exportable(&core, path);
  Ok((header, interface))
}

fn harvest(interface: &mut Interface, types: &Types) {
  let exported = interface
    .functions
    .iter()
    .map(|(name, _)| name)
    .chain(&interface.constants);

  interface.schemes = exported
    .filter_map(|name| {
      let scheme = if types.failed.contains(name) {
        Scheme::new(1, Type::Gen(0))
      } else {
        types.scheme(name)?.clone()
      };

      Some((name.clone(), scheme))
    })
    .collect();
  interface.types.clone_from(&types.type_defs);

  for sig in &mut interface.traits {
    let Some(def) = types
      .traits
      .iter()
      .find(|t| crate::types::print::bare(&t.path) == sig.name)
    else {
      continue;
    };

    sig.schemes.clone_from(&def.methods);

    if !def.recipe.is_empty() {
      sig.recipe.clone_from(&def.recipe);
    }
  }

  interface.impls.clone_from(&types.impls);

  for effect in &mut interface.effects {
    effect.schemes = types
      .op_schemes
      .iter()
      .filter(|((e, _), _)| *e == effect.name)
      .map(|((_, op), scheme)| (op.clone(), scheme.clone()))
      .collect();
    effect.schemes.sort_by(|a, b| a.0.cmp(&b.0));
  }
}

fn empty() -> Interface {
  Interface {
    functions: Vec::new(),
    constants: Vec::new(),
    ctors: Vec::new(),
    schemes: Vec::new(),
    types: Vec::new(),
    traits: Vec::new(),
    impls: Vec::new(),
    hosts: Vec::new(),
    effects: Vec::new(),
    binds: Vec::new(),
    inline: Vec::new(),
  }
}

fn shape(module: &Module) -> Interface {
  let decls: Vec<&Decl> =
    module.zones.iter().flat_map(|zone| &zone.decls).collect();

  let arity = |name: &str| {
    decls.iter().find_map(|decl| match decl {
      Decl::Fn(f) if f.name.text == name => Some(f.params.len()),
      _ => None,
    })
  };

  let trait_decl = |name: &str| {
    decls.iter().find_map(|decl| match decl {
      Decl::Trait(t) if t.name.text == name => Some(t),
      _ => None,
    })
  };

  let traits: Vec<TraitSig> = decls
    .iter()
    .filter_map(|decl| match decl {
      Decl::Export(export) if export.methods.is_some() => {
        trait_decl(&export.name.text)
      }
      _ => None,
    })
    .map(|t| TraitSig {
      name: t.name.text.clone(),
      param: t.param.text.clone(),
      methods: t
        .methods
        .iter()
        .map(|m| (m.name.text.clone(), m.params.len()))
        .collect(),
      schemes: Vec::new(),
      recipe: t
        .recipe
        .iter()
        .flat_map(|r| &r.cases)
        .map(|c| (c.name.text.clone(), Scheme::new(0, Type::unit())))
        .collect(),
    })
    .collect();

  let mut functions: Vec<(String, usize)> = decls
    .iter()
    .filter_map(|decl| match decl {
      Decl::Export(export) => {
        arity(&export.name.text).map(|n| (export.name.text.clone(), n))
      }
      _ => None,
    })
    .collect();

  for t in &traits {
    let Some(decl) = trait_decl(&t.name) else { continue };

    for case in decl.recipe.iter().flat_map(|r| &r.cases) {
      functions.push((recipe_fn(&t.name, &case.name.text), case.params.len()));
    }
  }

  let is_constant = |name: &str| {
    decls
      .iter()
      .any(|decl| matches!(decl, Decl::Const(c) if c.name.text == name))
  };

  let constants = decls
    .iter()
    .filter_map(|decl| match decl {
      Decl::Export(export) if is_constant(&export.name.text) => {
        Some(export.name.text.clone())
      }
      _ => None,
    })
    .collect();

  let ctors = decls
    .iter()
    .filter_map(|decl| match decl {
      Decl::Type(ty) => match &ty.body {
        TypeBody::Variants(v) => Some((ty, v)),
        TypeBody::Alias(_) => None,
      },
      _ => None,
    })
    .flat_map(|(ty, v)| {
      v.ctors
        .iter()
        .map(|c| (c.name.text.clone(), c.args.len(), ty.name.text.clone()))
    })
    .collect();

  let (hosts, effects, binds) = effect_shape(&decls);

  Interface {
    functions,
    constants,
    ctors,
    traits,
    hosts,
    effects,
    binds,
    ..empty()
  }
}

type EffectShape = (Vec<String>, Vec<EffectSig>, Vec<(String, String)>);

fn effect_shape(decls: &[&Decl]) -> EffectShape {
  let exported = |name: &str| {
    decls.iter().any(|decl| {
      matches!(decl, Decl::Export(e) if e.name.text == name && e.methods.is_none())
    })
  };

  let hosts = decls
    .iter()
    .filter_map(|decl| match decl {
      Decl::Host(h) if exported(&h.name.text) => Some(h.name.text.clone()),
      _ => None,
    })
    .collect();

  let effects = decls
    .iter()
    .filter_map(|decl| match decl {
      Decl::Effect(e) if exported(&e.name.text) => Some(EffectSig {
        name: e.name.text.clone(),
        host: e.host.as_ref().map(|h| h.text.clone()),
        native: e.native,
        ops: e
          .ops
          .iter()
          .map(|op| (op.name.text.clone(), op.params.len()))
          .collect(),
        schemes: Vec::new(),
      }),
      _ => None,
    })
    .collect();

  let binds = decls
    .iter()
    .filter_map(|decl| match decl {
      Decl::Bind(b) => Some((b.effect.text.clone(), b.host.text.clone())),
      _ => None,
    })
    .collect();

  (hosts, effects, binds)
}

#[must_use]
pub fn recipe_fn(trait_name: &str, case: &str) -> String {
  format!("$recipe${trait_name}${case}")
}
