mod common;

use common::{invariants::check_invariants, sexp::sexp};
use polar_compiler::{
  CompileOptions, compile,
  core::{dump::dump_module_shape, lower},
  format,
  shared::codes::DiagnosticCode::{
    self, ConstantCalled, ConstantUsesBelow, DeclWrongZone,
    DuplicateDefinition, UnknownName, ZoneOutOfOrder,
  },
  shared::diagnostic::{Diagnostic, DiagnosticBag},
  shared::source::SourceFile,
  stdlib,
  syntax::ast::{Decl, ZoneKind, dump::dump_ast_shape},
  syntax::lexer::lex,
  syntax::parser::parse,
};

fn parse_src(src: &str) -> (String, Vec<Diagnostic>) {
  let file = SourceFile::new("test.px", src);
  let mut bag = DiagnosticBag::default();
  let module = parse(&file, &lex(&file, &mut bag), &mut bag);

  check_invariants(src, &module);

  (dump_ast_shape(&module), bag.into_sorted())
}

fn lower_src(src: &str) -> (String, Vec<Diagnostic>) {
  let file = SourceFile::new("test.px", src);
  let mut bag = DiagnosticBag::default();
  let module = parse(&file, &lex(&file, &mut bag), &mut bag);

  assert!(!bag.has_errors(), "the fixture must parse: {:?}", bag.into_sorted());

  let core = lower(&module, &mut bag);

  (dump_module_shape(&core), bag.into_sorted())
}

fn codes(diagnostics: &[Diagnostic]) -> Vec<DiagnosticCode> {
  diagnostics.iter().map(|d| d.code).collect()
}

fn lower_codes(src: &str) -> Vec<DiagnosticCode> {
  codes(&lower_src(src).1)
}

fn js(src: &str) -> String {
  let out = compile(src, "test.px", &CompileOptions::default());

  assert!(out.diagnostics.is_empty(), "{:?}", out.diagnostics);
  out.js
}

const GEO: &str = "\
constants
  /// Half a turn.
  pi: Float = 3.14
  tau = pi * 2.0

functions
  area(r) {
    pi * r * r
  }

exports
  area
  pi
";

mod parse {
  use super::*;

  #[test]
  fn with_and_without_a_type() {
    let (dump, diagnostics) =
      parse_src("constants\n  pi: Float = 3.14\n  tau = pi * 2.0\n");

    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert_eq!(
      dump,
      sexp!(Module
        (Zone constants
          (ConstDecl pi (TypeRef Float) (FloatLit raw="3.14"))
          (ConstDecl tau (Binary * (Var pi) (FloatLit raw="2.0")))))
    );
  }

  #[test]
  fn keeps_docs() {
    let file = SourceFile::new("test.px", GEO);
    let mut bag = DiagnosticBag::default();
    let module = parse(&file, &lex(&file, &mut bag), &mut bag);

    assert_eq!(module.zones[0].kind, ZoneKind::CONSTANTS);

    let Decl::Const(pi) = &module.zones[0].decls[0] else {
      panic!("expected a constant")
    };

    assert_eq!(pi.docs.len(), 1);
  }

  #[test]
  fn typed_constant_in_another_zone() {
    let (_, diagnostics) = parse_src("functions\n  pi: Float = 3.14\n");

    assert_eq!(codes(&diagnostics), [DeclWrongZone]);
    assert_eq!(
      diagnostics[0].message,
      "a constant belongs in the `constants` zone, not `functions`"
    );
  }

  #[test]
  fn comes_after_types_and_before_functions() {
    let (_, diagnostics) =
      parse_src("functions\n  f() { 1 }\n\nconstants\n  x = 1\n");

    assert_eq!(codes(&diagnostics), [ZoneOutOfOrder]);

    let (_, diagnostics) =
      parse_src("constants\n  x = 1\n\ntypes\n  T = Int\n");

    assert_eq!(codes(&diagnostics), [ZoneOutOfOrder]);
  }
}

mod fmt {
  use super::*;

  #[test]
  fn round_trips() {
    assert_eq!(format(GEO, "test.px").output.as_deref(), Some(GEO));
  }

  #[test]
  fn normalises_spacing() {
    let src = "constants\n  pi:Float=3.14\n  tau   =  pi*2.0\n";

    assert_eq!(
      format(src, "test.px").output.as_deref(),
      Some("constants\n  pi: Float = 3.14\n  tau = pi * 2.0\n")
    );
  }
}

mod lowering {
  use super::*;

