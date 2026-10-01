use std::{
  collections::{BTreeMap, BTreeSet},
  path::{Component, Path, PathBuf},
};

use crate::{
  CompileOptions,
  check::{
    Types,
    hosts::{label_hosts, runs_on, show},
  },
  core::{
    dictionaries::walk,
    effects::EffectTable,
    ir::{Boundary, CDecl, CExpr, CExprKind, CModule, DeclKind},
  },
  shared::codes::DiagnosticCode::{HostCantRunMain, MissingBinding},
  shared::diagnostic::{Diagnostic, DiagnosticBag, Label},
  shared::source::Span,
  types::{
    print::bare,
    ty::{Label as EffectLabel, Type},
  },
};

/// Picks the host a build is for, from the `--host` flag and the hosts the
/// program declares.
///
/// # Errors
///
/// Fails when the flag names an undeclared host, or when it is missing and the
/// program declares more than one host.
pub fn choose_host(
  flag: Option<&str>,
  declared: &[String],
) -> Result<Option<String>, String> {
  let listed = || {
    let names: Vec<String> =
      declared.iter().map(|h| format!("`{h}`")).collect();

    match names.as_slice() {
      [] => "no hosts".to_string(),
      [one] => format!("host {one}"),
      [init @ .., last] => format!("hosts {} and {last}", init.join(", ")),
    }
  };

  match (flag, declared) {
    (Some(host), _) if declared.iter().any(|h| h == host) => {
      Ok(Some(host.to_string()))
    }
    (Some(host), _) => {
      let quoted: Vec<String> =
        declared.iter().map(|h| format!("`{h}`")).collect();
      let known = if quoted.is_empty() {
        "no hosts".to_string()
      } else {
        quoted.join(", ")
      };

      Err(format!("unknown host `{host}`; this program declares {known}"))
    }
    (None, []) => Ok(None),
    (None, [only]) => Ok(Some(only.clone())),
    (None, _) => Err(format!(
      "this program declares {}; choose one with `--host`",
      listed()
    )),
  }
}

pub(crate) fn for_host(
  mut module: CModule,
  types: &Types,
  effects: &EffectTable,
  host: Option<&str>,
  options: &CompileOptions,
  bag: &mut DiagnosticBag,
) -> (CModule, Vec<(String, String)>) {
  let Some(host) = host else {
    module.binds.clear();
    module.bridges.clear();

    let files = prepare_externs(&mut module, types, options);

    return (module, files);
  };
  let hosts: BTreeMap<&str, &crate::check::hosts::HostSet> =
    types.hosts.iter().map(|(n, h)| (n.as_str(), h)).collect();
  let runnable = |name: &str| hosts.get(name).is_none_or(|h| runs_on(h, host));

  if let Some(main) = module.decls.iter().find(|d| &*d.sym.name == "main")
    && !runnable("main")
  {
    bag.push(cant_run_main(types, effects, host, main.origin.clone()));
  }

  module.decls.retain(|decl| {
    decl.kind == DeclKind::Constant
      || decl.sym.name.starts_with('$')
      || runnable(&decl.sym.name)
  });
  module.binds.retain(|bind| bind.host == host);
  module.bridges.retain(|b| b.host == host || b.via == host);
  module.host = Some(host.to_string());
  module.std_imports.retain(|name| {
    crate::stdlib::interface(name)
      .is_none_or(|i| i.hosts.is_empty() || i.hosts.iter().any(|h| h == host))
  });
  drop_unused_constants(&mut module);

  let mut used: BTreeSet<String> = BTreeSet::new();

  for bridge in module.bridges.iter().filter(|b| b.via == host) {
    used.insert(bridge.effect.clone());
  }

  for body in bodies(&module) {
    walk(body, &mut |e| {
      if let CExprKind::Op { effect, .. } = &e.kind {
        used.insert(effect.clone());
      }
    });
  }

  for effect in &used {
    if module.binds.iter().any(|b| b.effect == *effect) {
      module.bind_refs.push((effect.clone(), None));
      continue;
    }

    let imported = effects
      .binds
      .iter()
      .find(|b| b.effect == *effect && b.host == host && b.module.is_some())
      .and_then(|b| b.module.clone());

    if let Some(path) = imported {
      if !module.user_imports.iter().any(|(p, _)| *p == path)
        && let Some(source) = options.modules.iter().find(|m| m.path == path)
      {
        module.user_imports.push((path.clone(), source.specifier.clone()));
      }

      module.bind_refs.push((effect.clone(), Some(path)));
      continue;
    }

    let native = effects
      .effects
      .get(effect)
      .is_some_and(|e| e.host.as_deref() == Some(host));

    if native {
      bag.push(missing_binding(types, &module, effect, host));
    }
  }

  let files = prepare_externs(&mut module, types, options);

  (module, files)
}

