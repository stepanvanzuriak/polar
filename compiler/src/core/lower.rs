use crate::{
  backend::codegen::builtins,
  core::{
    effects::EffectTable,
    ir::{
      CArm, CDecl, CExpr, CExprKind, CImpl, CModule, CPattern, CtorId,
      CtorInfo, DeclKind, Lit, PrimOp, Sym, SymGen,
    },
    scope::{Scopes, suggest},
  },
  shared::codes::DiagnosticCode::{
    self, BuiltinModuleAsValue, CallArity, ConstantCalled, ConstantUsesBelow,
    ConstructorArity, DuplicateDefinition, DuplicateImpl, ImportCycle,
    ImportNotSupported, IncompleteTraitExport, ListTypeNotInScope,
    MissingMethod, NotDerivable, NumberOutOfRange, OrphanImpl,
    UnknownBuiltinMember, UnknownMethod, UnknownModule, UnknownName,
    UnknownStdModule, UnknownTrait, UnknownUppercaseName,
  },
  shared::diagnostic::{Diagnostic, DiagnosticBag, Label},
  shared::ice::ice,
  shared::modules::{self, ModuleSource},
  shared::source::Span,
  stdlib::{self, Interface},
  syntax::ast::{
    Binary, BinaryOp, Block, Bound, Call, ConstDecl, Decl, Else, ExportDecl,
    Expr, FieldAccess, FnDecl, If, ImplDecl, Import, IntLit, Lambda, ListLit,
    Match, MatchArm, Module, Name, Param, PatLit, Pattern, Stmt, StringLit,
    StringPart, TraitDecl, Try, TypeBody, TypeDecl, TypeExpr, Unary, UnaryOp,
    Var,
  },
  syntax::builder::Expansion,
};
use std::{
  collections::{BTreeMap, HashMap, HashSet},
  sync::Arc,
};

const MAX_SAFE_INTEGER: u64 = (1 << 53) - 1;

#[derive(Debug, Clone, PartialEq)]
pub enum Resolved {
  Local(Sym),
  Top(Sym),
  Ctor(CtorId),
  List { nil: CtorId, cons: CtorId },
  Builtin { module: &'static str, member: &'static str },
  Imported { module: String, member: String },
  Trait(String),
  Method { trait_path: String, method: String },
  StdList { nil: CtorId, cons: CtorId },
  Operation { effect: String, op: String },
  Extern(String),
}

pub const STD_RESULT: &str = "Std.Result.Result";
pub const STD_OPTION: &str = "Std.Option.Option";

#[derive(Debug, Default)]
pub struct Resolutions(pub HashMap<(usize, usize), Resolved>);

impl Resolutions {
  #[must_use]
  pub fn get(&self, span: &Span) -> Option<&Resolved> {
    self.0.get(&(span.start, span.end))
  }

  pub(super) fn insert(&mut self, span: &Span, resolved: Resolved) {
    self.0.insert((span.start, span.end), resolved);
  }
}

#[derive(Debug)]
pub struct Lowered {
  pub core: CModule,
  pub resolutions: Resolutions,
  pub interfaces: BTreeMap<String, Interface>,
  pub cycle: Option<Vec<String>>,
  pub derived: HashMap<(usize, usize), (String, String)>,
  pub effects: EffectTable,
}

pub fn lower(module: &Module, diagnostics: &mut DiagnosticBag) -> CModule {
  lower_with(module, &[], diagnostics).core
}

pub fn lower_with(
  module: &Module,
  modules: &[ModuleSource],
  diagnostics: &mut DiagnosticBag,
) -> Lowered {
  lower_expanded(module, modules, &Expansion::default(), diagnostics)
}

pub fn lower_expanded(
  module: &Module,
  modules: &[ModuleSource],
  expansion: &Expansion,
  diagnostics: &mut DiagnosticBag,
) -> Lowered {
  let mut lowerer = Lowerer {
    diagnostics,
    modules,
    syms: SymGen::default(),
    scopes: Scopes::default(),
    fns: BTreeMap::new(),
    consts: BTreeMap::new(),
    lowering_const: None,
    ctors: Vec::new(),
    ctor_ids: BTreeMap::new(),
    imports: BTreeMap::new(),
    std_used: Vec::new(),
    user_used: Vec::new(),
    resolutions: Resolutions::default(),
    interfaces: BTreeMap::new(),
    cycle: None,
    traits: BTreeMap::new(),
    methods: BTreeMap::new(),
    impl_heads: Vec::new(),
    local_types: Vec::new(),
    std_lists: expansion.std_lists.clone(),
    std_list_ctors: None,
    std_ctors: expansion.std_ctors.clone(),
    std_result_ctors: None,
    std_option_ctors: None,
    module_name: module.name.as_ref().map(|n| n.text.clone()),
    effects: EffectTable::default(),
    in_bind: false,
  };
  for name in &expansion.std_types {
    if let Some(interface) = stdlib::interface(name) {
      lowerer.interfaces.entry(format!("Std.{name}")).or_insert(interface);
    }
  }

  let core = lowerer.module(module);

  Lowered {
    core,
    resolutions: lowerer.resolutions,
    interfaces: lowerer.interfaces,
    cycle: lowerer.cycle,
    derived: expansion.derived.clone(),
    effects: lowerer.effects,
  }
}

pub(super) struct TopFn {
  pub(super) sym: Sym,
  pub(super) arity: usize,
  pub(super) span: Span,
}

pub(super) struct TopConst {
  pub(super) sym: Sym,
  pub(super) index: usize,
  pub(super) span: Span,
}

#[derive(Debug, Clone)]
pub(super) struct TraitInfo {
  path: String,
  methods: Vec<String>,
  param: Option<String>,
  sigs: Vec<(Vec<(String, TypeExpr)>, TypeExpr)>,
  span: Span,
  local: bool,
  cases: Option<Vec<String>>,
}

#[derive(Debug, Clone)]
pub(super) struct MethodInfo {
  trait_path: String,
  span: Span,
  imported: bool,
}

#[derive(Debug, Clone)]
pub(super) enum Origin {
  Std(String),
  User(String),
}

impl Origin {
  fn name(&self) -> &str {
    match self {
      Origin::Std(name) | Origin::User(name) => name,
    }
  }

  pub(super) fn path(&self) -> String {
    match self {
      Origin::Std(name) => format!("Std.{name}"),
      Origin::User(path) => path.clone(),
    }
  }

  fn member(&self, member: &str) -> CExprKind {
    let member = member.to_string();

    match self {
      Origin::Std(module) => CExprKind::Std { module: module.clone(), member },
      Origin::User(module) => {
        CExprKind::User { module: module.clone(), member }
      }
    }
  }
}

pub(super) struct Lowerer<'a> {
  pub(super) diagnostics: &'a mut DiagnosticBag,
  pub(super) modules: &'a [ModuleSource],
  pub(super) syms: SymGen,
  pub(super) scopes: Scopes,
  pub(super) fns: BTreeMap<String, TopFn>,
  pub(super) consts: BTreeMap<String, TopConst>,
  pub(super) lowering_const: Option<usize>,
  pub(super) ctors: Vec<CtorInfo>,
  pub(super) ctor_ids: BTreeMap<String, (CtorId, Span)>,
  pub(super) imports: BTreeMap<String, (Origin, Interface)>,
  pub(super) std_used: Vec<String>,
  pub(super) user_used: Vec<(String, String)>,
  pub(super) resolutions: Resolutions,
  pub(super) interfaces: BTreeMap<String, Interface>,
  pub(super) cycle: Option<Vec<String>>,
  pub(super) traits: BTreeMap<String, TraitInfo>,
  pub(super) methods: BTreeMap<String, MethodInfo>,
  pub(super) impl_heads: Vec<((String, String), Span)>,
  pub(super) local_types: Vec<String>,
  pub(super) std_lists: HashSet<(usize, usize)>,
  pub(super) std_list_ctors: Option<(CtorId, CtorId)>,
  pub(super) std_ctors: HashSet<(usize, usize)>,
  pub(super) std_result_ctors: Option<(CtorId, CtorId)>,
  pub(super) std_option_ctors: Option<(CtorId, CtorId)>,
  pub(super) module_name: Option<String>,
  pub(super) effects: EffectTable,
  pub(super) in_bind: bool,
}