  #[test]
  fn constants_come_first_and_can_be_exported() {
    let (dump, diagnostics) = lower_src(GEO);

    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert!(
      dump
        .starts_with("(module\n  (const pi export (lit 3.14))\n  (const tau "),
      "{dump}"
    );
    assert!(dump.contains("(fn area export"), "{dump}");
  }

  #[test]
  fn earlier_constants_and_constructors() {
    let src = "types\n  T = A(Int)\n\nconstants\n  one = 1\n  a = A(one)\n";

    assert_eq!(lower_codes(src), []);
  }

  #[test]
  fn a_later_constant() {
    let (_, diagnostics) = lower_src("constants\n  a = b\n  b = 1\n");

    assert_eq!(codes(&diagnostics), [ConstantUsesBelow]);
    assert_eq!(
      diagnostics[0].message,
      "a constant cannot use the constant `b`"
    );
  }

  #[test]
  fn itself() {
    assert_eq!(lower_codes("constants\n  a = a\n"), [ConstantUsesBelow]);
  }

  #[test]
  fn a_function() {
    let (_, diagnostics) =
      lower_src("constants\n  a = f()\n\nfunctions\n  f() {\n    1\n  }\n");

    assert_eq!(codes(&diagnostics), [ConstantUsesBelow]);
    assert_eq!(
      diagnostics[0].message,
      "a constant cannot use the function `f`"
    );
  }

  #[test]
  fn a_function_inside_a_lambda() {
    let src =
      "constants\n  a = function() { f() }\n\nfunctions\n  f() {\n    1\n  }\n";

    assert_eq!(lower_codes(src), [ConstantUsesBelow]);
  }

  #[test]
  fn locals_shadow_constants() {
    let src = "constants\n  x = 1\n\nfunctions\n  f(x) {\n    x()\n  }\n";

    assert_eq!(lower_codes(src), []);
  }

  #[test]
  fn called() {
    let (_, diagnostics) =
      lower_src("constants\n  pi = 3\n\nfunctions\n  f() {\n    pi()\n  }\n");

    assert_eq!(codes(&diagnostics), [ConstantCalled]);
    assert_eq!(diagnostics[0].message, "`pi` is a constant, not a function");
  }

  #[test]
  fn clashes_with_a_function() {
    let src = "constants\n  f = 1\n\nfunctions\n  f() {\n    1\n  }\n";

    assert_eq!(lower_codes(src), [DuplicateDefinition]);
    assert_eq!(
      lower_codes("constants\n  a = 1\n  a = 2\n"),
      [DuplicateDefinition]
    );
  }

  #[test]
  fn unknown_export() {
    let (_, diagnostics) = lower_src("constants\n  a = 1\n\nexports\n  b\n");

    assert_eq!(codes(&diagnostics), [UnknownName]);
    assert_eq!(
      diagnostics[0].message,
      "cannot find function or constant `b` to export"
    );
    assert_eq!(diagnostics[0].help.as_deref(), Some("did you mean `a`?"));
  }

  #[test]
  fn exported_type() {
    for ty in ["Pair = { name: String }", "Tag = Tag(Int)"] {
      let name = &ty[..ty.find(' ').unwrap()];
      let (_, diagnostics) =
        lower_src(&format!("types\n  {ty}\n\nexports\n  {name}\n"));

      assert_eq!(codes(&diagnostics), []);
    }
  }
}

mod emission {
  use super::*;

  #[test]
  fn plain_values_are_consts() {
    let out = js(GEO);

    assert!(out.contains("export const pi = 3.14;"), "{out}");
    assert!(out.contains("\nconst tau = pi * 2;"), "{out}");
    assert!(out.contains("return pi * r * r;"), "{out}");
  }

  #[test]
  fn statements_run_in_a_sync_arrow() {
    let src = "constants\n  x = if 1 < 2 {\n    let y = 3\n    y\n  } else {\n    4\n  }\n\nexports\n  x\n";
    let out = js(src);

    assert!(out.contains("export const x = (() => {"), "{out}");
    assert!(!out.contains("await"), "{out}");
  }
}

mod interface {
  use super::*;

  #[test]
  fn lists_exported_constants() {
    let (_, interface) = stdlib::interface_of("geo.px", GEO);

    assert_eq!(interface.constants, ["pi"]);
    assert!(interface.constant("pi"));
    assert!(!interface.constant("tau"));
    assert_eq!(interface.function("area"), Some(1));
    assert_eq!(interface.function("pi"), None);
  }
}
