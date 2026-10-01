mod common;

use std::path::{Path, PathBuf};

use common::invariants::check_invariants;
use polar_compiler::{
  format,
  shared::codes::DiagnosticCode::{
    self, ExpectedDeclaration, MethodNeedsAnnotation,
    RedundantDeclarationKeyword, TraitParamCount, UnknownRecipeCase,
    ZoneOutOfOrder,
  },
  shared::diagnostic::{Diagnostic, DiagnosticBag},
  shared::source::{SourceFile, Span},
  syntax::ast::{
    Decl, ImplDecl, Module, Name, TraitDecl, ZoneKind, eq::ast_eq,
  },
  syntax::lexer::{
    lex,
    token::{TokenKind, keyword},
  },
  syntax::parser::parse,
};

fn parse_module(src: &str) -> (Module, Vec<Diagnostic>) {
  let file = SourceFile::new("test.px", src);
  let mut bag = DiagnosticBag::default();
  let lexed = lex(&file, &mut bag);
  let module = parse(&file, &lexed, &mut bag);

  (module, bag.into_sorted())
}

fn parse_ok(src: &str) -> Module {
  let (module, diagnostics) = parse_module(src);

  assert!(diagnostics.is_empty(), "{diagnostics:#?}");
  check_invariants(src, &module);
  module
}

fn errors(src: &str) -> Vec<(DiagnosticCode, String)> {
  let (_, diagnostics) = parse_module(src);

  diagnostics
    .iter()
    .map(|d| {
      let span = &d.primary.span;

      (d.code, src[span.start..span.end].to_string())
    })
    .collect()
}

fn first_trait(module: &Module) -> &TraitDecl {
  module
    .zones
    .iter()
    .flat_map(|z| &z.decls)
    .find_map(|d| match d {
      Decl::Trait(t) => Some(t),
      _ => None,
    })
    .expect("a trait")
}

fn first_impl(module: &Module) -> &ImplDecl {
  module
    .zones
    .iter()
    .flat_map(|z| &z.decls)
    .find_map(|d| match d {
      Decl::Impl(i) => Some(i),
      _ => None,
    })
    .expect("an impl")
}

fn formatted(src: &str) -> String {
  let result = format(src, "test.px");

  result.output.unwrap_or_else(|| panic!("{:#?}", result.diagnostics))
}

#[test]
fn keywords() {
  for (text, kind) in [
    ("traits", TokenKind::KwTraits),
    ("impls", TokenKind::KwImpls),
    ("for", TokenKind::KwFor),
    ("where", TokenKind::KwWhere),
    ("trait", TokenKind::KwTrait),
    ("impl", TokenKind::KwImpl),
  ] {
    assert_eq!(keyword(text), Some(kind), "{text}");
  }
}

#[test]
fn display_names() {
  assert_eq!(TokenKind::KwImpls.display_name(), "keyword `impls`");
  assert_eq!(TokenKind::KwTraits.display_name(), "keyword `traits`");
  assert_eq!(TokenKind::KwWhere.display_name(), "keyword `where`");
}

fn px_files(dir: &Path, out: &mut Vec<PathBuf>) {
  for entry in std::fs::read_dir(dir).expect("read a directory") {
    let path = entry.expect("a directory entry").path();

    if path.is_dir() {
      px_files(&path, out);
    } else if path.extension().is_some_and(|e| e == "px") {
      out.push(path);
    }
  }
}

#[test]
fn no_repo_file_uses_new_keywords() {
  let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
  let mut files = Vec::new();

  for dir in ["compiler/tests/fixtures/programs", "std", "projects"] {
    px_files(&root.join(dir), &mut files);
  }

  assert!(!files.is_empty());

  for path in files {
    let src = std::fs::read_to_string(&path).expect("read a .px file");
    let file = SourceFile::new("repo.px", src.as_str());
    let mut bag = DiagnosticBag::default();
    let lexed = lex(&file, &mut bag);

    parse(&file, &lexed, &mut bag);

    let diagnostics = bag.into_sorted();
    let uses_a_plugin = diagnostics.iter().any(|d| {
      d.help.as_deref().is_some_and(|h| h.contains("is a plugin zone"))
    });

    assert!(
      uses_a_plugin || diagnostics.is_empty(),
      "{}: {diagnostics:#?}",
      path.display()
    );
  }
}

