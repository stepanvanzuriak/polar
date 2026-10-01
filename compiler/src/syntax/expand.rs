use std::sync::Arc;

use polar_plugin as api;

use crate::{
  shared::codes::DiagnosticCode::{PluginError, PluginFailed},
  shared::diagnostic::{Diagnostic, DiagnosticBag, Label},
  shared::modules::ModuleSource,
  shared::source::{SourceFile, Span},
  syntax::ast::{
    Builtin, Decl, Module, TypeExpr, Zone, ZoneKind,
    fields::{AsNode, NodeRef, children},
  },
  syntax::builder::{Expansion, Gen},
  syntax::derive,
  syntax::lexer::lex,
  syntax::params,
  syntax::parser::parse,
  syntax::plugins,
};

pub fn expand(
  module: &mut Module,
  file: &SourceFile,
  modules: &[ModuleSource],
  bag: &mut DiagnosticBag,
) -> Expansion {
  params::desugar(module);

  let mut builder = Gen::new(file);
  let emitted = expand_plugins(module, file, &mut builder, bag);

  for kind in Builtin::ALL {
    let decls: Vec<Decl> = emitted
      .iter()
      .filter(|(k, _)| *k == kind)
      .map(|(_, d)| d.clone())
      .collect();

    if !decls.is_empty() {
      insert_decls(module, kind, decls, &builder.file.clone());
    }
  }

  derive::expand(module, modules, &mut builder);
  builder.finish()
}

fn expand_plugins(
  module: &Module,
  file: &SourceFile,
  builder: &mut Gen,
  bag: &mut DiagnosticBag,
) -> Vec<(Builtin, Decl)> {
  let zones: Vec<&Zone> = module
    .zones
    .iter()
    .filter(|z| matches!(z.kind, ZoneKind::Plugin(_)))
    .collect();

  if zones.is_empty() {
    return Vec::new();
  }

  let lexed = lex(file, &mut DiagnosticBag::default());
  let summary = summarize(module);
  let mut emitted = Vec::new();
  let mut named: Vec<String> = Vec::new();

  for zone in zones {
    let ZoneKind::Plugin(id) = zone.kind else { continue };
    let keyword = plugins::keyword(id);
    let at = Span::new(
      builder.file.clone(),
      zone.span.start,
      zone.span.start + keyword.len(),
    );
    let entries = plugins::entries(file, &lexed, zone);
    let zone_span = api::Span { start: at.start, end: at.end };

    let expansion =
      match plugins::expand(id, zone_span, entries, summary.clone()) {
        Ok(expansion) => expansion,
        Err(message) => {
          bag.push(Diagnostic::error(
            PluginFailed,
            format!("the `{keyword}` plugin failed: {message}"),
            Label::new(at),
          ));
          continue;
        }
      };

    for diagnostic in &expansion.diagnostics {
      bag.push(convert(diagnostic, &builder.file));
    }

    if !expansion.diagnostics.is_empty() {
      continue;
    }

    for sibling in &expansion.siblings {
      if let Some(problem) = sibling_problem(module, &sibling.name, &named) {
        bag.push(Diagnostic::error(
          PluginFailed,
          format!(
            "the `{keyword}` plugin generated the module `{}`, but {problem}",
            sibling.name
          ),
          Label::new(span_of(&builder.file, sibling.origin)),
        ));
      }

      named.push(sibling.name.clone());
    }

    builder.needs_std("Option");
    builder.needs_std("List");

    for generated in &expansion.generated {
      let origin = span_of(&builder.file, generated.origin);
      let target =
        Builtin::ALL.into_iter().find(|b| b.as_str() == generated.zone);

      let Some(target) =
        target.filter(|t| plugins::generates_below(zone.kind, *t))
      else {
        bag.push(
          Diagnostic::error(
            PluginFailed,
            format!(
              "the `{keyword}` plugin generated into `{}`, but a plugin can \
               only generate into a built-in zone below `{keyword}`",
              generated.zone
            ),
            Label::new(origin),
          )
          .with_note(format!(
            "the zone order puts `{keyword}` after `{}`",
            plugins::after(id).as_str()
          )),
        );
        continue;
      };

      match parse_generated(builder, generated, target, &origin) {
        Ok(decls) => emitted.extend(decls.into_iter().map(|d| (target, d))),
        Err(problem) => bag.push(
          Diagnostic::error(
            PluginFailed,
            format!(
              "the `{keyword}` plugin generated code that doesn't parse: {problem}"
            ),
            Label::new(origin),
          )
          .with_note(format!("it generated:\n{}", generated.source)),
        ),
      }
    }
  }

  emitted
}