fn drop_unused_constants(module: &mut CModule) {
  let constant = |d: &CDecl| d.kind == DeclKind::Constant && !d.exported;
  let mut live: BTreeSet<u32> = BTreeSet::new();
  let mut pending: Vec<&CExpr> = module
    .decls
    .iter()
    .filter(|d| !constant(d))
    .map(|d| &d.body)
    .chain(module.binds.iter().flat_map(|b| b.ops.iter().map(|op| &op.body)))
    .chain(module.impls.iter().flat_map(|i| i.methods.iter().map(|m| &m.body)))
    .collect();

  while let Some(body) = pending.pop() {
    let mut found: Vec<u32> = Vec::new();

    walk(body, &mut |e| {
      if let CExprKind::Var(sym) = &e.kind {
        found.push(sym.id);
      }
    });

    for id in found {
      if live.insert(id)
        && let Some(decl) =
          module.decls.iter().find(|d| d.sym.id == id && constant(d))
      {
        pending.push(&decl.body);
      }
    }
  }

  module.decls.retain(|d| !constant(d) || live.contains(&d.sym.id));
}

fn bodies(module: &CModule) -> impl Iterator<Item = &CExpr> {
  module
    .decls
    .iter()
    .map(|d| &d.body)
    .chain(module.binds.iter().flat_map(|b| b.ops.iter().map(|op| &op.body)))
}

fn prepare_externs(
  module: &mut CModule,
  types: &Types,
  options: &CompileOptions,
) -> Vec<(String, String)> {
  let mut externs: BTreeSet<String> = BTreeSet::new();

  for body in bodies(module) {
    walk(body, &mut |e| {
      if let CExprKind::Extern { name } = &e.kind {
        externs.insert(name.clone());
      }
    });
  }

  module.externs.retain(|e| externs.contains(&e.name));

  let mut files: Vec<(String, String)> = Vec::new();

  for ext in &mut module.externs {
    let (rewritten, copy) = specifier(&ext.module, options);

    ext.module = rewritten;

    if let Some(Type::Fn { params, ret, .. }) =
      types.extern_types.get(&ext.name)
    {
      ext.params = params.iter().map(boundary).collect();
      ext.ret = boundary(ret);
    }

    if let Some(copy) = copy
      && !files.contains(&copy)
    {
      files.push(copy);
    }
  }

  files
}

#[must_use]
pub fn boundary(ty: &Type) -> Boundary {
  match ty {
    Type::Record(row) if row.fields.is_empty() && row.tail.is_closed() => {
      Boundary::Unit
    }
    Type::Record(row) => {
      let fields: Vec<(String, Boundary)> = row
        .fields
        .iter()
        .map(|(name, ty)| (name.to_string(), boundary(ty)))
        .filter(|(_, b)| *b != Boundary::Plain)
        .collect();

      if fields.is_empty() { Boundary::Plain } else { Boundary::Record(fields) }
    }
    Type::Con { name, args } if args.len() == 1 => match bare(name) {
      "Option" => Boundary::Option(Box::new(boundary(&args[0]))),
      "List" => Boundary::List(Box::new(boundary(&args[0]))),
      _ => Boundary::Plain,
    },
    Type::Con { name, args } if args.len() == 2 && bare(name) == "Result" => {
      Boundary::Result(
        Box::new(boundary(&args[1])),
        Box::new(boundary(&args[0])),
      )
    }
    _ => Boundary::Plain,
  }
}

fn first_use(types: &Types, name: &str, label: &EffectLabel) -> Option<Span> {
  types
    .first_uses
    .get(name)?
    .iter()
    .find(|(l, _)| l == label)
    .map(|(_, s)| s.clone())
}