#[test]
fn zone_order() {
  assert!(ZoneKind::USES < ZoneKind::TRAITS);
  assert!(ZoneKind::TRAITS < ZoneKind::TYPES);
  assert!(ZoneKind::FUNCTIONS < ZoneKind::IMPLS);
  assert!(ZoneKind::IMPLS < ZoneKind::EXPORTS);
}

#[test]
fn codes_registered() {
  let all: Vec<u16> =
    DiagnosticCode::ALL.iter().map(|code| *code as u16).collect();

  for code in (221..=223).chain(701..=711) {
    assert!(all.contains(&code), "POLAR{code:04} is not registered");
  }
}

#[test]
fn ast_eq_ignores_spans() {
  let a = parse_ok(
    "traits\n  Describe<a> {\n    describe(value: a) -> String\n  }\n",
  );
  let b = parse_ok(
    "traits\n\n\n  Describe<a>   {\n      describe( value : a )  ->  String\n  }\n",
  );

  assert!(ast_eq(&a, &b));
  assert_ne!(first_trait(&a).span, first_trait(&b).span);
}

#[test]
fn trait_basic() {
  let module = parse_ok(
    "traits\n  Describe<a> {\n    describe(value: a) -> String\n  }\n",
  );
  let t = first_trait(&module);

  assert_eq!(t.name.text, "Describe");
  assert_eq!(t.param.text, "a");
  assert_eq!(t.methods.len(), 1);
  assert_eq!(t.methods[0].name.text, "describe");
  assert!(t.recipe.is_none());
}

#[test]
fn trait_two_methods_with_docs() {
  let src = "traits\n  Describe<a> {\n    describe(value: a) -> String\n    /// A label.\n    label(value: a, prefix: String) -> String\n  }\n";
  let module = parse_ok(src);
  let t = first_trait(&module);

  assert_eq!(t.methods.len(), 2);
  assert!(t.methods[0].docs.is_empty());
  assert_eq!(t.methods[1].docs.len(), 1);

  let doc: &Span = &t.methods[1].docs[0];

  assert_eq!(&src[doc.start..doc.end], "/// A label.");
}

#[test]
fn trait_param_count() {
  let src = "traits\n  Pair<a, b> {\n    left(value: Pair) -> Int\n  }\n";

  assert_eq!(errors(src), [(TraitParamCount, "<a, b>".to_string())]);
}

#[test]
fn trait_no_param() {
  let src = "traits\n  Show {\n    show(value: Int) -> String\n  }\n";

  assert_eq!(errors(src), [(TraitParamCount, "Show".to_string())]);
}

#[test]
fn method_needs_annotation() {
  let src = "traits\n  Describe<a> {\n    describe(value) -> String\n  }\n";

  assert_eq!(errors(src), [(MethodNeedsAnnotation, "value".to_string())]);
}

#[test]
fn method_needs_return() {
  let src = "traits\n  Describe<a> {\n    describe(value: a)\n  }\n";

  assert_eq!(errors(src), [(MethodNeedsAnnotation, "describe".to_string())]);
}

const RECIPE: &str = "traits
  Describe<a> {
    describe(value: a) -> String
  } derive {
    record(fields: List<{ name: String, value: String }>) -> String {
      \"r\"
    }

    variant(name: String, args: List<String>) -> String {
      name
    }
  }
";

#[test]
fn recipe() {
  let module = parse_ok(RECIPE);
  let recipe = first_trait(&module).recipe.as_ref().expect("a recipe");
  let names: Vec<&str> =
    recipe.cases.iter().map(|c| c.name.text.as_str()).collect();

  assert_eq!(names, ["record", "variant"]);
}

#[test]
fn recipe_unknown_case() {
  let src = "traits\n  Describe<a> {\n    describe(value: a) -> String\n  } derive {\n    tuple(x: Int) -> Int { x }\n  }\n";

  assert_eq!(errors(src), [(UnknownRecipeCase, "tuple".to_string())]);
}

#[test]
fn recipe_repeated_case() {
  let src = "traits\n  Describe<a> {\n    describe(value: a) -> String\n  } derive {\n    variant(x: Int) -> Int { x }\n    variant(x: Int) -> Int { x }\n  }\n";
  let found = errors(src);

  assert_eq!(found.len(), 1, "{found:?}");
  assert_eq!(found[0].0, UnknownRecipeCase);
}

