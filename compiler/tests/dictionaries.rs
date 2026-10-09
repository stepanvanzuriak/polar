mod common;

use common::node::run_program;
use polar_compiler::{
  CompileOptions, Stage, compile, dump_stage, dump_stage_with,
  shared::modules::ModuleSource,
};

const DESCRIBE: &str = "traits
  Describe<a> {
    describe(value: a) -> String
  }

types
  Status = Draft | Published(Int)
  List<a> = Nil | Cons(a, List<a>)
";

const IMPLS: &str = "impls
  Describe for Status {
    describe(value) {
      match value {
        Draft -> \"draft\",
        Published(year) -> String.concat(\"published in \", Int.to_string(year)),
      }
    }
  }

  Describe for List<a> where Describe<a> {
    describe(value) {
      match value {
        Nil -> \"\",
        Cons(head, Nil) -> describe(head),
        Cons(head, tail) -> String.concat(String.concat(describe(head), \", \"), describe(tail)),
      }
    }
  }
";

fn indent(src: &str) -> String {
  src.lines().flat_map(|l| ["  ", l, "\n"]).collect()
}

fn with_describe(fns: &str) -> String {
  let functions = if fns.is_empty() {
    String::new()
  } else {
    format!("functions\n{}\n", indent(fns))
  };

  format!("{DESCRIBE}\n{functions}{IMPLS}\nexports\n  main\n")
}

fn with_main(fns: &str) -> String {
  with_describe(fns)
}

fn dictionaries(src: &str) -> String {
  let result = dump_stage(src, "test.px", Stage::Dictionaries);

  assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
  result.output.unwrap()
}

fn run(src: &str) -> String {
  run_program(src, "test.px", &[]).unwrap_or_else(|e| panic!("{e}"))
}

#[test]
fn impl_becomes_record() {
  let out = dictionaries(&with_describe("main() {\n  1\n}\n"));

  assert!(out.contains("(const $Describe$Status/"), "{out}");
  assert!(out.contains("export (record (describe (lam"), "{out}");
}

#[test]
fn conditional_impl_is_function() {
  let out = dictionaries(&with_describe("main() {\n  1\n}\n"));

  assert!(out.contains("(fn $Describe$List/"), "{out}");
  assert!(out.contains(" export ($d0/"), "{out}");
}

#[test]
fn dict_params_first() {
  let out = dictionaries(&with_describe(
    "twice(x) {\n  String.concat(describe(x), describe(x))\n}\n\nmain() {\n  1\n}\n",
  ));

  assert!(out.contains("(fn twice/0 ($d0/"), "{out}");
  assert!(out.contains(" x/"), "{out}");
}

#[test]
fn call_passes_dict() {
  let out = dictionaries(&with_describe(
    "twice(x) {\n  describe(x)\n}\n\nmain() {\n  twice(Draft)\n}\n",
  ));

  assert!(
    out.contains("(app (var twice/0) (dict Describe Status) (ctor Draft))"),
    "{out}"
  );
}

#[test]
fn nested_dict() {
  let out = dictionaries(&with_describe(
    "twice(x) {\n  describe(x)\n}\n\nmain() {\n  twice(Cons(Draft, Nil))\n}\n",
  ));

  assert!(
    out.contains("(app (var twice/0) (dict Describe List (dict Describe Status)) (ctor Cons"),
    "{out}"
  );
}

#[test]
fn given_passed_through() {
  let out = dictionaries(&with_describe(
    "twice(x) {\n  describe(x)\n}\n\nf(x: a) -> String where Describe<a> {\n  twice(x)\n}\n\nmain() {\n  1\n}\n",
  ));
  let f = out.lines().find(|l| l.contains("(fn f/")).expect("f");
  let param = f.split("($d0/").nth(1).unwrap().split(' ').next().unwrap();

  assert!(
    f.contains(&format!("(app (var twice/0) (var $d0/{param}) (var x/")),
    "{out}"
  );
}

#[test]
fn recursive_group() {
  let out = dictionaries(&with_describe(
    "even(x, n) {\n  if n == 0 { describe(x) } else { odd(x, n - 1) }\n}\n\nodd(x, n) {\n  if n == 0 { describe(x) } else { even(x, n - 1) }\n}\n\nmain() {\n  even(Draft, 3)\n}\n",
  ));
  let even = out.lines().find(|l| l.contains("(fn even/")).expect("even");
  let d = even.split("($d0/").nth(1).unwrap().split(' ').next().unwrap();

  assert!(even.contains(&format!("(app (var odd/1) (var $d0/{d})")), "{out}");
}

#[test]
fn constrained_as_value() {
  let out = dictionaries(&with_describe(
    "twice(x) {\n  describe(x)\n}\n\napply(f, x) {\n  f(x)\n}\n\nmain() {\n  apply(twice, Draft)\n}\n",
  ));

  assert!(
    out.contains("(lam ($a0/")
      && out.contains("(app (var twice/0) (dict Describe Status) (var $a0/"),
    "{out}"
  );
}

#[test]
fn dict_before_constants() {
  let src = format!(
    "{DESCRIBE}\nconstants\n  label: String = describe(Draft)\n\nfunctions\n  main() {{\n    Log.info(label)\n  }}\n\n{IMPLS}\nexports\n  main\n"
  );

  assert_eq!(run(&src), "draft\n");
}

#[test]
fn runs() {
  let src = with_main(
    "twice(x) {\n  String.concat(describe(x), describe(x))\n}\n\nmain() {\n  Log.info(twice(Cons(Draft, Cons(Published(2026), Nil))))\n}\n",
  );

  assert_eq!(run(&src), "draft, published in 2026draft, published in 2026\n");
}

const SHAPES: &str = "module Shapes

traits
  Describe<a> {
    describe(value: a) -> String
  }

types
  Shape = Circle(Float) | Square(Float)

impls
  Describe for Shape {
    describe(value) {
      match value {
        Circle(r) -> \"a circle\",
        Square(w) -> \"a square\",
      }
    }
  }

exports
  Shape
  Describe { describe }
";

#[test]
fn imported_impl() {
  let modules = vec![ModuleSource {
    path: "Shapes".to_string(),
    source: SHAPES.to_string(),
    specifier: "./shapes.js".to_string(),
    plugins: Vec::new(),
  }];
  let src = "uses\n  Shapes { describe }\n\nfunctions\n  main() {\n    Log.info(describe(Circle(1.0)))\n  }\n\nexports\n  main\n";
  let out = compile(
    src,
    "main.px",
    &CompileOptions { modules: modules.clone(), ..CompileOptions::default() },
  );

  assert!(out.diagnostics.is_empty(), "{:#?}", out.diagnostics);
  assert!(out.js.contains("$m$Shapes.$Describe$Shape"), "{}", out.js);
  assert_eq!(run_program(src, "main.px", &modules).unwrap(), "a circle\n");
}

#[test]
fn imported_constrained_function() {
  let lib = "module Lib

traits
  Describe<a> {
    describe(value: a) -> String
  }

types
  Shape = Circle(Float)

functions
  loud(x) {
    String.uppercase(describe(x))
  }

impls
  Describe for Shape {
    describe(value) {
      \"circle\"
    }
  }

exports
  Shape
  Describe { describe }
  loud
";
  let modules = vec![ModuleSource {
    path: "Lib".to_string(),
    source: lib.to_string(),
    specifier: "./lib.js".to_string(),
    plugins: Vec::new(),
  }];
  let src = "uses\n  Lib\n\nfunctions\n  main() {\n    Log.info(Lib.loud(Circle(1.0)))\n  }\n\nexports\n  main\n";

  assert_eq!(run_program(src, "main.px", &modules).unwrap(), "CIRCLE\n");
}

#[test]
fn no_method_left() {
  let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");

  for name in ["hello.px", "blog.px", "effects.px"] {
    let src = std::fs::read_to_string(
      root.join("compiler/tests/fixtures/programs").join(name),
    )
    .unwrap();
    let out = dump_stage_with(
      &src,
      name,
      Stage::Dictionaries,
      &CompileOptions::default(),
    );

    if let Some(out) = out.output {
      assert!(!out.contains("(method "), "{name}: {out}");
    }
  }
}
