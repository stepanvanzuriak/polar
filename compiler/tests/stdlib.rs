use polar_compiler::{
  CompileOptions, compile,
  core::{dump::dump_module, lower},
  format,
  shared::codes::DiagnosticCode::{
    self, BuiltinModuleAsValue, CallArity, ConstantCalled, DuplicateDefinition,
    ImportNotSupported, UnknownBuiltinMember, UnknownStdModule,
  },
  shared::diagnostic::{Diagnostic, DiagnosticBag},
  shared::source::SourceFile,
  stdlib::{self, MODULES},
  syntax::lexer::lex,
  syntax::parser::parse,
};

fn lower_src(src: &str) -> (String, Vec<String>, Vec<Diagnostic>) {
  let file = SourceFile::new("test.px", src);
  let mut bag = DiagnosticBag::default();
  let lexed = lex(&file, &mut bag);
  let module = parse(&file, &lexed, &mut bag);

  assert!(!bag.has_errors(), "the fixture must parse: {:?}", bag.into_sorted());

  let core = lower(&module, &mut bag);

  (dump_module(&core), core.std_imports, bag.into_sorted())
}

fn codes(src: &str) -> Vec<DiagnosticCode> {
  lower_src(src).2.iter().map(|d| d.code).collect()
}

fn using(uses: &str, body: &str) -> String {
  format!("uses\n  {uses}\n\nfunctions\n  main() {{\n    {body}\n  }}\n")
}

mod modules {
  use super::*;

  #[test]
  fn every_module_compiles_cleanly() {
    for m in MODULES {
      let options = CompileOptions {
        runtime: "../runtime.js".to_string(),
        ..CompileOptions::default()
      };
      let out = compile(m.source, &stdlib::filename(m.name), &options);

      assert!(
        out.diagnostics.is_empty(),
        "std/{}.px: {:#?}",
        m.name,
        out.diagnostics
      );
      assert!(
        out.js.starts_with("import * as $rt from \"../runtime.js\";"),
        "{}",
        out.js
      );
    }
  }

  #[test]
  fn every_module_is_formatted() {
    for m in MODULES {
      assert_eq!(
        format(m.source, "std.px").output.as_deref(),
        Some(m.source),
        "std/{}.px",
        m.name
      );
    }
  }

  #[test]
  fn modules_are_sorted_and_named_after_their_header() {
    assert!(MODULES.windows(2).all(|w| w[0].name < w[1].name));

    for m in MODULES {
      let (header, _) =
        stdlib::interface_of(&stdlib::filename(m.name), m.source);

      assert_eq!(header.as_deref(), Some(m.name), "std/{}.px", m.name);
    }
  }

  #[test]
  fn list_interface() {
    let list = stdlib::interface("List").expect("Std.List exists");

    assert_eq!(list.function("map"), Some(2));
    assert_eq!(list.function("fold"), Some(3));
    assert_eq!(list.function("nope"), None);
    assert_eq!(
      list.ctors,
      [
        ("Nil".to_string(), 0, "List".to_string()),
        ("Cons".to_string(), 2, "List".to_string())
      ]
    );
    assert!(stdlib::interface("Nope").is_none());
  }

  #[test]
  fn option_interface() {
    let option = stdlib::interface("Option").expect("Std.Option exists");

    assert_eq!(option.function("map"), Some(2));
    assert_eq!(option.function("with_default"), Some(2));
    assert_eq!(option.function("and_then"), Some(2));
    assert_eq!(
      option.ctors,
      [
        ("None".to_string(), 0, "Option".to_string()),
        ("Some".to_string(), 1, "Option".to_string())
      ]
    );
  }

  #[test]
  fn inline_bodies_are_the_non_recursive_combinators() {
    let names = |module: &str| -> Vec<String> {
      stdlib::interface(module)
        .expect("a std module")
        .inline
        .iter()
        .map(|body| body.decl.sym.name.to_string())
        .collect()
    };

    let option = names("Option");

    assert!(option.contains(&"map".to_string()), "{option:?}");
    assert!(option.contains(&"and_then".to_string()), "{option:?}");
    assert!(names("List").is_empty(), "{:?}", names("List"));
  }

