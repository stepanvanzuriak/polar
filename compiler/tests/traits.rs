use polar_compiler::{
  CompileOptions, Stage,
  core::lower::{Resolved, lower_with},
  dump_stage_with,
  shared::diagnostic::{Diagnostic, DiagnosticBag},
  shared::modules::ModuleSource,
  shared::source::SourceFile,
  syntax::lexer::lex,
  syntax::parser::parse,
};

const DESCRIBE: &str = "traits
  Describe<a> {
    describe(value: a) -> String
  }
";

fn recipe_module(src: &str) -> String {
  format!(
    "{}  }} derive {{\n    variant(name: String, args: List<String>) -> String {{\n      name\n    }}\n  }}\n\n{src}",
    DESCRIBE.trim_end_matches("  }\n")
  )
}

fn module(src: &str) -> String {
  format!("{DESCRIBE}\n{src}")
}

fn source(path: &str, src: &str) -> ModuleSource {
  ModuleSource {
    path: path.to_string(),
    source: src.to_string(),
    specifier: format!("./{}.js", path.to_lowercase()),
    plugins: Vec::new(),
  }
}

fn diagnostics_with(src: &str, modules: Vec<ModuleSource>) -> Vec<Diagnostic> {
  let options = CompileOptions { modules, ..CompileOptions::default() };

  dump_stage_with(src, "test.px", Stage::Core, &options).diagnostics
}

fn errors_with(src: &str, modules: Vec<ModuleSource>) -> Vec<String> {
  let file = SourceFile::new("test.px", src);

  diagnostics_with(src, modules)
    .iter()
    .map(|d| format!("{} at {:?}", d.code, file.slice(&d.primary.span)))
    .collect()
}

fn errors(src: &str) -> Vec<String> {
  errors_with(src, Vec::new())
}

fn diagnostic(src: &str) -> Diagnostic {
  let diagnostics = diagnostics_with(src, Vec::new());

  assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
  diagnostics.into_iter().next().unwrap()
}

fn resolved_at(
  src: &str,
  needle: &str,
  nth: usize,
  modules: &[ModuleSource],
) -> Option<Resolved> {
  let file = SourceFile::new("test.px", src);
  let mut bag = DiagnosticBag::default();
  let parsed = parse(&file, &lex(&file, &mut bag), &mut bag);
  let lowered = lower_with(&parsed, modules, &mut bag);

  assert!(!bag.has_errors(), "{:#?}", bag.into_sorted());

  let start = src.match_indices(needle).nth(nth).expect("needle").0;

  lowered.resolutions.0.get(&(start, start + needle.len())).cloned()
}

const SHAPES: &str = "module Shapes

traits
  Describe<a> {
    describe(value: a) -> String
  }

types
  Circle = Circle(Float)

impls
  Describe for Circle {
    describe(value) {
      \"circle\"
    }
  }

exports
  Describe { describe }
";

#[test]
fn method_resolves() {
  let src = module("functions\n  f(x) {\n    describe(x)\n  }\n");

  assert_eq!(
    resolved_at(&src, "describe", 1, &[]),
    Some(Resolved::Method {
      trait_path: "Describe".to_string(),
      method: "describe".to_string()
    })
  );
}

#[test]
fn local_shadows_method() {
  let src = module("functions\n  f(describe) {\n    describe\n  }\n");

  assert!(matches!(
    resolved_at(&src, "describe", 2, &[]),
    Some(Resolved::Local(_))
  ));
}

#[test]
fn method_clashes_with_function() {
  let src = module("functions\n  describe(x) {\n    x\n  }\n");

  assert_eq!(errors(&src), ["POLAR0302 at \"describe\""]);

  let d = diagnostic(&src);

  assert_eq!(d.primary.span.start, src.rfind("describe(x)").unwrap());
}

#[test]
fn unknown_trait_in_derive() {
  let src = module("types\n  Status = A | B derive(Descrbe)\n");
  let d = diagnostic(&src);

  assert_eq!(errors(&src), ["POLAR0701 at \"Descrbe\""]);
  assert!(d.help.iter().any(|h| h.contains("`Describe`")), "{d:#?}");
}