enum Step {
  Bind(Option<Sym>, CExpr, Span),
  Match(CExpr, CPattern, Span),
}

impl Lowerer<'_> {
  fn module(&mut self, module: &Module) -> CModule {
    let decls: Vec<&Decl> =
      module.zones.iter().flat_map(|zone| &zone.decls).collect();
    let mut exports = Vec::new();

    if !is_prelude(module) && !imports_prelude(&decls) {
      self.implicit_prelude();
    }

    self.local_types = decls
      .iter()
      .filter_map(|decl| match decl {
        Decl::Type(ty) => Some(ty.name.text.clone()),
        _ => None,
      })
      .collect();

    for decl in &decls {
      match decl {
        Decl::Import(import) => self.import(import),
        Decl::Trait(t) => self.declare_trait(t),
        Decl::Type(ty) => {
          self.not_a_builtin_module(&ty.name, "type");

          if let TypeBody::Variants(variants) = &ty.body {
            for ctor in &variants.ctors {
              self.declare_ctor(&ctor.name, ctor.args.len(), &ty.name.text);
            }
          }
        }
        Decl::Const(c) => self.declare_const(c),
        Decl::Fn(f) => self.declare_fn(f),
        Decl::Impl(_)
        | Decl::Host(_)
        | Decl::Effect(_)
        | Decl::Extern(_)
        | Decl::Bind(_)
        | Decl::Plugin(_) => {}
        Decl::Export(export) => exports.push(export),
      }
    }

    self.imported_method_clashes(&decls);
    self.declare_effects(&decls);

    for decl in &decls {
      match decl {
        Decl::Type(ty) => {
          for name in ty.derive.iter().flat_map(|d| &d.names) {
            if let Some(info) = self.resolve_trait(name) {
              self.derivable(ty, name, &info);
            }
          }
        }
        Decl::Fn(f) => self.resolve_bounds(&f.bounds),
        _ => {}
      }
    }

    let consts = decls.iter().filter_map(|decl| match decl {
      Decl::Const(c) => Some(c),
      _ => None,
    });
    let mut out: Vec<CDecl> =
      consts.enumerate().map(|(index, c)| self.const_decl(c, index)).collect();

    out.extend(decls.iter().filter_map(|decl| match decl {
      Decl::Fn(f) => Some(self.fn_decl(f)),
      Decl::Import(_)
      | Decl::Trait(_)
      | Decl::Type(_)
      | Decl::Const(_)
      | Decl::Impl(_)
      | Decl::Export(_)
      | Decl::Host(_)
      | Decl::Effect(_)
      | Decl::Extern(_)
      | Decl::Bind(_)
      | Decl::Plugin(_) => None,
    }));

    self.recipes(&decls, &mut out);

    let binds = self.binds(&decls);
    let impls = decls
      .iter()
      .filter_map(|decl| match decl {
        Decl::Impl(imp) => self.impl_decl(imp),
        _ => None,
      })
      .collect();

    for export in &exports {
      if !self.export_effect_or_host(export) {
        self.export(export, &mut out);
      }
    }

    self.check_effect_exports(&exports);

    CModule {
      decls: out,
      impls,
      binds,
      bridges: Vec::new(),
      host: None,
      externs: self.externs(),
      bind_refs: Vec::new(),
      ctors: std::mem::take(&mut self.ctors),
      std_imports: std::mem::take(&mut self.std_used),
      user_imports: std::mem::take(&mut self.user_used),
    }
  }

  fn recipes(&mut self, decls: &[&Decl], out: &mut Vec<CDecl>) {
    for decl in decls {
      if let Decl::Trait(t) = decl {
        for case in t.recipe.iter().flat_map(|r| &r.cases) {
          let name = stdlib::recipe_fn(&t.name.text, &case.name.text);

          if self.fns.contains_key(&name) {
            let mut lowered = self.fn_named(case, &name);

            lowered.exported = true;
            out.push(lowered);
          }
        }
      }
    }
  }

  fn implicit_prelude(&mut self) {
    let Some(interface) = stdlib::interface("Prelude") else { return };
    let span = Span::empty(Arc::from("<prelude>"), 0);
    let name = Name { text: "Prelude".to_string(), span: span.clone() };
    let import =
      Import { span, path: vec![name.clone()], alias: None, methods: None };

    self.bind_import(&import, &name, Origin::Std("Prelude".into()), interface);
  }

  fn export(&mut self, export: &ExportDecl, out: &mut [CDecl]) {
    let name = &export.name;

    if let Some(listed) = &export.methods {
      let Some(info) = self.traits.get(&name.text).filter(|t| t.local).cloned()
      else {
        return self.error(
          IncompleteTraitExport,
          format!(
            "`{}` is not a trait; only traits have method lists",
            name.text
          ),
          name.span.clone(),
          Some(format!("write `{}` without the braces", name.text)),
        );
      };

      self.resolutions.insert(&name.span, Resolved::Trait(info.path.clone()));

      for method in listed {
        if !info.methods.contains(&method.text) {
          let help =
            suggest(&method.text, info.methods.iter().map(String::as_str))
              .map(|s| format!("did you mean `{s}`?"));

          self.error(
            UnknownMethod,
            format!(
              "the trait `{}` has no method `{}`",
              name.text, method.text
            ),
            method.span.clone(),
            help,
          );
        }
      }

      let missing: Vec<&String> = info
        .methods
        .iter()
        .filter(|m| !listed.iter().any(|l| l.text == **m))
        .collect();

      if !missing.is_empty() {
        let names: Vec<String> =
          missing.iter().map(|m| format!("`{m}`")).collect();

        self.error(
          IncompleteTraitExport,
          format!("exporting `{}` must list all its methods", name.text),
          export.span.clone(),
          Some(format!(
            "add {}: write `{} {{ {} }}`",
            names.join(", "),
            name.text,
            info.methods.join(", ")
          )),
        );
      }

      return;
    }

    if is_upper(&name.text) {
      match self.traits.get(&name.text).filter(|t| t.local).cloned() {
        Some(info) => {
          self.resolutions.insert(&name.span, Resolved::Trait(info.path));
          self.error(
            IncompleteTraitExport,
            format!("exporting `{}` must list all its methods", name.text),
            export.span.clone(),
            Some(format!(
              "write `{} {{ {} }}`",
              name.text,
              info.methods.join(", ")
            )),
          );
        }
        None if self.local_types.contains(&name.text) => self.error(
          UnknownName,
          format!("`{}` is a type, and types are always exported", name.text),
          name.span.clone(),
          Some("remove it from `exports`".to_string()),
        ),
        None => self.unknown_name(name, "function or constant", "to export"),
      }

      return;
    }

    match out.iter_mut().find(|decl| *decl.sym.name == *name.text) {
      Some(decl) => decl.exported = true,
      None => self.unknown_name(name, "function or constant", "to export"),
    }
  }

  fn declare_trait(&mut self, t: &TraitDecl) {
    let name = &t.name;

    self.not_a_builtin_module(name, "trait");

    if let Some(first) = self.traits.get(&name.text) {
      let first = first.span.clone();

      self.duplicate(name, "trait", first);
      return;
    }

    let mut methods = Vec::new();

    for method in &t.methods {
      let m = &method.name;

      if let Some(first) = self.top_span(&m.text) {
        self.duplicate(m, "method", first);
        continue;
      }

      self.methods.insert(
        m.text.clone(),
        MethodInfo {
          trait_path: name.text.clone(),
          span: m.span.clone(),
          imported: false,
        },
      );
      methods.push(m.text.clone());
    }

    for case in t.recipe.iter().flat_map(|r| &r.cases) {
      if !matches!(case.name.text.as_str(), "record" | "variant") {
        continue;
      }

      let fn_name = stdlib::recipe_fn(&name.text, &case.name.text);

      if self.fns.contains_key(&fn_name) {
        continue;
      }

      let sym = self.syms.fresh(&fn_name);

      self.fns.insert(
        fn_name,
        TopFn { sym, arity: case.params.len(), span: case.name.span.clone() },
      );
    }

    let builtin = matches!(
      (self.module_name.as_deref(), name.text.as_str()),
      (Some("Prelude"), "Eq" | "Show") | (Some("Json"), "Json")
    );
    let cases = if builtin {
      Some(vec!["record".to_string(), "variant".to_string()])
    } else {
      t.recipe
        .as_ref()
        .map(|r| r.cases.iter().map(|c| c.name.text.clone()).collect())
    };

    self.traits.insert(
      name.text.clone(),
      TraitInfo {
        path: name.text.clone(),
        methods,
        param: Some(t.param.text.clone()),
        sigs: t
          .methods
          .iter()
          .map(|m| {
            let params = m
              .params
              .iter()
              .filter_map(|p| Some((p.name.text.clone(), p.ty.clone()?)))
              .collect();

            (params, m.return_type.clone())
          })
          .collect(),
        span: name.span.clone(),
        local: true,
        cases,
      },
    );
  }

  fn imported_method_clashes(&mut self, decls: &[&Decl]) {
    let imported: Vec<(String, Span)> = self
      .methods
      .iter()
      .filter(|(_, m)| m.imported)
      .map(|(name, m)| (name.clone(), m.span.clone()))
      .collect();

    for (name, at) in imported {
      let local = self
        .fns
        .get(&name)
        .map(|f| f.span.clone())
        .or_else(|| self.consts.get(&name).map(|c| c.span.clone()));

      if let Some(local) = local {
        self.diagnostics.push(
          Diagnostic::error(
            DuplicateDefinition,
            format!(
              "`{name}` is imported here and also defined in this module"
            ),
            Label::new(at),
          )
          .with_secondary(Label::new(local).with_message("defined here"))
          .with_help(format!(
            "remove `{name}` from the list, or rename the function"
          )),
        );
      }
    }

    let _ = decls;
  }

  fn resolve_trait(&mut self, name: &Name) -> Option<TraitInfo> {
    if let Some(info) = self.traits.get(&name.text) {
      let info = info.clone();

      self.resolutions.insert(&name.span, Resolved::Trait(info.path.clone()));
      return Some(info);
    }

    let help = if name.text == "Json" {
      Some("add `uses Std.Json`".to_string())
    } else {
      suggest(&name.text, self.traits.keys().map(String::as_str))
        .map(|s| format!("did you mean `{s}`?"))
    };

    self.error(
      UnknownTrait,
      format!("cannot find trait `{}`", name.text),
      name.span.clone(),
      help,
    );

    None
  }

  fn derivable(&mut self, ty: &TypeDecl, name: &Name, info: &TraitInfo) {
    let Some(cases) = &info.cases else {
      return self.error(
        NotDerivable,
        format!(
          "`{}` can't be derived: it has no `derive {{ … }}` recipe",
          name.text
        ),
        name.span.clone(),
        Some("write the impl in the `impls` zone instead".to_string()),
      );
    };
    let (case, shape) = match &ty.body {
      TypeBody::Variants(_) => ("variant", "a variant"),
      TypeBody::Alias(_) => ("record", "a record"),
    };

    if !cases.iter().any(|c| c == case) {
      self.error(
        NotDerivable,
        format!(
          "`{}` can't be derived for {shape}: its recipe has no `{case}` case",
          name.text
        ),
        name.span.clone(),
        Some(format!(
          "add a `{case}` case to the `derive {{ … }}` recipe of `{}`",
          name.text
        )),
      );
    }
  }

  fn resolve_bounds(&mut self, bounds: &[Bound]) {
    for bound in bounds {
      self.resolve_trait(&bound.trait_name);
    }
  }

  fn missing_methods(
    &mut self,
    imp: &ImplDecl,
    info: &TraitInfo,
    header: &Span,
  ) {
    for (i, method) in info.methods.iter().enumerate() {
      let misspelt = imp.methods.iter().any(|m| {
        !info.methods.contains(&m.name.text)
          && suggest(&m.name.text, [method.as_str()]).is_some()
      });

      if !misspelt && !imp.methods.iter().any(|m| m.name.text == *method) {
        let signature = match (&info.param, info.sigs.get(i)) {
          (Some(param), Some((params, ret))) => {
            let target = type_text(&TypeExpr::Ref(imp.target.clone()), "", "");
            let params: Vec<String> = params
              .iter()
              .map(|(n, t)| format!("{n}: {}", type_text(t, param, &target)))
              .collect();

            format!(
              "{method}({}) -> {}",
              params.join(", "),
              type_text(ret, param, &target)
            )
          }
          _ => format!("{method}(…)"),
        };

        self.error(
          MissingMethod,
          format!("this impl is missing `{method}`"),
          header.clone(),
          Some(format!("add `{signature} {{ … }}`")),
        );
      }
    }
  }

  fn impl_decl(&mut self, imp: &ImplDecl) -> Option<CImpl> {
    let info = self.resolve_trait(&imp.trait_name)?;
    let header = imp.trait_name.span.join(&imp.target.span);
    let target = &imp.target.name.text;

    self.resolve_bounds(&imp.bounds);

    self.missing_methods(imp, &info, &header);

    let mut seen: Vec<&Name> = Vec::new();

    for method in &imp.methods {
      if !info.methods.contains(&method.name.text) {
        let help =
          suggest(&method.name.text, info.methods.iter().map(String::as_str))
            .map(|s| format!("did you mean `{s}`?"));

        self.error(
          UnknownMethod,
          format!(
            "the trait `{}` has no method `{}`",
            imp.trait_name.text, method.name.text
          ),
          method.name.span.clone(),
          help,
        );
      }

      if let Some(first) = seen.iter().find(|n| n.text == method.name.text) {
        let first = first.span.clone();

        self.duplicate(&method.name, "method", first);
      }

      seen.push(&method.name);
    }

    let local_type = self.local_types.contains(target);

    if !info.local && !local_type {
      self.diagnostics.push(
        Diagnostic::error(
          OrphanImpl,
          format!(
            "this impl must live next to `{}` or `{target}`",
            imp.trait_name.text
          ),
          Label::new(header.clone()),
        )
        .with_help(format!(
          "move this impl into the module that declares `{}` or the one \
           that declares `{target}`",
          imp.trait_name.text
        )),
      );

      return None;
    }

    let key = (info.path.clone(), target.clone());

    if let Some((_, first)) = self.impl_heads.iter().find(|(k, _)| *k == key) {
      let first = first.clone();

      self.diagnostics.push(
        Diagnostic::error(
          DuplicateImpl,
          format!("`{target}` already has a `{}` impl", imp.trait_name.text),
          Label::new(header),
        )
        .with_secondary(Label::new(first).with_message("the first one")),
      );

      return None;
    }

    self.impl_heads.push((key, header.clone()));

    let methods = imp
      .methods
      .iter()
      .filter(|m| info.methods.contains(&m.name.text))
      .map(|m| {
        let name = m.name.text.clone();

        self.fn_named(m, &name)
      })
      .collect();

    Some(CImpl {
      trait_path: info.path,
      target: target.clone(),
      bounds: imp.bounds.len(),
      methods,
      origin: Some(header),
    })
  }

  fn import(&mut self, import: &Import) {
    if import.path.first().is_some_and(|first| first.text != "Std") {
      return self.user_import(import);
    }

    let [_, name] = import.path.as_slice() else {
      return self.import_not_supported(import);
    };

    if builtins::is_module(&name.text) {
      if let Some(alias) = &import.alias {
        self.error(
          ImportNotSupported,
          format!("the builtin module `{}` cannot be renamed", name.text),
          alias.span.clone(),
          Some(format!("use it as `{}`", name.text)),
        );
      }

      return;
    }

    let Some(interface) = stdlib::interface(&name.text) else {
      let available: Vec<String> =
        stdlib::MODULES.iter().map(|m| format!("`Std.{}`", m.name)).collect();

      return self.error(
        UnknownStdModule,
        format!("there is no std module `Std.{}`", name.text),
        name.span.clone(),
        Some(format!("available: {}", available.join(", "))),
      );
    };

    let local = import.alias.as_ref().unwrap_or(name);

    if self.bind_import(
      import,
      local,
      Origin::Std(name.text.clone()),
      interface,
    ) && !self.std_used.contains(&name.text)
    {
      self.std_used.push(name.text.clone());
    }
  }

  fn user_import(&mut self, import: &Import) {
    let Some(last) = import.path.last() else { return };
    let path: Vec<&str> = import.path.iter().map(|n| n.text.as_str()).collect();
    let path = path.join(".");
    let file = modules::file(&path);
    let span = import.path[0].span.join(&last.span);

    let Some(module) = self.modules.iter().find(|m| m.path == path) else {
      return self.error(
        UnknownModule,
        format!("there is no module `{path}`"),
        span,
        Some(format!("expected it in `{file}`, under the source root")),
      );
    };
    let specifier = module.specifier.clone();
    let (header, interface) = match stdlib::user_interface(
      &path,
      &file,
      &module.source,
      self.modules,
    ) {
      Ok(found) => found,
      Err(cycle) => {
        let message = format!("import cycle: {}", cycle.join(" → "));

        self.error(
          ImportCycle,
          message,
          span.clone(),
          Some("move what both modules need into a third module".to_string()),
        );
        self.cycle = Some(cycle);
        stdlib::shape_of(&file, &module.source)
      }
    };

    if let Some(header) = header.filter(|h| *h != last.text) {
      return self.error(
        UnknownModule,
        format!(
          "`{file}` declares `module {header}`, not `module {}`",
          last.text
        ),
        span,
        Some(format!("rename it `module {}`", last.text)),
      );
    }

    let local = import.alias.as_ref().unwrap_or(last);

    if self.bind_import(import, local, Origin::User(path.clone()), interface)
      && !self.user_used.iter().any(|(p, _)| *p == path)
    {
      self.user_used.push((path, specifier));
    }
  }

  fn bind_import(
    &mut self,
    import: &Import,
    local: &Name,
    origin: Origin,
    interface: Interface,
  ) -> bool {
    if builtins::is_module(&local.text) {
      self.error(
        DuplicateDefinition,
        format!("`{}` is already the name of a builtin module", local.text),
        local.span.clone(),
        Some("import it `as` another name".to_string()),
      );
      return false;
    }

    if self.imports.contains_key(&local.text) {
      self.error(
        DuplicateDefinition,
        format!("`{}` is imported more than once", local.text),
        local.span.clone(),
        None,
      );
      return false;
    }

    for (ctor, arity, owner) in &interface.ctors {
      let ctor = Name { text: ctor.clone(), span: import.span.clone() };

      self.declare_ctor(&ctor, *arity, owner);
    }

    for sig in &interface.traits {
      let path = format!("{}.{}", origin.path(), sig.name);

      if let Some(first) = self.traits.get(&sig.name) {
        let first = first.span.clone();
        let name = Name { text: sig.name.clone(), span: local.span.clone() };

        self.duplicate(&name, "trait", first);
        continue;
      }

      let builtin = matches!(
        path.as_str(),
        "Std.Prelude.Eq" | "Std.Prelude.Show" | "Std.Json.Json"
      );
      let cases = if builtin {
        Some(vec!["record".to_string(), "variant".to_string()])
      } else if sig.derivable() {
        Some(sig.recipe.iter().map(|(c, _)| c.clone()).collect())
      } else {
        None
      };

      self.traits.insert(
        sig.name.clone(),
        TraitInfo {
          path,
          methods: sig.methods.iter().map(|(m, _)| m.clone()).collect(),
          param: None,
          sigs: Vec::new(),
          span: local.span.clone(),
          local: false,
          cases,
        },
      );
    }

    for method in import.methods.iter().flatten() {
      let Some(sig) = interface.method_owner(&method.text) else {
        let available: Vec<&str> = interface
          .traits
          .iter()
          .flat_map(|t| &t.methods)
          .map(|(m, _)| m.as_str())
          .collect();
        let help = suggest(&method.text, available.iter().copied())
          .map(|s| format!("did you mean `{s}`?"));

        self.error(
          UnknownBuiltinMember,
          format!("`{}` exports no method `{}`", local.text, method.text),
          method.span.clone(),
          help,
        );
        continue;
      };

      if let Some(first) = self.methods.get(&method.text) {
        let first = first.span.clone();

        self.duplicate(method, "method", first);
        continue;
      }

      self.methods.insert(
        method.text.clone(),
        MethodInfo {
          trait_path: format!("{}.{}", origin.path(), sig.name),
          span: method.span.clone(),
          imported: true,
        },
      );
    }

    self.import_effects(&origin.path(), &interface, &local.span);
    self.interfaces.insert(origin.path(), interface.clone());
    self.imports.insert(local.text.clone(), (origin, interface));

    true
  }

  fn import_not_supported(&mut self, import: &Import) {
    self.error(
      ImportNotSupported,
      "import one module of `Std` at a time",
      import.span.clone(),
      Some("for example `Std.List`".to_string()),
    );
  }

  fn std_import(&self, expr: &Expr) -> Option<&(Origin, Interface)> {
    match expr {
      Expr::Var(var) => self.imports.get(&var.name.text),
      _ => None,
    }
  }

  fn declare_const(&mut self, c: &ConstDecl) {
    let name = &c.name;

    if let Some(first) = self.top_span(&name.text) {
      self.duplicate(name, "constant", first);
      return;
    }

    let sym = self.syms.fresh(&name.text);
    let index = self.consts.len();

    self.consts.insert(
      name.text.clone(),
      TopConst { sym, index, span: name.span.clone() },
    );
  }

  fn declare_fn(&mut self, f: &FnDecl) {
    let name = &f.name;

    if let Some(first) = self.top_span(&name.text) {
      self.duplicate(name, "function", first);
      return;
    }

    let sym = self.syms.fresh(&name.text);

    self.fns.insert(
      name.text.clone(),
      TopFn { sym, arity: f.params.len(), span: name.span.clone() },
    );
  }

  fn top_names(&self) -> impl Iterator<Item = &str> {
    self
      .fns
      .keys()
      .filter(|name| !name.starts_with('$'))
      .chain(self.consts.keys())
      .chain(self.methods.keys())
      .map(String::as_str)
  }

  pub(super) fn top_span(&self, name: &str) -> Option<Span> {
    let func = self.fns.get(name).map(|f| &f.span);
    let method =
      || self.methods.get(name).filter(|m| !m.imported).map(|m| &m.span);

    func
      .or_else(|| self.consts.get(name).map(|c| &c.span))
      .or_else(method)
      .cloned()
  }

  fn declare_ctor(&mut self, name: &Name, arity: usize, owner: &str) {
    self.not_a_builtin_module(name, "constructor");

    if let Some((_, first)) = self.ctor_ids.get(&name.text) {
      let first = first.clone();

      self.duplicate(name, "constructor", first);
      return;
    }

    let id = CtorId(u32::try_from(self.ctors.len()).unwrap_or(u32::MAX));

    self.ctors.push(CtorInfo {
      name: name.text.clone(),
      arity,
      owner: owner.to_string(),
    });
    self.ctor_ids.insert(name.text.clone(), (id, name.span.clone()));
  }

  pub(super) fn not_a_builtin_module(&mut self, name: &Name, what: &str) {
    if builtins::is_module(&name.text) {
      self.error(
        DuplicateDefinition,
        format!("the {what} `{}` has the name of a builtin module", name.text),
        name.span.clone(),
        Some("choose another name".to_string()),
      );
    }
  }

  fn fn_decl(&mut self, f: &FnDecl) -> CDecl {
    let name = f.name.text.clone();

    self.fn_named(f, &name)
  }

  pub(super) fn fn_named(&mut self, f: &FnDecl, name: &str) -> CDecl {
    let sym = match self.fns.get(name) {
      Some(top) if top.span == f.name.span => top.sym.clone(),
      _ => self.syms.fresh(name),
    };

    self.scopes.push();

    let params = self.params(&f.params);
    let body = self.block(&f.body);

    self.scopes.pop();

    CDecl {
      sym,
      kind: DeclKind::Function,
      exported: false,
      params,
      body,
      origin: Some(f.name.span.clone()),
      effects: None,
    }
  }

  fn const_decl(&mut self, c: &ConstDecl, index: usize) -> CDecl {
    let sym = match self.consts.get(&c.name.text) {
      Some(top) if top.index == index => top.sym.clone(),
      _ => self.syms.fresh(&c.name.text),
    };

    self.lowering_const = Some(index);
    self.scopes.push();

    let body = self.expr(&c.value);

    self.scopes.pop();
    self.lowering_const = None;

    CDecl {
      sym,
      kind: DeclKind::Constant,
      exported: false,
      params: Vec::new(),
      body,
      origin: Some(c.name.span.clone()),
      effects: None,
    }
  }

  fn params(&mut self, params: &[Param]) -> Vec<Sym> {
    let mut seen: Vec<&Name> = Vec::new();

    params
      .iter()
      .map(|param| {
        if let Some(first) = seen.iter().find(|n| n.text == param.name.text) {
          let first = first.span.clone();

          self.duplicate(&param.name, "parameter", first);
        }

        seen.push(&param.name);

        let sym = self.bind(&param.name);

        self.resolutions.insert(&param.name.span, Resolved::Local(sym.clone()));
        sym
      })
      .collect()
  }

  fn bind(&mut self, name: &Name) -> Sym {
    let sym = self.syms.fresh(&name.text);

    self.scopes.bind(&name.text, sym.clone());

    sym
  }

  fn expr(&mut self, expr: &Expr) -> CExpr {
    let origin = Some(expr.span().clone());

    let kind = match expr {
      Expr::Int(lit) => return self.int(lit),
      Expr::Float(lit) => return self.float(&lit.raw, &lit.span),
      Expr::Bool(lit) => CExprKind::Lit(Lit::Bool(lit.value)),
      Expr::String(lit) => return self.string(lit),
      Expr::Var(var) => return self.var(var),
      Expr::Field(field) => return self.field(field),
      Expr::Call(call) => return self.call(call),
      Expr::Pipe(pipe) => {
        let left = self.expr(&pipe.left);
        let stage = pipe.right.span().clone();

        return match &*pipe.right {
          Expr::Call(call) => {
            let mut args = vec![left];

            args.extend(call.args.iter().map(|arg| self.expr(arg)));
            self.apply(&call.callee, args, stage)
          }
          other => self.apply(other, vec![left], stage),
        };
      }
      Expr::Binary(binary) => self.binary(binary),
      Expr::Unary(unary) => self.unary(unary),
      Expr::Record(record) => {
        let fields = record
          .fields
          .iter()
          .map(|field| (field.name.text.clone(), self.expr(&field.value)))
          .collect();

        match &record.spread {
          Some(base) => {
            CExprKind::Update { base: Box::new(self.expr(base)), fields }
          }
          None => CExprKind::Record { fields },
        }
      }
      Expr::List(list) => return self.list(list),
      Expr::Lambda(lambda) => self.lambda(lambda),
      Expr::Block(block) => return self.block(block),
      Expr::If(node) => return self.if_expr(node),
      Expr::Match(node) => self.match_expr(node),
      Expr::Throw(node) => CExprKind::Throw {
        value: Box::new(self.expr(&node.value)),
        tag: String::new(),
      },
      Expr::Try(node) => self.try_expr(node),
      Expr::Invalid(_) => {
        ice("an `InvalidExpr` reached lowering", expr.span().into())
      }
    };

    CExpr::new(kind, origin)
  }

  fn list(&mut self, list: &ListLit) -> CExpr {
    let origin = Some(list.span.clone());
    let items: Vec<CExpr> = list.items.iter().map(|i| self.expr(i)).collect();
    let tail = list.tail.as_ref().map(|t| self.expr(t));

    let (nil, cons) =
      if self.std_lists.contains(&(list.span.start, list.span.end)) {
        let (nil, cons) = self.std_list_ctors();

        self.resolutions.insert(&list.span, Resolved::StdList { nil, cons });
        (nil, cons)
      } else {
        let Some((nil, cons)) = self.list_ctors(&list.span) else {
          return placeholder(origin);
        };

        self.resolutions.insert(&list.span, Resolved::List { nil, cons });
        (nil, cons)
      };

    let end = CExprKind::Ctor { ctor: nil, args: vec![] };
    let mut acc = tail.unwrap_or_else(|| CExpr::new(end, origin.clone()));

    for item in items.into_iter().rev() {
      let kind = CExprKind::Ctor { ctor: cons, args: vec![item, acc] };

      acc = CExpr::new(kind, origin.clone());
    }

    acc
  }

  fn std_list_ctors(&mut self) -> (CtorId, CtorId) {
    if let Some(ids) = self.std_list_ctors {
      return ids;
    }

    let imported = self
      .imports
      .values()
      .any(|(origin, _)| matches!(origin, Origin::Std(name) if name == "List"));
    let known = |name: &str| {
      self
        .ctor_ids
        .get(name)
        .map(|(id, _)| *id)
        .filter(|id| self.ctors[id.0 as usize].owner == "List")
    };

    let ids = if let (true, Some(nil), Some(cons)) =
      (imported, known("Nil"), known("Cons"))
    {
      (nil, cons)
    } else {
      let mut push = |name: &str, arity| {
        let id = CtorId(u32::try_from(self.ctors.len()).unwrap_or(u32::MAX));

        self.ctors.push(CtorInfo {
          name: name.to_string(),
          arity,
          owner: "List".to_string(),
        });
        id
      };

      (push("Nil", 0), push("Cons", 2))
    };

    self.std_list_ctors = Some(ids);
    ids
  }

  fn ctor_named(&mut self, name: &Name) -> Option<(CtorId, Span)> {
    if self.std_ctors.contains(&(name.span.start, name.span.end)) {
      let id = match name.text.as_str() {
        "None" => self.std_option_ctors().0,
        "Some" => self.std_option_ctors().1,
        "Err" => self.std_result_ctors().0,
        _ => self.std_result_ctors().1,
      };

      return Some((id, name.span.clone()));
    }

    self.ctor_ids.get(&name.text).map(|(id, span)| (*id, span.clone()))
  }

  fn std_result_ctors(&mut self) -> (CtorId, CtorId) {
    if let Some(ids) = self.std_result_ctors {
      return ids;
    }

    let imported = self.imports.values().any(
      |(origin, _)| matches!(origin, Origin::Std(name) if name == "Result"),
    );
    let known = |name: &str| {
      self
        .ctor_ids
        .get(name)
        .map(|(id, _)| *id)
        .filter(|id| self.ctors[id.0 as usize].owner == "Result")
    };

    let ids = if let (true, Some(err), Some(ok)) =
      (imported, known("Err"), known("Ok"))
    {
      (err, ok)
    } else {
      if let Some(interface) = stdlib::interface("Result") {
        self.interfaces.entry("Std.Result".to_string()).or_insert(interface);
      }

      let mut push = |name: &str| {
        let id = CtorId(u32::try_from(self.ctors.len()).unwrap_or(u32::MAX));

        self.ctors.push(CtorInfo {
          name: name.to_string(),
          arity: 1,
          owner: STD_RESULT.to_string(),
        });
        id
      };

      (push("Err"), push("Ok"))
    };

    self.std_result_ctors = Some(ids);
    ids
  }

  fn std_option_ctors(&mut self) -> (CtorId, CtorId) {
    if let Some(ids) = self.std_option_ctors {
      return ids;
    }

    let imported = self.imports.values().any(
      |(origin, _)| matches!(origin, Origin::Std(name) if name == "Option"),
    );
    let known = |name: &str| {
      self
        .ctor_ids
        .get(name)
        .map(|(id, _)| *id)
        .filter(|id| self.ctors[id.0 as usize].owner == "Option")
    };

    let ids = if let (true, Some(none), Some(some)) =
      (imported, known("None"), known("Some"))
    {
      (none, some)
    } else {
      if let Some(interface) = stdlib::interface("Option") {
        self.interfaces.entry("Std.Option".to_string()).or_insert(interface);
      }

      let mut push = |name: &str, arity| {
        let id = CtorId(u32::try_from(self.ctors.len()).unwrap_or(u32::MAX));

        self.ctors.push(CtorInfo {
          name: name.to_string(),
          arity,
          owner: STD_OPTION.to_string(),
        });
        id
      };

      (push("None", 0), push("Some", 1))
    };

    self.std_option_ctors = Some(ids);
    ids
  }

  fn list_ctors(&mut self, span: &Span) -> Option<(CtorId, CtorId)> {
    let ctor = |name: &str| {
      self
        .ctor_ids
        .get(name)
        .map(|(id, _)| *id)
        .filter(|id| self.ctors[id.0 as usize].owner == "List")
    };

    if let (Some(nil), Some(cons)) = (ctor("Nil"), ctor("Cons")) {
      return Some((nil, cons));
    }

    self.error(
      ListTypeNotInScope,
      "list syntax needs the `List` type",
      span.clone(),
      Some("add `uses Std.List`".to_string()),
    );

    None
  }

  fn int(&mut self, lit: &IntLit) -> CExpr {
    let digits: String = lit.raw.chars().filter(|&c| c != '_').collect();

    let value = match digits.parse::<u64>() {
      #[allow(clippy::cast_precision_loss, reason = "at most 2^53 − 1, exact")]
      Ok(n) if n <= MAX_SAFE_INTEGER => n as f64,
      _ => {
        self.error(
          NumberOutOfRange,
          format!("integer literal `{}` is too large", lit.raw),
          lit.span.clone(),
          Some(format!("integers are exact up to {MAX_SAFE_INTEGER} in M0")),
        );
        0.0
      }
    };

    CExpr::new(CExprKind::Lit(Lit::Number(value)), Some(lit.span.clone()))
  }

  fn float(&mut self, raw: &str, span: &Span) -> CExpr {
    let value = self.float_value(raw, span);

    CExpr::new(CExprKind::Lit(Lit::Number(value)), Some(span.clone()))
  }

  fn float_value(&mut self, raw: &str, span: &Span) -> f64 {
    let digits: String = raw.chars().filter(|&c| c != '_').collect();

    match digits.parse::<f64>() {
      Ok(value) if value.is_finite() => value,
      _ => {
        self.error(
          NumberOutOfRange,
          format!("float literal `{raw}` is out of range"),
          span.clone(),
          None,
        );
        0.0
      }
    }
  }

  fn string(&mut self, lit: &StringLit) -> CExpr {
    let origin = Some(lit.span.clone());
    let interpolated =
      lit.parts.iter().any(|part| matches!(part, StringPart::Interp(_)));

    if !interpolated {
      let text: String = lit
        .parts
        .iter()
        .map(|part| match part {
          StringPart::Text(text) => text.value.as_str(),
          StringPart::Interp(_) => "",
        })
        .collect();

      return CExpr::new(CExprKind::Lit(Lit::String(text)), origin);
    }

    let parts = lit
      .parts
      .iter()
      .map(|part| match part {
        StringPart::Text(text) => CExpr::new(
          CExprKind::Lit(Lit::String(text.value.clone())),
          Some(text.span.clone()),
        ),
        StringPart::Interp(interp) => self.expr(&interp.expr),
      })
      .collect();

    CExpr::new(CExprKind::Concat { parts }, origin)
  }

  fn var(&mut self, var: &Var) -> CExpr {
    let origin = Some(var.span.clone());
    let name = &var.name.text;

    if !is_upper(name) {
      if let Some(sym) = self.scopes.lookup(name) {
        let sym = sym.clone();

        self.resolutions.insert(&var.name.span, Resolved::Local(sym.clone()));
        return CExpr::new(CExprKind::Var(sym), origin);
      }

      if let Some(method) = self.methods.get(name) {
        let trait_path = method.trait_path.clone();

        return self.method(&var.name.span, trait_path, name, origin);
      }

      if let Some(top) = self.consts.get(name) {
        let sym = top.sym.clone();

        if self.lowering_const.is_some_and(|current| top.index >= current) {
          self.constant_uses_below(&var.name, "the constant");
        }

        self.resolutions.insert(&var.name.span, Resolved::Top(sym.clone()));
        return CExpr::new(CExprKind::Var(sym), origin);
      }

      if let Some(top) = self.fns.get(name) {
        let sym = top.sym.clone();

        if self.lowering_const.is_some() {
          self.constant_uses_below(&var.name, "the function");
        } else if self.in_bind && !name.starts_with('$') {
          self.bind_uses_function(&var.name);
        }

        self.resolutions.insert(&var.name.span, Resolved::Top(sym.clone()));
        return CExpr::new(CExprKind::Var(sym), origin);
      }

      if let Some(found) = self.extern_ref(&var.name, origin.clone()) {
        return found;
      }

      self.unknown_name(&var.name, "", "in this scope");
      return placeholder(origin);
    }

    if let Some((ctor, _)) = self.ctor_named(&var.name) {
      self.resolutions.insert(&var.name.span, Resolved::Ctor(ctor));

      let kind = if self.ctors[ctor.0 as usize].arity == 0 {
        CExprKind::Ctor { ctor, args: vec![] }
      } else {
        CExprKind::CtorFn { ctor }
      };

      return CExpr::new(kind, origin);
    }

    if self.effect_as_value(&var.name, &var.span) {
      return placeholder(origin);
    }

    if self.imports.contains_key(name) {
      self.error(
        BuiltinModuleAsValue,
        format!("`{name}` is a module, not a value"),
        var.span.clone(),
        Some(format!("use one of its functions, like `{name}.<function>(…)`")),
      );
    } else if builtins::is_module(name) {
      self.error(
        BuiltinModuleAsValue,
        format!("`{name}` is a builtin module, not a value"),
        var.span.clone(),
        Some(format!("use one of its members, like `{name}.<member>(…)`")),
      );
    } else {
      self.unknown_uppercase(&var.name);
    }

    placeholder(origin)
  }

  fn method(
    &mut self,
    span: &Span,
    trait_path: String,
    method: &str,
    origin: Option<Span>,
  ) -> CExpr {
    let method = method.to_string();

    self.resolutions.insert(
      span,
      Resolved::Method {
        trait_path: trait_path.clone(),
        method: method.clone(),
      },
    );

    CExpr::new(CExprKind::Method { trait_path, method }, origin)
  }

  fn field(&mut self, field: &FieldAccess) -> CExpr {
    let origin = Some(field.span.clone());

    if let Some(effect) = self.effect_target(field) {
      return self.operation(&effect, field);
    }

    if let Some((module, interface)) = self.std_import(&field.target) {
      let member = &field.field.text;

      if let Some(sig) = interface.method_owner(member) {
        let trait_path = format!("{}.{}", module.path(), sig.name);

        return self.method(&field.field.span, trait_path, member, origin);
      }

      if interface.function(member).is_some() || interface.constant(member) {
        let kind = module.member(member);
        let resolved =
          Resolved::Imported { module: module.path(), member: member.clone() };

        self.resolutions.insert(&field.field.span, resolved);
        return CExpr::new(kind, origin);
      }

      let module = module.name();

      let available: Vec<String> = interface
        .functions
        .iter()
        .map(|(f, _)| f)
        .filter(|name| !name.starts_with('$'))
        .chain(&interface.constants)
        .chain(interface.traits.iter().flat_map(|t| &t.methods).map(|(m, _)| m))
        .map(|name| format!("`{name}`"))
        .collect();
      let message = format!("`{module}` exports no function `{member}`");

      self.error(
        UnknownBuiltinMember,
        message,
        field.field.span.clone(),
        Some(format!("available: {}", available.join(", "))),
      );

      return placeholder(origin);
    }

    let Some(module) = builtin_module(&field.target) else {
      let target = self.expr(&field.target);

      return CExpr::new(
        CExprKind::Field {
          target: Box::new(target),
          name: field.field.text.clone(),
        },
        origin,
      );
    };

    let member = &field.field.text;

    if let Some(owner) = builtins::std_owner(module)
      && self.module_name.as_deref() != Some(owner)
    {
      self.error(
        UnknownBuiltinMember,
        format!("`{module}` is internal to `Std.{owner}`"),
        field.target.span().clone(),
        Some(format!("use the functions of `Std.{owner}` instead")),
      );

      return placeholder(origin);
    }

    if let Some(builtin) = builtins::lookup(module, member) {
      self.resolutions.insert(
        &field.field.span,
        Resolved::Builtin { module: builtin.module, member: builtin.member },
      );

      return CExpr::new(
        CExprKind::Builtin { module: builtin.module, member: builtin.member },
        origin,
      );
    }

    let available: Vec<String> =
      builtins::members(module).map(|m| format!("`{m}`")).collect();
    let help = if available.is_empty() {
      format!("`{module}` has no members yet")
    } else {
      format!("available members: {}", available.join(", "))
    };

    self.error(
      UnknownBuiltinMember,
      format!("`{module}` has no member `{member}`"),
      field.field.span.clone(),
      Some(help),
    );

    placeholder(origin)
  }

  fn call(&mut self, call: &Call) -> CExpr {
    let args = call.args.iter().map(|arg| self.expr(arg)).collect();

    self.apply(&call.callee, args, call.span.clone())
  }

  fn apply(&mut self, callee: &Expr, args: Vec<CExpr>, origin: Span) -> CExpr {
    if let Expr::Var(var) = callee {
      let name = &var.name.text;

      if let Some((ctor, declared)) = self.ctor_named(&var.name) {
        let arity = self.ctors[ctor.0 as usize].arity;

        self.resolutions.insert(&var.name.span, Resolved::Ctor(ctor));

        if args.len() != arity {
          self.arity_error(
            ConstructorArity,
            name,
            arity,
            args.len(),
            &origin,
            declared,
          );
        }

        return CExpr::new(CExprKind::Ctor { ctor, args }, Some(origin));
      }

      if self.scopes.lookup(name).is_none() {
        if let Some(top) = self.consts.get(name) {
          let declared = top.span.clone();

          self.constant_called(name, &origin, Some(declared));
        }

        if let Some(top) = self.fns.get(name) {
          let (arity, declared) = (top.arity, top.span.clone());

          if args.len() != arity {
            self.arity_error(
              CallArity,
              name,
              arity,
              args.len(),
              &origin,
              declared,
            );
          }
        } else if let Some((arity, declared)) = self.extern_arity(name)
          && args.len() != arity
        {
          self.arity_error(
            CallArity,
            name,
            arity,
            args.len(),
            &origin,
            declared,
          );
        }
      }
    }

    if let Expr::Field(field) = callee {
      self.operation_arity(field, args.len(), &origin);

      let constant = self.std_import(&field.target).and_then(|(module, i)| {
        i.constant(&field.field.text)
          .then(|| format!("{}.{}", module.name(), field.field.text))
      });

      if let Some(name) = constant {
        self.constant_called(&name, &origin, None);
      }

      let std_arity =
        self.std_import(&field.target).and_then(|(module, interface)| {
          let arity = interface.function(&field.field.text)?;

          Some((format!("{}.{}", module.name(), field.field.text), arity))
        });

      if let Some((name, arity)) = std_arity {
        if args.len() != arity {
          self.error(
            CallArity,
            arity_message(&name, arity, args.len()),
            origin.clone(),
            None,
          );
        }
      }

      if let Some(module) = builtin_module(&field.target) {
        if let Some(builtin) = builtins::lookup(module, &field.field.text) {
          if args.len() != builtin.arity {
            let name = format!("{module}.{}", builtin.member);

            self.error(
              CallArity,
              arity_message(&name, builtin.arity, args.len()),
              origin.clone(),
              None,
            );
          }
        }
      }
    }

    let func = self.expr(callee);

    CExpr::new(CExprKind::App { func: Box::new(func), args }, Some(origin))
  }

  fn binary(&mut self, binary: &Binary) -> CExprKind {
    let op = match binary.op {
      BinaryOp::Or => PrimOp::Or,
      BinaryOp::And => PrimOp::And,
      BinaryOp::Eq => PrimOp::Eq,
      BinaryOp::NotEq => PrimOp::Ne,
      BinaryOp::Lt => PrimOp::Lt,
      BinaryOp::LtEq => PrimOp::Le,
      BinaryOp::Gt => PrimOp::Gt,
      BinaryOp::GtEq => PrimOp::Ge,
      BinaryOp::Add => PrimOp::Add,
      BinaryOp::Sub => PrimOp::Sub,
      BinaryOp::Mul => PrimOp::Mul,
      BinaryOp::Div => PrimOp::Div,
      BinaryOp::Rem => PrimOp::Mod,
      BinaryOp::BitAnd => PrimOp::BitAnd,
      BinaryOp::BitOr => PrimOp::BitOr,
      BinaryOp::BitXor => PrimOp::BitXor,
    };

    let args = vec![self.expr(&binary.left), self.expr(&binary.right)];

    CExprKind::Prim { op, args }
  }

  fn unary(&mut self, unary: &Unary) -> CExprKind {
    let op = match unary.op {
      UnaryOp::Negate => PrimOp::Neg,
      UnaryOp::Not => PrimOp::Not,
      UnaryOp::BitNot => PrimOp::BitNot,
    };

    CExprKind::Prim { op, args: vec![self.expr(&unary.operand)] }
  }

  fn lambda(&mut self, lambda: &Lambda) -> CExprKind {
    self.scopes.push();

    let params = self.params(&lambda.params);
    let body = self.block(&lambda.body);

    self.scopes.pop();

    CExprKind::Lam { params, body: Box::new(body) }
  }

  fn block(&mut self, block: &Block) -> CExpr {
    self.scopes.push();

    let mut steps = Vec::with_capacity(block.stmts.len());

    for stmt in &block.stmts {
      let step = match stmt {
        Stmt::Let(stmt) => {
          let value = self.expr(&stmt.value);
          let span = stmt.span.clone();

          match &stmt.pattern {
            Pattern::Var(var) => {
              let sym = self.bind(&var.name);

              self
                .resolutions
                .insert(&var.name.span, Resolved::Local(sym.clone()));
              Step::Bind(Some(sym), value, span)
            }
            Pattern::Wildcard(_) => Step::Bind(None, value, span),
            pattern => {
              Step::Match(value, self.pattern(pattern, &mut Vec::new()), span)
            }
          }
        }
        Stmt::Expr(stmt) => {
          Step::Bind(None, self.expr(&stmt.expr), stmt.span.clone())
        }
      };

      steps.push(step);
    }

    let mut acc = self.expr(&block.result);

    for step in steps.into_iter().rev() {
      acc = match step {
        Step::Bind(sym, value, span) => CExpr::new(
          CExprKind::Let { sym, value: Box::new(value), body: Box::new(acc) },
          Some(span),
        ),
        Step::Match(value, pattern, span) => CExpr::new(
          CExprKind::Case {
            scrutinee: Box::new(value),
            arms: vec![CArm { pattern, body: acc, origin: Some(span.clone()) }],
          },
          Some(span),
        ),
      };
    }

    self.scopes.pop();

    acc
  }

  fn if_expr(&mut self, node: &If) -> CExpr {
    let cond = self.expr(&node.cond);
    let then_branch = self.block(&node.then_branch);
    let else_branch = match &*node.else_branch {
      Else::Block(block) => self.block(block),
      Else::If(inner) => self.if_expr(inner),
    };

    CExpr::new(
      CExprKind::If {
        cond: Box::new(cond),
        then_branch: Box::new(then_branch),
        else_branch: Box::new(else_branch),
      },
      Some(node.span.clone()),
    )
  }

  fn match_expr(&mut self, node: &Match) -> CExprKind {
    let scrutinee = self.expr(&node.scrutinee);
    let arms = self.arms(&node.arms);

    CExprKind::Case { scrutinee: Box::new(scrutinee), arms }
  }

  fn try_expr(&mut self, node: &Try) -> CExprKind {
    let body = self.block(&node.body);
    let caught = self.syms.fresh("$v");
    let arms = self.arms(&node.arms);
    let handler = CExpr::new(
      CExprKind::Case {
        scrutinee: Box::new(CExpr::new(
          CExprKind::Var(caught.clone()),
          Some(node.catch_span.clone()),
        )),
        arms,
      },
      Some(node.catch_span.clone()),
    );

    CExprKind::Try {
      body: Box::new(body),
      caught,
      handler: Box::new(handler),
      handles: Vec::new(),
    }
  }

  fn arms(&mut self, arms: &[MatchArm]) -> Vec<CArm> {
    arms
      .iter()
      .map(|arm| {
        self.scopes.push();

        let pattern = self.pattern(&arm.pattern, &mut Vec::new());
        let body = self.expr(&arm.body);

        self.scopes.pop();

        CArm { pattern, body, origin: Some(arm.span.clone()) }
      })
      .collect()
  }

  fn bind_uses_function(&mut self, name: &Name) {
    self.error(
      ConstantUsesBelow,
      format!("a binding cannot use the function `{}`", name.text),
      name.span.clone(),
      Some(
        "a binding can use externs, other effects, constants and std, but \
         not the `functions` zone below it"
          .to_string(),
      ),
    );
  }

  fn pattern(&mut self, pattern: &Pattern, seen: &mut Vec<Name>) -> CPattern {
    match pattern {
      Pattern::Wildcard(_) => CPattern::Wildcard,
      Pattern::Var(var) => {
        if let Some(first) = seen.iter().find(|n| n.text == var.name.text) {
          let first = first.span.clone();

          self.duplicate(&var.name, "binding", first);
        }

        seen.push(var.name.clone());

        let sym = self.bind(&var.name);

        self.resolutions.insert(&var.name.span, Resolved::Local(sym.clone()));
        CPattern::Bind(sym)
      }
      Pattern::Lit(lit) => {
        let value = match &lit.lit {
          PatLit::Int(int) => match self.int(int).kind {
            CExprKind::Lit(lit) => lit,
            _ => Lit::Number(0.0),
          },
          PatLit::Float(float) => {
            Lit::Number(self.float_value(&float.raw, &float.span))
          }
          PatLit::Bool(b) => Lit::Bool(b.value),
          PatLit::String(s) => Lit::String(
            s.parts
              .iter()
              .map(|part| match part {
                StringPart::Text(text) => text.value.as_str(),
                StringPart::Interp(_) => "",
              })
              .collect(),
          ),
        };

        CPattern::Lit(match value {
          Lit::Number(n) if lit.negative && n != 0.0 => Lit::Number(-n),
          other => other,
        })
      }
      Pattern::Ctor(ctor) => {
        let args: Vec<CPattern> =
          ctor.args.iter().map(|arg| self.pattern(arg, seen)).collect();

        let Some((id, declared)) = self.ctor_named(&ctor.name) else {
          self.unknown_uppercase(&ctor.name);
          return CPattern::Wildcard;
        };
        let arity = self.ctors[id.0 as usize].arity;

        self.resolutions.insert(&ctor.name.span, Resolved::Ctor(id));

        if args.len() != arity {
          self.arity_error(
            ConstructorArity,
            &ctor.name.text,
            arity,
            args.len(),
            &ctor.span,
            declared,
          );
        }

        CPattern::Ctor { ctor: id, args }
      }
      Pattern::Record(record) => CPattern::Record {
        fields: record
          .fields
          .iter()
          .map(|field| {
            (field.name.text.clone(), self.pattern(&field.pattern, seen))
          })
          .collect(),
      },
      Pattern::List(list) => {
        let items: Vec<CPattern> =
          list.items.iter().map(|item| self.pattern(item, seen)).collect();
        let tail = list.tail.as_ref().map(|t| self.pattern(t, seen));

        let (nil, cons) = if self
          .std_lists
          .contains(&(list.span.start, list.span.end))
        {
          let (nil, cons) = self.std_list_ctors();

          self.resolutions.insert(&list.span, Resolved::StdList { nil, cons });
          (nil, cons)
        } else {
          let Some((nil, cons)) = self.list_ctors(&list.span) else {
            return CPattern::Wildcard;
          };

          self.resolutions.insert(&list.span, Resolved::List { nil, cons });
          (nil, cons)
        };

        let end = CPattern::Ctor { ctor: nil, args: vec![] };

        items.into_iter().rev().fold(tail.unwrap_or(end), |acc, item| {
          CPattern::Ctor { ctor: cons, args: vec![item, acc] }
        })
      }
      Pattern::Invalid(invalid) => {
        ice("an `InvalidPattern` reached lowering", Some(&invalid.span))
      }
    }
  }

  pub(super) fn error(
    &mut self,
    code: DiagnosticCode,
    message: impl Into<String>,
    span: Span,
    help: Option<String>,
  ) {
    let mut diagnostic = Diagnostic::error(code, message, Label::new(span));

    if let Some(help) = help {
      diagnostic = diagnostic.with_help(help);
    }

    self.diagnostics.push(diagnostic);
  }

  fn unknown_name(&mut self, name: &Name, what: &str, place: &str) {
    let candidates: Vec<&str> =
      self.scopes.names().chain(self.top_names()).collect();
    let help =
      suggest(&name.text, candidates).map(|s| format!("did you mean `{s}`?"));
    let what = if what.is_empty() { String::new() } else { format!("{what} ") };

    self.error(
      UnknownName,
      format!("cannot find {what}`{}` {place}", name.text),
      name.span.clone(),
      help,
    );
  }

  fn unknown_uppercase(&mut self, name: &Name) {
    let candidates = self.ctor_ids.keys().map(String::as_str);
    let help =
      suggest(&name.text, candidates).map(|s| format!("did you mean `{s}`?"));

    self.error(
      UnknownUppercaseName,
      format!("cannot find constructor `{}`", name.text),
      name.span.clone(),
      help,
    );
  }

  pub(super) fn duplicate(&mut self, name: &Name, what: &str, first: Span) {
    self.diagnostics.push(
      Diagnostic::error(
        DuplicateDefinition,
        format!("the {what} `{}` is defined more than once", name.text),
        Label::new(name.span.clone()),
      )
      .with_secondary(Label::new(first).with_message("first defined here")),
    );
  }

  fn constant_called(&mut self, name: &str, at: &Span, declared: Option<Span>) {
    let mut diagnostic = Diagnostic::error(
      ConstantCalled,
      format!("`{name}` is a constant, not a function"),
      Label::new(at.clone()),
    )
    .with_help(format!("use it without parentheses: `{name}`"));

    if let Some(declared) = declared {
      diagnostic = diagnostic
        .with_secondary(Label::new(declared).with_message("defined here"));
    }

    self.diagnostics.push(diagnostic);
  }

  fn constant_uses_below(&mut self, name: &Name, what: &str) {
    self.error(
      ConstantUsesBelow,
      format!("a constant cannot use {what} `{}`", name.text),
      name.span.clone(),
      Some(
        "a constant can only use what is declared above it: imports, \
         constructors and earlier constants"
          .to_string(),
      ),
    );
  }

  fn arity_error(
    &mut self,
    code: DiagnosticCode,
    name: &str,
    expected: usize,
    found: usize,
    at: &Span,
    declared: Span,
  ) {
    self.diagnostics.push(
      Diagnostic::error(
        code,
        arity_message(name, expected, found),
        Label::new(at.clone()),
      )
      .with_secondary(Label::new(declared).with_message("defined here")),
    );
  }
}