/// Why a plugin can't generate a sibling module named `name`, if it can't.
fn sibling_problem(
  module: &Module,
  name: &str,
  named: &[String],
) -> Option<&'static str> {
  let mut chars = name.chars();
  let pascal = chars.next().is_some_and(|c| c.is_ascii_uppercase())
    && chars.all(|c| c.is_ascii_alphanumeric());
  let own = module.name.as_ref().map(|n| n.text.as_str());

  if !pascal {
    Some("a module name is one PascalCase word, like `Paths`")
  } else if own.is_some_and(|own| own.rsplit('.').next() == Some(name)) {
    Some("that is the name of the module holding the zone")
  } else if named.iter().any(|n| n == name) {
    Some("another zone in this module generates it too")
  } else {
    None
  }
}

/// The sibling modules the plugin zones of `module` generate, as
/// `(name, source)`. Zones that fail or report errors generate none; compiling
/// the module itself reports why.
pub(crate) fn siblings(
  module: &Module,
  file: &SourceFile,
) -> Vec<(String, String)> {
  let mut module = module.clone();

  params::desugar(&mut module);

  let lexed = lex(file, &mut DiagnosticBag::default());
  let summary = summarize(&module);
  let mut out: Vec<(String, String)> = Vec::new();
  let mut named: Vec<String> = Vec::new();

  for zone in &module.zones {
    let ZoneKind::Plugin(id) = zone.kind else { continue };
    let keyword = plugins::keyword(id);
    let at = api::Span {
      start: zone.span.start,
      end: zone.span.start + keyword.len(),
    };
    let entries = plugins::entries(file, &lexed, zone);
    let Ok(expansion) = plugins::expand(id, at, entries, summary.clone())
    else {
      continue;
    };

    if !expansion.diagnostics.is_empty() {
      continue;
    }

    for sibling in expansion.siblings {
      if sibling_problem(&module, &sibling.name, &named).is_none() {
        named.push(sibling.name.clone());
        out.push((
          sibling.name,
          format!(
            "// Generated by the `{keyword}` zone in `{}`. Change that zone, \
             not this module.\n{}",
            file.name(),
            sibling.source
          ),
        ));
      }
    }
  }

  out
}

fn parse_generated(
  builder: &mut Gen,
  generated: &api::Generated,
  target: Builtin,
  origin: &Span,
) -> Result<Vec<Decl>, String> {
  let header = format!("{}\n", target.as_str());
  let length = header.len() + generated.source.len();
  let base = builder.reserve(length);
  let text = format!("{}{header}{}", " ".repeat(base), generated.source);
  let snippet = SourceFile::new(&*builder.file, text.as_str());
  let mut inner = DiagnosticBag::default();
  let module = plugins::within(&[], || {
    let lexed = lex(&snippet, &mut inner);

    parse(&snippet, &lexed, &mut inner)
  });

  if let Some(first) = inner.into_sorted().first() {
    return Err(first.message.clone());
  }

  let start = base + header.len();

  for offset in base..=base + length {
    builder.map(offset, origin);
  }

  for piece in &generated.origins {
    let span = span_of(&builder.file, piece.span);

    for offset in
      start + piece.start..start + piece.end.min(generated.source.len())
    {
      builder.map(offset, &span);
    }
  }

  let mut zones = module.zones.into_iter();
  let decls = match (zones.next(), zones.next()) {
    (Some(zone), None) if zone.kind == ZoneKind::Builtin(target) => zone.decls,
    _ => {
      return Err(format!("expected only `{}` declarations", target.as_str()));
    }
  };

  for decl in &decls {
    mark_std(builder, decl.as_node());
  }

  Ok(decls)
}