#[test]
fn impl_with_bounds() {
  let src = "impls\n  Describe for List<a> where Describe<a> {\n    describe(value) { \"x\" }\n  }\n";
  let module = parse_ok(src);
  let Decl::Impl(imp) = &module.zones[0].decls[0] else {
    panic!("not an impl")
  };

  assert_eq!(imp.trait_name.text, "Describe");
  assert_eq!(imp.target.name.text, "List");
  assert_eq!(imp.bounds.len(), 1);
  assert_eq!(imp.bounds[0].var.text, "a");
}

#[test]
fn impl_methods_unannotated() {
  let module = parse_ok(
    "impls\n  Describe for Status {\n    describe(value) { \"x\" }\n  }\n",
  );
  let imp = first_impl(&module);

  assert_eq!(imp.methods.len(), 1);
  assert!(imp.methods[0].params[0].ty.is_none());
  assert!(imp.methods[0].return_type.is_none());
}

fn first_fn(module: &Module) -> &polar_compiler::syntax::ast::FnDecl {
  module
    .zones
    .iter()
    .flat_map(|z| &z.decls)
    .find_map(|d| match d {
      Decl::Fn(f) => Some(f),
      _ => None,
    })
    .expect("a function")
}

fn bound_names(bounds: &[polar_compiler::syntax::ast::Bound]) -> Vec<String> {
  bounds
    .iter()
    .map(|b| format!("{}<{}>", b.trait_name.text, b.var.text))
    .collect()
}

#[test]
fn fn_where() {
  let module = parse_ok(
    "functions\n  f(x: a) -> String where Show<a>, Eq<a> {\n    \"\"\n  }\n",
  );

  assert_eq!(bound_names(&first_fn(&module).bounds), ["Show<a>", "Eq<a>"]);
}

#[test]
fn fn_where_after_effects() {
  let module = parse_ok(
    "functions\n  f(x: a) -> String / {Db} where Show<a> {\n    \"\"\n  }\n",
  );
  let f = first_fn(&module);

  assert!(f.effects.is_some());
  assert_eq!(bound_names(&f.bounds), ["Show<a>"]);
}

#[test]
fn fn_where_needs_a_trait() {
  let src = "functions\n  f(x: a) -> String where {\n    \"\"\n  }\n";
  let (_, diagnostics) = parse_module(src);

  assert!(
    diagnostics.iter().any(|d| d.message.contains("expected a trait name")),
    "{diagnostics:#?}"
  );
}

fn names(list: Option<&Vec<Name>>) -> Option<Vec<&str>> {
  list.map(|names| names.iter().map(|n| n.text.as_str()).collect())
}

#[test]
fn export_method_list() {
  let module = parse_ok("exports\n  Describe { describe, label }\n  main\n");
  let exports: Vec<_> = module.zones[0]
    .decls
    .iter()
    .map(|d| match d {
      Decl::Export(e) => (e.name.text.as_str(), names(e.methods.as_ref())),
      other => panic!("not an export: {other:?}"),
    })
    .collect();

  assert_eq!(
    exports,
    [("Describe", Some(vec!["describe", "label"])), ("main", None)]
  );
}

#[test]
fn export_empty_method_list() {
  let module = parse_ok("exports\n  Describe {}\n");
  let Decl::Export(e) = &module.zones[0].decls[0] else { panic!() };

  assert_eq!(names(e.methods.as_ref()), Some(vec![]));
}

#[test]
fn uses_method_list() {
  let module = parse_ok("uses\n  Shapes as S { describe }\n");
  let Decl::Import(import) = &module.zones[0].decls[0] else {
    panic!("not an import")
  };

  assert_eq!(import.alias.as_ref().map(|a| a.text.as_str()), Some("S"));
  assert_eq!(names(import.methods.as_ref()), Some(vec!["describe"]));
}

#[test]
fn redundant_trait_keyword() {
  let src = "traits\n  trait Show<a> {\n    show(value: a) -> String\n  }\n";

  assert_eq!(errors(src), [(RedundantDeclarationKeyword, "trait".to_string())]);
}

#[test]
fn redundant_impl_keyword() {
  let src = "impls\n  impl Show for Int {\n    show(value) { \"\" }\n  }\n";

  assert_eq!(errors(src), [(RedundantDeclarationKeyword, "impl".to_string())]);
}