fn builtin_module(expr: &Expr) -> Option<&'static str> {
  match expr {
    Expr::Var(var) => builtins::BUILTIN_MODULES
      .iter()
      .find(|module| **module == var.name.text)
      .copied(),
    _ => None,
  }
}

pub(super) fn arity_message(
  name: &str,
  expected: usize,
  found: usize,
) -> String {
  let args = if expected == 1 { "argument" } else { "arguments" };
  let were = if found == 1 { "was" } else { "were" };

  format!("`{name}` takes {expected} {args} but {found} {were} given")
}

fn type_text(ty: &TypeExpr, param: &str, target: &str) -> String {
  let list = |items: &[TypeExpr]| {
    items
      .iter()
      .map(|t| type_text(t, param, target))
      .collect::<Vec<_>>()
      .join(", ")
  };

  match ty {
    TypeExpr::Ref(r) if r.args.is_empty() => r.name.text.clone(),
    TypeExpr::Ref(r) => format!("{}<{}>", r.name.text, list(&r.args)),
    TypeExpr::Var(v) if v.name.text == param => target.to_string(),
    TypeExpr::Var(v) => v.name.text.clone(),
    TypeExpr::Fn(f) => {
      format!(
        "function({}) -> {}",
        list(&f.params),
        type_text(&f.ret, param, target)
      )
    }
    TypeExpr::Record(r) => {
      let mut fields: Vec<String> = r
        .fields
        .iter()
        .map(|f| {
          format!("{}: {}", f.name.text, type_text(&f.ty, param, target))
        })
        .collect();

      if let Some(tail) = &r.tail {
        fields.push(format!("| {}", tail.text));
      }

      if fields.is_empty() {
        "{}".to_string()
      } else {
        format!("{{ {} }}", fields.join(", "))
      }
    }
    TypeExpr::Invalid(_) => "…".to_string(),
  }
}

fn imports_prelude(decls: &[&Decl]) -> bool {
  decls.iter().any(|decl| {
    matches!(decl, Decl::Import(import)
      if import.path.len() == 2
        && import.path[0].text == "Std"
        && import.path[1].text == "Prelude")
  })
}

fn is_prelude(module: &Module) -> bool {
  module.name.as_ref().is_some_and(|name| name.text == "Prelude")
}

fn is_upper(name: &str) -> bool {
  name.chars().next().is_some_and(char::is_uppercase)
}

fn placeholder(origin: Option<Span>) -> CExpr {
  CExpr::new(CExprKind::Lit(Lit::Number(0.0)), origin)
}