  #[test]
  fn result_interface() {
    let result = stdlib::interface("Result").expect("Std.Result exists");

    assert_eq!(result.function("map"), Some(2));
    assert_eq!(result.function("map_err"), Some(2));
    assert_eq!(result.function("and_then"), Some(2));
    assert_eq!(
      result.ctors,
      [
        ("Err".to_string(), 1, "Result".to_string()),
        ("Ok".to_string(), 1, "Result".to_string())
      ]
    );
  }

  #[test]
  fn math_interface() {
    let math = stdlib::interface("Math").expect("Std.Math exists");

    assert_eq!(math.constants, ["pi"]);
    assert_eq!(math.function("pi"), None);
    assert!(math.ctors.is_empty());
  }

  #[test]
  fn list_imports_option() {
    let list = stdlib::module("List").expect("Std.List exists");
    let options = CompileOptions {
      runtime: "../runtime.js".to_string(),
      ..CompileOptions::default()
    };
    let out = compile(list.source, &stdlib::filename("List"), &options);

    assert!(out.diagnostics.is_empty(), "{:?}", out.diagnostics);
    assert_eq!(out.std_imports, ["Option"]);
    assert!(
      out.js.contains("import * as $Option from \"../std/Option.js\";"),
      "{}",
      out.js
    );
  }

  #[test]
  fn importing_list_does_not_bring_in_option() {
    assert_eq!(codes(&using("Std.List", "List.head(Nil)")), [],);
    assert_eq!(
      codes(&using("Std.List", "Some(1)")),
      [DiagnosticCode::UnknownUppercaseName]
    );
  }

  #[test]
  fn specifiers_sit_beside_the_runtime() {
    assert_eq!(
      stdlib::specifier("./_polar/runtime.js", "List"),
      "./_polar/std/List.js"
    );
    assert_eq!(
      stdlib::specifier("../_polar/runtime.js", "List"),
      "../_polar/std/List.js"
    );
    assert_eq!(stdlib::specifier("../runtime.js", "List"), "../std/List.js");
  }
}

mod imports {
  use super::*;

  #[test]
  fn std_function_lowers_to_std() {
    let (core, imports, diagnostics) =
      lower_src(&using("Std.List", "List.map(Nil, function(x) { x })"));

    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert!(core.contains("(std List map)"), "{core}");
    assert_eq!(imports, ["List"]);
  }

  #[test]
  fn constructors_come_in_unqualified() {
    let src = using(
      "Std.List",
      "match Cons(1, Nil) {\n      Cons(x, _) -> x,\n      Nil -> 0,\n    }",
    );

    assert_eq!(codes(&src), []);
  }

  #[test]
  fn alias() {
    assert_eq!(codes(&using("Std.List as L", "L.length(Nil)")), []);
    assert_eq!(
      codes(&using("Std.List as L", "List.length(Nil)")),
      [DiagnosticCode::UnknownUppercaseName]
    );
  }

  #[test]
  fn builtin_modules_are_part_of_std() {
    assert_eq!(codes(&using("Std.String", "String.trim(\" a \")")), []);
    assert_eq!(
      codes(&using("Std.String as S", "String.trim(\" a \")")),
      [ImportNotSupported]
    );
  }

  #[test]
  fn unknown_std_module() {
    let (_, _, diagnostics) = lower_src(&using("Std.Nope", "1"));

    assert_eq!(diagnostics[0].code, UnknownStdModule);
    assert_eq!(diagnostics[0].message, "there is no std module `Std.Nope`");
    assert!(
      diagnostics[0].help.as_deref().is_some_and(|h| h.contains("`Std.List`"))
    );
  }