#[test]
fn zone_order_is_checked() {
  let src = "impls\n  Show for Int {\n    show(value) { \"\" }\n  }\n\nfunctions\n  f() {\n    1\n  }\n";
  let codes: Vec<DiagnosticCode> =
    errors(src).into_iter().map(|(c, _)| c).collect();

  assert_eq!(codes, [ZoneOutOfOrder]);
}

#[test]
fn recovery() {
  let src = "traits\n  Describe<a> {\n    describe(value: a) ->\n  }\n\nimpls\n  Describe for Status {\n    describe(value) { \"x\" }\n  }\n";
  let (module, diagnostics) = parse_module(src);

  assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
  assert_eq!(first_impl(&module).trait_name.text, "Describe");
}

#[test]
fn not_a_declaration_in_traits() {
  let src = "traits\n  42\n";
  let (_, diagnostics) = parse_module(src);

  assert_eq!(diagnostics[0].code, ExpectedDeclaration);
  assert!(
    diagnostics[0].help.iter().any(|h| h.contains("Show<a>")),
    "{diagnostics:#?}"
  );
}

const CANONICAL: &str = "traits
  /// Something that can be described.
  Describe<a> {
    describe(value: a) -> String
    label(value: a, prefix: String) -> String
  } derive {
    record(fields: List<{ name: String, value: String }>) -> String {
      \"r\"
    }

    variant(name: String, args: List<String>) -> String {
      name
    }
  }

impls
  Describe for List<a> where Describe<a> {
    describe(value: List<a>) -> String {
      \"list\"
    }

    label(value, prefix) {
      prefix
    }
  }

exports
  Describe { describe, label }
";

#[test]
fn format_trait() {
  let squashed = "traits\n  /// Something that can be described.\n  Describe<a> { describe(value: a) -> String\n label(value: a, prefix: String) -> String } derive { record(fields: List<{ name: String, value: String }>) -> String { \"r\" } variant(name: String, args: List<String>) -> String { name } }\n\nimpls\n  Describe for List<a> where Describe<a> { describe(value: List<a>) -> String { \"list\" } label(value, prefix) { prefix } }\n\nexports\n  Describe {describe,label}\n";

  assert_eq!(formatted(squashed), CANONICAL);
}

#[test]
fn format_is_idempotent() {
  assert_eq!(formatted(CANONICAL), CANONICAL);
  assert_eq!(formatted(&formatted(RECIPE)), formatted(RECIPE));
}

#[test]
fn format_keeps_comments() {
  let src = "traits
  Describe<a> {
    describe(value: a) -> String
    // between the methods
    label(value: a) -> String
  }

impls
  Describe for Int {
    // inside the impl
    describe(value) {
      \"int\"
    }

    label(value) {
      \"int\"
    }
  }
";

  assert_eq!(formatted(src), src);
}

#[test]
fn format_long_where() {
  let src = "functions\n  describe_everything(first: a, second: b, third: c) -> String where Describe<a>, Describe<b>, Show<c> {\n    \"\"\n  }\n";
  let out = formatted(src);

  assert_eq!(
    out,
    "functions\n  describe_everything(first: a, second: b, third: c) -> String\n    where Describe<a>, Describe<b>, Show<c> {\n    \"\"\n  }\n"
  );
  assert_eq!(formatted(&out), out);
}

#[test]
fn format_short_where_stays() {
  let src = "functions\n  f(x: a) -> String where Show<a> {\n    \"\"\n  }\n";

  assert_eq!(formatted(src), src);
}

#[test]
fn format_method_lists() {
  assert_eq!(
    formatted("exports\n  Describe {describe,label}\n"),
    "exports\n  Describe { describe, label }\n"
  );
  assert_eq!(
    formatted("uses\n  Shapes as S {describe}\n"),
    "uses\n  Shapes as S { describe }\n"
  );
  assert_eq!(formatted("exports\n  Describe {}\n"), "exports\n  Describe {}\n");
}

#[test]
fn format_long_method_list() {
  let src = "exports\n  Describe { describe_the_first_thing, describe_the_second_thing, describe_the_third_thing }\n";

  assert_eq!(
    formatted(src),
    "exports\n  Describe {\n    describe_the_first_thing,\n    describe_the_second_thing,\n    describe_the_third_thing,\n  }\n"
  );
}
