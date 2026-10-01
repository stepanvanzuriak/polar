use crate::{
  shared::diagnostic::DiagnosticBag, shared::source::SourceFile,
  syntax::ast::Decl, syntax::ast::Module, syntax::lexer::lex,
  syntax::parser::parse, syntax::plugins,
};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModuleSource {
  pub path: String,
  pub source: String,
  pub specifier: String,
  pub plugins: Vec<String>,
}

fn parse_with(source: &str, filename: &str, sets: &[String]) -> Module {
  plugins::within(sets, || {
    let file = SourceFile::new(filename, source);
    let mut bag = DiagnosticBag::default();

    parse(&file, &lex(&file, &mut bag), &mut bag)
  })
}

#[must_use]
pub fn file(path: &str) -> String {
  let segments: Vec<String> = path.split('.').map(snake_case).collect();

  format!("{}.px", segments.join("/"))
}

fn snake_case(segment: &str) -> String {
  let mut out = String::new();
  let mut prev_lower = false;

  for c in segment.chars() {
    if c.is_uppercase() && prev_lower {
      out.push('_');
    }

    prev_lower = c.is_lowercase() || c.is_ascii_digit();
    out.extend(c.to_lowercase());
  }

  out
}

#[must_use]
pub fn imports(source: &str, filename: &str, sets: &[String]) -> Vec<String> {
  let module = parse_with(source, filename, sets);
  let mut paths = Vec::new();

  for decl in module.zones.iter().flat_map(|zone| &zone.decls) {
    let Decl::Import(import) = decl else { continue };

    if import.path.first().is_none_or(|first| first.text == "Std") {
      continue;
    }

    let path: Vec<&str> =
      import.path.iter().map(|name| name.text.as_str()).collect();
    let path = path.join(".");

    if !paths.contains(&path) {
      paths.push(path);
    }
  }

  paths
}

#[must_use]
pub fn hosts(source: &str, filename: &str, sets: &[String]) -> Vec<String> {
  let module = parse_with(source, filename, sets);

  module
    .zones
    .iter()
    .flat_map(|zone| &zone.decls)
    .filter_map(|decl| match decl {
      Decl::Host(host) => Some(host.name.text.clone()),
      _ => None,
    })
    .collect()
}