#[test]
fn unknown_trait_in_where() {
  let src =
    module("functions\n  f(x: a) -> String where Shw<a> {\n    \"\"\n  }\n");

  assert_eq!(errors(&src), ["POLAR0701 at \"Shw\""]);
}

#[test]
fn trait_resolves_in_where_and_derive() {
  let src = recipe_module(
    "types\n  Status = A | B derive(Describe)\n\nfunctions\n  f(x: a) -> String where Describe<a> {\n    \"\"\n  }\n",
  );

  for nth in [1, 2] {
    assert_eq!(
      resolved_at(&src, "Describe", nth, &[]),
      Some(Resolved::Trait("Describe".to_string()))
    );
  }
}

const TWO: &str = "traits
  Describe<a> {
    describe(value: a) -> String
    label(value: a) -> String
  }

types
  Status = Draft | Published(Int)
";

#[test]
fn impl_missing_method() {
  let src = format!(
    "{TWO}\nimpls\n  Describe for Status {{\n    describe(value) {{\n      \"\"\n    }}\n  }}\n"
  );
  let d = diagnostic(&src);

  assert_eq!(errors(&src), ["POLAR0703 at \"Describe for Status\""]);
  assert!(d.message.contains("`label`"), "{d:#?}");
  assert!(d.help.iter().any(|h| h.contains("label")), "{d:#?}");
}

#[test]
fn impl_unknown_method() {
  let src = format!(
    "{TWO}\nimpls\n  Describe for Status {{\n    describe(value) {{\n      \"\"\n    }}\n\n    label(value) {{\n      \"\"\n    }}\n\n    descrbe(value) {{\n      \"\"\n    }}\n  }}\n"
  );
  let d = diagnostic(&src);

  assert_eq!(errors(&src), ["POLAR0702 at \"descrbe\""]);
  assert!(d.help.iter().any(|h| h.contains("`describe`")), "{d:#?}");
}

fn describe_impl(target: &str) -> String {
  format!(
    "  Describe for {target} {{\n    describe(value) {{\n      \"\"\n    }}\n  }}\n"
  )
}

#[test]
fn impl_duplicate() {
  let src = module(&format!(
    "types\n  Status = Draft | Published(Int)\n\nimpls\n{}\n{}",
    describe_impl("Status"),
    describe_impl("Status")
  ));
  let d = diagnostic(&src);

  assert_eq!(errors(&src), ["POLAR0705 at \"Describe for Status\""]);
  assert_eq!(d.primary.span.start, src.rfind("Describe for Status").unwrap());
  assert_eq!(d.secondary.len(), 1);
}

#[test]
fn impl_duplicate_by_name() {
  let src = module(&format!(
    "types\n  Box<a> = Box(a)\n\nimpls\n{}\n{}",
    describe_impl("Box<a>"),
    describe_impl("Box<Int>")
  ));

  assert_eq!(errors(&src), ["POLAR0705 at \"Describe for Box<Int>\""]);
}

const GEO: &str = "module Geo

types
  Point = Point(Float, Float)
";

const TRAIT_ONLY: &str = "module Shapes

traits
  Describe<a> {
    describe(value: a) -> String
  }

exports
  Describe { describe }
";

#[test]
fn orphan() {
  let src = format!(
    "module A\n\nuses\n  Shapes\n  Geo\n\nimpls\n{}",
    describe_impl("Point")
  );
  let modules = vec![source("Shapes", TRAIT_ONLY), source("Geo", GEO)];
  let found = errors_with(&src, modules);

  assert_eq!(found, ["POLAR0704 at \"Describe for Point\""]);
}

#[test]
fn orphan_ok_either_side() {
  let beside_trait = format!(
    "module Shapes\n\nuses\n  Geo\n\ntraits\n  Describe<a> {{\n    describe(value: a) -> String\n  }}\n\nimpls\n{}\nexports\n  Describe {{ describe }}\n",
    describe_impl("Point")
  );

  assert!(errors_with(&beside_trait, vec![source("Geo", GEO)]).is_empty());

  let beside_type = format!(
    "module Geo\n\nuses\n  Shapes\n\ntypes\n  Point = Point(Float, Float)\n\nimpls\n{}",
    describe_impl("Point")
  );

  assert!(
    errors_with(&beside_type, vec![source("Shapes", TRAIT_ONLY)]).is_empty()
  );
}