fn mark_std(builder: &mut Gen, node: NodeRef<'_>) {
  const STD_CTORS: [&str; 4] = ["Some", "None", "Ok", "Err"];

  match node {
    NodeRef::Var(v) if STD_CTORS.contains(&v.name.text.as_str()) => {
      builder.std_ctor_span(&v.name.span);
    }
    NodeRef::PCtor(c) if STD_CTORS.contains(&c.name.text.as_str()) => {
      builder.std_ctor_span(&c.name.span);
    }
    NodeRef::ListLit(l) => builder.std_list_span(&l.span),
    NodeRef::PList(l) => builder.std_list_span(&l.span),
    _ => {}
  }

  for child in children(node) {
    mark_std(builder, child);
  }
}

fn span_of(file: &Arc<str>, span: api::Span) -> Span {
  Span::new(file.clone(), span.start, span.end)
}

fn convert(d: &api::Diagnostic, file: &Arc<str>) -> Diagnostic {
  let mut label = Label::new(span_of(file, d.span));

  if let Some(message) = &d.label {
    label = label.with_message(message.clone());
  }

  let mut out = Diagnostic::error(PluginError, d.message.clone(), label);

  for secondary in &d.secondary {
    out = out.with_secondary(
      Label::new(span_of(file, secondary.span))
        .with_message(secondary.message.clone()),
    );
  }

  for note in &d.notes {
    out = out.with_note(note.clone());
  }

  if let Some(help) = &d.help {
    out = out.with_help(help.clone());
  }

  out
}

fn summarize(module: &Module) -> api::Module {
  let decls = module.zones.iter().flat_map(|z| &z.decls);
  let span = |s: &Span| api::Span { start: s.start, end: s.end };
  let mut out = api::Module {
    name: module.name.as_ref().map(|n| n.text.clone()),
    ..api::Module::default()
  };

  for decl in decls {
    match decl {
      Decl::Import(import) => out.imports.push(api::Import {
        path: import.path.iter().map(|n| n.text.clone()).collect(),
        alias: import.alias.as_ref().map(|a| a.text.clone()),
        span: span(&import.span),
      }),
      Decl::Fn(f) => out.functions.push(api::Function {
        name: f.name.text.clone(),
        span: span(&f.name.span),
        params: f
          .params
          .iter()
          .map(|p| api::Param {
            name: p.name.text.clone(),
            span: span(&p.span),
            ty: p.ty.as_ref().map(type_text),
          })
          .collect(),
        ret: f.return_type.as_ref().map(type_text),
      }),
      Decl::Type(t) => out.types.push(t.name.text.clone()),
      _ => {}
    }
  }

  out
}

fn type_text(ty: &TypeExpr) -> String {
  let list = |items: &[TypeExpr]| {
    items.iter().map(type_text).collect::<Vec<_>>().join(", ")
  };

  match ty {
    TypeExpr::Ref(r) if r.args.is_empty() => r.name.text.clone(),
    TypeExpr::Ref(r) => format!("{}<{}>", r.name.text, list(&r.args)),
    TypeExpr::Var(v) => v.name.text.clone(),
    TypeExpr::Fn(f) => {
      format!("function({}) -> {}", list(&f.params), type_text(&f.ret))
    }
    TypeExpr::Record(r) => {
      let fields: Vec<String> = r
        .fields
        .iter()
        .map(|f| format!("{}: {}", f.name.text, type_text(&f.ty)))
        .collect();

      format!("{{ {} }}", fields.join(", "))
    }
    TypeExpr::Invalid(_) => String::new(),
  }
}

pub(crate) fn insert_decls(
  module: &mut Module,
  kind: Builtin,
  decls: Vec<Decl>,
  file: &Arc<str>,
) {
  let kind = ZoneKind::Builtin(kind);

  if let Some(zone) = module.zones.iter_mut().find(|z| z.kind == kind) {
    zone.decls.extend(decls);
    return;
  }

  let at = module
    .zones
    .iter()
    .position(|z| z.kind > kind)
    .unwrap_or(module.zones.len());

  module
    .zones
    .insert(at, Zone { span: Span::empty(file.clone(), 0), kind, decls });
}