  #[test]
  fn std_takes_one_module() {
    assert_eq!(codes("uses\n  Std\n"), [ImportNotSupported]);
    assert_eq!(codes("uses\n  Std.List.Deep\n"), [ImportNotSupported]);
  }

  #[test]
  fn unknown_member() {
    let (_, _, diagnostics) =
      lower_src(&using("Std.List", "List.mapp(Nil, function(x) { x })"));

    assert_eq!(diagnostics[0].code, UnknownBuiltinMember);
    assert_eq!(diagnostics[0].message, "`List` exports no function `mapp`");
    assert!(
      diagnostics[0].help.as_deref().is_some_and(|h| h.contains("`map`"))
    );
  }

  #[test]
  fn arity_is_checked() {
    assert_eq!(codes(&using("Std.List", "List.map(Nil)")), [CallArity]);
  }

  #[test]
  fn module_is_not_a_value() {
    assert_eq!(codes(&using("Std.List", "List")), [BuiltinModuleAsValue]);
  }

  #[test]
  fn clashing_constructor() {
    let src = "uses\n  Std.List\n\ntypes\n  Mine = Nil | Other\n";

    assert_eq!(codes(src), [DuplicateDefinition]);
  }

  #[test]
  fn imported_twice() {
    assert_eq!(codes("uses\n  Std.List\n  Std.List\n"), [DuplicateDefinition]);
  }

  #[test]
  fn constants_are_values() {
    let out = compile(
      &using("Std.Math", "Math.pi * 2.0"),
      "a.px",
      &CompileOptions::default(),
    );

    assert!(out.diagnostics.is_empty(), "{:?}", out.diagnostics);
    assert!(out.js.contains("return $Math.pi * 2;"), "{}", out.js);
  }

  #[test]
  fn constants_are_not_called() {
    let (_, _, diagnostics) = lower_src(&using("Std.Math", "Math.pi()"));

    assert_eq!(diagnostics[0].code, ConstantCalled);
    assert_eq!(
      diagnostics[0].message,
      "`Math.pi` is a constant, not a function"
    );
  }

  #[test]
  fn unknown_member_lists_constants() {
    let (_, _, diagnostics) = lower_src(&using("Std.Math", "Math.tau"));

    assert_eq!(diagnostics[0].code, UnknownBuiltinMember);
    assert_eq!(diagnostics[0].help.as_deref(), Some("available: `pi`"));
  }

  #[test]
  fn compile_reports_imports_and_emits_the_import() {
    let out = compile(
      &using("Std.List", "List.length(Nil)"),
      "a.px",
      &CompileOptions::default(),
    );

    assert!(out.diagnostics.is_empty(), "{:?}", out.diagnostics);
    assert_eq!(out.std_imports, ["List"]);
    assert!(
      out.js.contains("import * as $List from \"./_polar/std/List.js\";"),
      "{}",
      out.js
    );
    assert!(
      out.js.contains("return $List.length({ $: \"Nil\" })"),
      "{}",
      out.js
    );
  }
}

mod list_syntax {
  use super::*;

  #[test]
  fn literals_and_patterns_are_nil_and_cons() {
    let src = using(
      "Std.List",
      "match [1, ..[2]] {\n      [] -> 0,\n      [x] -> x,\n      [x, .._] -> x,\n    }",
    );
    let (dump, _, diagnostics) = lower_src(&src);

    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert!(!dump.contains("Invalid"), "{dump}");
  }

  #[test]
  fn needs_the_list_type() {
    assert_eq!(
      codes("functions\n  main() {\n    []\n  }\n"),
      [DiagnosticCode::ListTypeNotInScope]
    );
  }

  #[test]
  fn formats_back_the_same() {
    let src = "functions\n  f(xs) {\n    match xs {\n      [] -> [],\n      [x, ..rest] -> [x, x, ..rest],\n    }\n  }\n";

    assert_eq!(format(src, "test.px").output.as_deref(), Some(src));
  }
}