#[test]
fn export_needs_list() {
  let src = module("exports\n  Describe\n");
  let d = diagnostic(&src);

  assert_eq!(errors(&src), ["POLAR0706 at \"Describe\""]);
  assert!(d.help.iter().any(|h| h.contains("Describe { describe }")), "{d:#?}");
}

#[test]
fn export_incomplete() {
  let src = format!("{TWO}\nexports\n  Describe {{ describe }}\n");
  let d = diagnostic(&src);

  assert_eq!(errors(&src), ["POLAR0706 at \"Describe { describe }\""]);
  assert!(d.help.iter().any(|h| h.contains("`label`")), "{d:#?}");
}

#[test]
fn export_list_on_a_function() {
  let src = module(
    "functions\n  main() {\n    1\n  }\n\nexports\n  main { describe }\n",
  );

  assert_eq!(errors(&src), ["POLAR0706 at \"main\""]);
}

#[test]
fn import_bare() {
  let src = "uses\n  Shapes { describe }\n\nfunctions\n  f(x) {\n    describe(x)\n  }\n";

  assert_eq!(
    resolved_at(src, "describe", 1, &[source("Shapes", SHAPES)]),
    Some(Resolved::Method {
      trait_path: "Shapes.Describe".to_string(),
      method: "describe".to_string()
    })
  );
}

#[test]
fn import_qualified() {
  let modules = vec![source("Shapes", SHAPES)];
  let src =
    "uses\n  Shapes\n\nfunctions\n  f(x) {\n    Shapes.describe(x)\n  }\n";

  assert_eq!(
    resolved_at(src, "describe", 0, &modules),
    Some(Resolved::Method {
      trait_path: "Shapes.Describe".to_string(),
      method: "describe".to_string()
    })
  );

  let bare = "uses\n  Shapes\n\nfunctions\n  f(x) {\n    describe(x)\n  }\n";

  assert_eq!(errors_with(bare, modules), ["POLAR0301 at \"describe\""]);
}

#[test]
fn import_brings_the_trait_name() {
  let modules = vec![source("Shapes", SHAPES)];
  let src = "uses\n  Shapes\n\nfunctions\n  f(x: a) -> String where Describe<a> {\n    Shapes.describe(x)\n  }\n";

  assert_eq!(
    resolved_at(src, "Describe", 0, &modules),
    Some(Resolved::Trait("Shapes.Describe".to_string()))
  );
}

#[test]
fn import_unknown_method() {
  let src = "uses\n  Shapes { descrbe }\n";
  let modules = vec![source("Shapes", SHAPES)];
  let found = diagnostics_with(src, modules.clone());

  assert_eq!(errors_with(src, modules), ["POLAR0305 at \"descrbe\""]);
  assert!(found[0].help.iter().any(|h| h.contains("`describe`")), "{found:#?}");
}

#[test]
fn import_clash() {
  let src =
    "uses\n  Shapes { describe }\n\nfunctions\n  describe(x) {\n    x\n  }\n";
  let modules = vec![source("Shapes", SHAPES)];

  assert_eq!(errors_with(src, modules), ["POLAR0302 at \"describe\""]);

  let d = &diagnostics_with(src, vec![source("Shapes", SHAPES)])[0];

  assert_eq!(d.primary.span.start, src.find("describe").unwrap());
}

#[test]
fn impl_duplicate_across_modules() {
  let src = format!(
    "module Main\n\nuses\n  Shapes\n\ntypes\n  Square = Square(Float)\n\nimpls\n{}",
    describe_impl("Circle")
  );
  let modules = vec![source("Shapes", SHAPES)];

  assert_eq!(
    errors_with(&src, modules),
    ["POLAR0704 at \"Describe for Circle\""]
  );
}

#[test]
fn impl_methods_lower_into_core() {
  let src = module(&format!(
    "types\n  Status = Draft | Published(Int)\n\nimpls\n{}",
    describe_impl("Status")
  ));
  let out =
    dump_stage_with(&src, "test.px", Stage::Core, &CompileOptions::default());

  assert!(out.diagnostics.is_empty(), "{:#?}", out.diagnostics);

  let core = out.output.unwrap();

  assert!(core.contains("(impl Describe Status"), "{core}");
  assert!(core.contains("(method describe/"), "{core}");
}