fn cant_run_main(
  types: &Types,
  effects: &EffectTable,
  host: &str,
  origin: Option<Span>,
) -> Diagnostic {
  let row = match types.scheme("main").map(|s| &s.ty) {
    Some(Type::Fn { effects: row, .. }) => row.labels.clone(),
    _ => Vec::new(),
  };
  let blocking = row
    .iter()
    .find(|label| !runs_on(&label_hosts(effects, label), host))
    .cloned();
  let span = origin.unwrap_or_else(|| Span::empty(std::sync::Arc::from(""), 0));
  let Some(label) = blocking else {
    return Diagnostic::error(
      HostCantRunMain,
      format!("`main` can't run on `{host}`"),
      Label::new(span),
    );
  };
  let hosts = show(&label_hosts(effects, &label));
  let mut diagnostic = Diagnostic::error(
    HostCantRunMain,
    format!(
      "`main` can't run on `{host}`: it uses `{}`, which runs on {hosts}",
      label.name
    ),
    Label::new(span),
  )
  .with_help(format!(
    "build for a host `main` can run on, or move the `{}` work elsewhere",
    label.name
  ));

  if let Some(at) = first_use(types, "main", &label) {
    diagnostic = diagnostic.with_secondary(
      Label::new(at).with_message(format!("`{}` is used here", label.name)),
    );
  }

  diagnostic
}

fn missing_binding(
  types: &Types,
  module: &CModule,
  effect: &str,
  host: &str,
) -> Diagnostic {
  let label = EffectLabel::effect(effect);
  let at = module
    .decls
    .iter()
    .find_map(|d| first_use(types, &d.sym.name, &label))
    .or_else(|| module.decls.iter().find_map(|d| d.origin.clone()))
    .unwrap_or_else(|| Span::empty(std::sync::Arc::from(""), 0));

  Diagnostic::error(
    MissingBinding,
    format!("`{effect}` has no binding for `{host}`"),
    Label::new(at).with_message(format!("`{effect}` is used here")),
  )
  .with_help(format!("add `{effect} in {host} {{ … }}` to a `binds` zone"))
}

fn specifier(
  spec: &str,
  options: &CompileOptions,
) -> (String, Option<(String, String)>) {
  if !spec.starts_with("./") && !spec.starts_with("../") {
    return (spec.to_string(), None);
  }

  let Some(source_dir) = &options.source_dir else {
    return (spec.to_string(), None);
  };
  let target = normalize(&Path::new(source_dir).join(spec));

  let Some(output_dir) = &options.output_dir else {
    return (file_url(&target), None);
  };

  if let Some(root) = &options.source_root
    && let Ok(inside) = target.strip_prefix(normalize(Path::new(root)))
  {
    let inside = inside.to_string_lossy().replace('\\', "/");

    return (
      spec.to_string(),
      Some((target.to_string_lossy().into_owned(), inside)),
    );
  }

  let relative = relative(&normalize(Path::new(output_dir)), &target);
  let text = relative.to_string_lossy().replace('\\', "/");
  let text = if text.starts_with("../") { text } else { format!("./{text}") };

  (text, None)
}

fn normalize(path: &Path) -> PathBuf {
  let mut out = PathBuf::new();

  for component in path.components() {
    match component {
      Component::CurDir => {}
      Component::ParentDir => {
        if !out.pop() {
          out.push("..");
        }
      }
      other => out.push(other.as_os_str()),
    }
  }

  out
}

fn relative(from_dir: &Path, to: &Path) -> PathBuf {
  let from: Vec<Component<'_>> = from_dir.components().collect();
  let to_parts: Vec<Component<'_>> = to.components().collect();
  let shared = from.iter().zip(&to_parts).take_while(|(a, b)| a == b).count();
  let mut out = PathBuf::new();

  for _ in shared..from.len() {
    out.push("..");
  }

  for part in &to_parts[shared..] {
    out.push(part.as_os_str());
  }

  out
}

fn file_url(path: &Path) -> String {
  let text = path.to_string_lossy().replace('\\', "/");
  let mut out = String::from("file://");

  if !text.starts_with('/') {
    out.push('/');
  }

  for c in text.chars() {
    match c {
      ' ' => out.push_str("%20"),
      '#' => out.push_str("%23"),
      '?' => out.push_str("%3F"),
      '%' => out.push_str("%25"),
      other => out.push(other),
    }
  }

  out
}
