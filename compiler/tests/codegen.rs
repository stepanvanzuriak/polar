mod common;

use std::path::{Path, PathBuf};

use common::node::{TempDir, node_check_module, run_main, runtime_url};
use polar_compiler::{
  CompileOptions, backend::js, compile, core,
  shared::diagnostic::DiagnosticBag, shared::modules::ModuleSource,
  shared::source::SourceFile, syntax::lexer::lex, syntax::parser::parse,
};

fn repo() -> PathBuf {
  PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn compile_ok(src: &str, filename: &str) -> String {
  let out = compile(
    src,
    filename,
    &CompileOptions { runtime: runtime_url(), ..CompileOptions::default() },
  );

  assert!(
    out.diagnostics.is_empty(),
    "{filename} must compile: {:?}",
    out.diagnostics
  );
  out.js
}

fn run(src: &str, filename: &str) -> String {
  common::node::run_program(src, filename, &[])
    .unwrap_or_else(|err| panic!("{err}"))
}

fn unchecked(src: &str, filename: &str) -> String {
  let file = SourceFile::new(filename, src);
  let mut bag = DiagnosticBag::default();
  let module = parse(&file, &lex(&file, &mut bag), &mut bag);
  let lowered = core::lower::lower_with(&module, &[], &mut bag);

  assert!(!bag.has_errors(), "{:?}", bag.into_sorted());

  let options =
    CompileOptions { runtime: runtime_url(), ..CompileOptions::default() };
  let core = core::matching::compile_matches(lowered.core);
  let program =
    polar_compiler::backend::codegen::emit::emit(&core, &file, &options);

  js::print::print_program(&program).code
}

fn rejected_with(src: &str, filename: &str, code: &str) {
  let out = compile(src, filename, &CompileOptions::default());

  assert!(
    out.diagnostics.iter().any(|d| d.code.to_string() == code),
    "{:?}",
    out.diagnostics
  );
}

fn expect_throw(js: &str) -> String {
  match run_main(js) {
    Ok(stdout) => panic!("expected a throw, got stdout {stdout:?}\n{js}"),
    Err(stderr) => stderr,
  }
}

fn per_host(path: &Path) -> Vec<(String, PathBuf)> {
  let stem = path.file_stem().unwrap().to_string_lossy().into_owned();
  let dir = path.parent().unwrap();
  let mut found: Vec<(String, PathBuf)> = std::fs::read_dir(dir)
    .unwrap()
    .filter_map(|entry| {
      let other = entry.ok()?.path();
      let name = other.file_name()?.to_str()?.to_string();
      let host = name
        .strip_prefix(&format!("{stem}."))?
        .strip_suffix(".expected.txt")?
        .to_string();

      Some((host, other))
    })
    .collect();

  found.sort();
  found
}

fn helpers(path: &Path) -> Vec<(String, String)> {
  let stem = path.file_stem().unwrap().to_string_lossy().into_owned();

  std::fs::read_dir(path.parent().unwrap())
    .unwrap()
    .filter_map(|entry| {
      let other = entry.ok()?.path();
      let name = other.file_name()?.to_str()?.to_string();

      (name.starts_with(&format!("{stem}."))
        && other.extension().is_some_and(|e| e.eq_ignore_ascii_case("js")))
      .then(|| (name, std::fs::read_to_string(&other).unwrap()))
    })
    .collect()
}

fn assert_runs_as_expected(path: &Path) {
  let src = std::fs::read_to_string(path).expect("read the .px file");
  let expected_path = path.with_extension("expected.txt");
  let name = path.file_name().unwrap().to_string_lossy();

  if let Ok(expected) = std::fs::read_to_string(&expected_path) {
    assert_eq!(run(&src, &name), expected, "{}", path.display());
    return;
  }

  let hosts = per_host(path);
  let files = helpers(path);
  let files: Vec<(&str, &str)> =
    files.iter().map(|(n, t)| (n.as_str(), t.as_str())).collect();

  assert!(!hosts.is_empty(), "missing {}", expected_path.display());

  for (host, expected) in hosts {
    let expected = std::fs::read_to_string(expected).unwrap();
    let ran =
      common::node::run_program_with(&src, &name, &[], Some(&host), &files)
        .unwrap_or_else(|err| panic!("{err}"));

    assert_eq!(ran, expected, "{} on {host}", path.display());
  }
}

fn example(name: &str) -> String {
  std::fs::read_to_string(
    repo().join("compiler/tests/fixtures/programs").join(name),
  )
  .expect("read the example")
}

fn codegen_fixtures() -> Vec<PathBuf> {
  let dir =
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/codegen");
  let mut paths: Vec<_> = std::fs::read_dir(dir)
    .expect("tests/fixtures/codegen exists")
    .map(|entry| entry.expect("a directory entry").path())
    .filter(|path| path.extension().is_some_and(|ext| ext == "px"))
    .collect();

  paths.sort();
  assert!(!paths.is_empty(), "no codegen fixtures found");
  paths
}

fn lines(src: &str) -> Vec<String> {
  run(src, "test.px").lines().map(str::to_string).collect()
}

#[test]
fn hello_runs() {
  assert_eq!(run(&example("hello.px"), "hello.px"), "hello, world\n");
  assert_runs_as_expected(
    &repo().join("compiler/tests/fixtures/programs/hello.px"),
  );
}

#[test]
fn blog_runs() {
  assert_runs_as_expected(
    &repo().join("compiler/tests/fixtures/programs/blog.px"),
  );
}

#[test]
fn effects_runs() {
  assert_runs_as_expected(
    &repo().join("compiler/tests/fixtures/programs/effects.px"),
  );
}

#[test]
fn row_polymorphic_access() {
  let out = lines(
    "\
functions
  title_of(r: { title: String | rest }) -> String {
    r.title
  }

  main() {
    Log.info(title_of({ id: 1, title: \"Post\", body: \"b\" }))
    Log.info(title_of({ title: \"About\" }))
  }

exports
  main
",
  );

  assert_eq!(out, ["Post", "About"]);
}

#[test]
fn recursion_over_a_list() {
  let out = lines(
    "\
types
  List<a> = Nil | Cons(a, List<a>)

functions
  map(xs, f) {
    match xs {
      Nil -> Nil,
      Cons(head, tail) -> Cons(f(head), map(tail, f)),
    }
  }

  each(xs) {
    match xs {
      Nil -> {},
      Cons(head, tail) -> {
        Log.info(head)
        each(tail)
      },
    }
  }

  main() {
    each(map(Cons(\"a\", Cons(\"b\", Cons(\"c\", Nil))), function(s) { \"#{s}!\" }))
  }

exports
  main
",
  );

  assert_eq!(out, ["a!", "b!", "c!"]);
}

#[test]
fn slug_pipeline() {
  let out = lines(
    "\
functions
  slug(r: { title: String | rest }) -> String {
    r.title |> String.lowercase |> String.replace(\" \", \"-\")
  }

  main() {
    Log.info(slug({ title: \"Hello There World\" }))
  }

exports
  main
",
  );

  assert_eq!(out, ["hello-there-world"]);
}

#[test]
fn interpolation_output() {
  let out = lines(
    "\
functions
  main() {
    let user = { name: \"Ada\", age: 36 }

    Log.info(\"#{user.name} is #{user.age}\")
  }

exports
  main
",
  );

  assert_eq!(out, ["Ada is 36"]);
}

#[test]
fn every_fixture_executes() {
  for path in codegen_fixtures() {
    let src = std::fs::read_to_string(&path).expect("read the .px file");
    let js = compile_ok(&src, &path.file_name().unwrap().to_string_lossy());

    std::fs::write(path.with_extension("js"), js).expect("write the .js");
    assert_runs_as_expected(&path);
  }
}

#[test]
fn recursion_depth_5000() {
  let src = std::fs::read_to_string(
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
      .join("tests/fixtures/codegen/countdown.px"),
  )
  .unwrap();

  assert_eq!(run(&src, "countdown.px"), "5000\n");
}

#[test]
fn no_temp_directories_remain() {
  let js = compile_ok(&example("hello.px"), "hello.px");
  let dir = TempDir::new();
  let path = dir.path.clone();

  common::node::run_main_in(&dir, &js).expect("hello runs");
  drop(dir);
  assert!(!path.exists(), "{} was left behind", path.display());

  let dir = TempDir::new();
  let path = dir.path.clone();

  assert!(
    common::node::run_main_in(&dir, "export function main() { throw 1 }")
      .is_err()
  );
  drop(dir);
  assert!(!path.exists(), "{} was left behind", path.display());
}

#[test]
fn every_snapshot_is_valid_js() {
  for path in codegen_fixtures() {
    let src = std::fs::read_to_string(&path).unwrap();
    let js = compile_ok(&src, &path.file_name().unwrap().to_string_lossy());

    node_check_module(&js)
      .unwrap_or_else(|err| panic!("{}\n{js}\n{err}", path.display()));
  }
}

#[test]
fn reserved_names_survive() {
  let out = lines(
    "\
types
  Wrapper = Map(Int)

functions
  main() {
    let class = Map(7)

    match class {
      Map(n) -> Log.info(\"#{n}\"),
    }
  }

exports
  main
",
  );

  assert_eq!(out, ["7"]);
}

#[test]
fn patterns_execute() {
  assert_runs_as_expected(
    &PathBuf::from(env!("CARGO_MANIFEST_DIR"))
      .join("tests/fixtures/codegen/patterns.px"),
  );
}

const SHAPES: &str = "\
types
  Shape = Circle(Float) | Square(Float)

functions
  radius(s) {
    let Circle(r) = s
    r
  }

";

#[test]
fn refutable_let_executes() {
  let ok = format!(
    "{SHAPES}  main() {{\n    Log.info(\"#{{radius(Circle(2.5))}}\")\n  }}\n\nexports\n  main\n"
  );

  assert_eq!(run_main(&unchecked(&ok, "shapes.px")).unwrap(), "2.5\n");

  let bad = format!(
    "{SHAPES}  main() {{\n    Log.info(\"#{{radius(Square(1.0))}}\")\n  }}\n\nexports\n  main\n"
  );
  let stderr = expect_throw(&unchecked(&bad, "shapes.px"));

  assert!(stderr.contains("PolarMatchError"), "{stderr}");
  rejected_with(&bad, "shapes.px", "POLAR0602");
}

#[test]
fn non_exhaustive_names_the_location() {
  let src = "\
functions
  pick(a, b) { b }

  main() {
    Log.info(pick(\"é\", match 1 {
      2 -> \"two\",
    }))
  }

exports
  main
";
  let stderr = expect_throw(&unchecked(src, "loc.px"));

  assert!(stderr.contains("no pattern matched (loc.px:5:24)"), "{stderr}");
  rejected_with(src, "loc.px", "POLAR0601");
}

#[test]
fn list_literals_use_the_runtime_helper() {
  let js = compile_ok(
    "uses\n  Std.List\n\nfunctions\n  f(xs) {\n    [[1, 2], [1, 2, ..xs], [1, ..xs], []]\n  }\n",
    "list.px",
  );

  assert!(js.contains("$rt.list([1, 2])"), "{js}");
  assert!(js.contains("$rt.list([1, 2], xs)"), "{js}");
  assert!(js.contains("{ $: \"Cons\", _0: 1, _1: xs }"), "{js}");
  assert!(js.contains("{ $: \"Nil\" }"), "{js}");
}

mod typed {
  use super::*;

  fn function_js(src: &str) -> String {
    compile_ok(&format!("functions\n  {src}\n"), "typed.px")
  }

  fn prints(expr: &str) -> String {
    run(
      &format!(
        "functions\n  main() {{\n    Log.info(\"#{{{expr}}}\")\n  }}\n\nexports\n  main\n"
      ),
      "typed.px",
    )
  }

  #[test]
  fn eq_int_is_strict() {
    let js = function_js("f(a: Int, b: Int) { a == b }");

    assert!(js.contains("a === b"), "{js}");
    assert!(!js.contains("$rt.eq"), "{js}");
  }

  #[test]
  fn ne_string_is_strict() {
    let js = function_js("f(a: String, b: String) { a != b }");

    assert!(js.contains("a !== b"), "{js}");
    assert!(!js.contains("$rt.eq"), "{js}");
  }

  #[test]
  fn eq_record_calls_the_derived_dictionary() {
    let js = compile_ok(
      "types\n  R = { x: Int } derive(Eq)\n\nfunctions\n  f(a: R, b: R) { a == b }\n",
      "typed.px",
    );

    assert!(js.contains("$Eq$R.eq(a, b)"), "{js}");
    assert!(!js.contains("$rt.eq"), "{js}");
  }

  #[test]
  fn eq_on_an_anonymous_record_is_rejected() {
    rejected_with(
      "functions\n  f(a: { x: Int }, b: { x: Int }) { a == b }\n",
      "typed.px",
      "POLAR0707",
    );
  }

  #[test]
  fn eq_generic_calls_the_eq_dictionary() {
    let js = function_js("f(a, b) { a == b }");

    assert!(js.contains("$d0.eq(a, b)"), "{js}");
    assert!(!js.contains("$rt.eq"), "{js}");
  }

  #[test]
  fn int_division_truncates() {
    assert_eq!(prints("7 / 2"), "3\n");
    assert!(
      function_js("f(a: Int, b: Int) { a / b }").contains("Math.trunc(a / b)")
    );
  }

  #[test]
  fn negative_int_division() {
    assert_eq!(prints("-7 / 2"), "-3\n");
  }

  #[test]
  fn float_division_unchanged() {
    assert_eq!(prints("7.0 / 2.0"), "3.5\n");
    assert!(
      !function_js("f(a: Float, b: Float) { a / b }").contains("Math.trunc")
    );
  }

  #[test]
  fn examples_still_run() {
    for name in ["blog.px", "effects.px", "hello.px"] {
      assert_runs_as_expected(
        &repo().join("compiler/tests/fixtures/programs").join(name),
      );
    }

    for fixture in codegen_fixtures() {
      assert_runs_as_expected(&fixture);
    }
  }
}

mod dictionaries {
  use super::*;
  use polar_compiler::{backend::js::ast::Item, check, syntax::expand};

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
      \"status\"
    }
  }

  Describe for List<a> where Describe<a> {
    describe(value) {
      match value {
        Nil -> \"\",
        Cons(head, tail) -> String.concat(describe(head), describe(tail)),
      }
    }
  }
";

  fn with_describe(fns: &str) -> String {
    let fns: String = fns.lines().flat_map(|l| ["  ", l, "\n"]).collect();

    format!("{DESCRIBE}\nfunctions\n{fns}\n{IMPLS}")
  }

  #[test]
  fn interpolation_int_direct() {
    let js =
      compile_ok("functions\n  f(n: Int) { \"n = #{n}\" }\n", "typed.px");

    assert!(js.contains("`n = ${n}`"), "{js}");
    assert!(!js.contains("Show"), "{js}");
  }

  #[test]
  fn interpolation_bool_and_float_direct() {
    let js = compile_ok(
      "functions\n  f(b: Bool, x: Float) { \"#{b} #{x}\" }\n",
      "typed.px",
    );

    assert!(js.contains("`${b} ${x}`"), "{js}");
  }

  #[test]
  fn constant_dict_hoisted() {
    let js = compile_ok(
      &with_describe(
        "f() {\n  String.concat(describe(Cons(Draft, Nil)), describe(Cons(Draft, Nil)))\n}\n",
      ),
      "typed.px",
    );

    assert_eq!(
      js.matches("const $dict0 = $Describe$List($Describe$Status);").count(),
      1,
      "{js}"
    );
    assert!(!js.contains("$dict1"), "{js}");
    assert_eq!(js.matches("$dict0.describe").count(), 2, "{js}");
  }

  #[test]
  fn param_dict_not_hoisted() {
    let js = compile_ok(
      &with_describe(
        "f(x: a) -> String where Describe<a> {\n  describe(Cons(x, Nil))\n}\n",
      ),
      "typed.px",
    );

    assert!(js.contains("$Describe$List($d0)"), "{js}");
    assert!(!js.contains("$dict0"), "{js}");
  }

  #[test]
  fn dictionaries_come_first_and_are_exported() {
    let js = compile_ok(&with_describe("f() {\n  1\n}\n"), "typed.px");
    let dict = js.find("export const $Describe$Status").expect("dictionary");
    let function = js.find("function f()").expect("f");

    assert!(dict < function, "{js}");
    assert!(js.contains("export function $Describe$List($d0)"), "{js}");
  }

  #[test]
  fn dict_source_map() {
    let src = with_describe("f() {\n  1\n}\n");
    let file = SourceFile::new("typed.px", src.as_str());
    let mut bag = DiagnosticBag::default();
    let mut module = parse(&file, &lex(&file, &mut bag), &mut bag);
    let expansion = expand::expand(&mut module, &file, &[], &mut bag);
    let lowered =
      core::lower::lower_expanded(&module, &[], &expansion, &mut bag);
    let types = check::check(&module, &lowered, &mut bag);

    assert!(!bag.has_errors(), "{:#?}", bag.into_sorted());

    let annotated = core::annotate::annotate(lowered.core, &types);
    let core = core::dictionaries::elaborate(annotated, &types, &[]);
    let core = core::matching::compile_matches(core);
    let program = polar_compiler::backend::codegen::emit::emit(
      &core,
      &file,
      &CompileOptions::default(),
    );
    let origin = program
      .items
      .iter()
      .find_map(|item| match item {
        Item::Stmt(js::ast::Stmt::Var(v))
          if v.name.name == "$Describe$Status" =>
        {
          v.origin.clone()
        }
        _ => None,
      })
      .expect("the dictionary");

    assert_eq!(file.slice(&origin), "Describe for Status");
  }

  #[test]
  fn runtime_eq_gone() {
    let runtime =
      std::fs::read_to_string(repo().join("runtime/runtime.js")).unwrap();

    assert!(!runtime.contains("export function eq"));
  }

  #[test]
  fn std_and_projects_still_compile() {
    for m in polar_compiler::stdlib::MODULES {
      let out = compile(
        m.source,
        &polar_compiler::stdlib::filename(m.name),
        &CompileOptions {
          runtime: "../runtime.js".to_string(),
          ..CompileOptions::default()
        },
      );

      assert!(out.diagnostics.is_empty(), "{}: {:#?}", m.name, out.diagnostics);
    }
  }
}

mod colours {
  use super::*;

  const COLOURS: &str = "\
uses
  Std.List

hosts
  Node

types
  Hide = Hide(function(Int) -> Int)

effects
  Clock in Node {
    now() -> Int
  }

binds
  Clock in Node {
    now() {
      7
    }
  }

functions
  apply(f, x) {
    f(x)
  }

  twice(f: function(Int) -> Int / {| e}, x: Int) -> Int / {| e} {
    f(f(x))
  }

  caller(f: function() -> Int / {| e}) -> function() -> Int / {| e} {
    function() { f() + 1 }
  }

  reveal(h, x) {
    match h {
      Hide(f) -> f(x),
    }
  }

  main() {
    let id = function(x) { x }
    Log.info(\"#{apply(id, 1)} #{apply(function(x) { x + Clock.now() }, 1)}\")
    Log.info(\"#{twice(function(x) { x * 2 }, 3)} #{twice(function(x) { x + Clock.now() }, 3)}\")
    let still = caller(function() { 1 })
    let timed = caller(function() { Clock.now() })
    Log.info(\"#{still()} #{timed()}\")
    Log.info(\"#{reveal(Hide(function(x) { x }), 4)}\")
    Log.info(\"#{List.fold([1, 2, 3], 0, function(a, b) { a + b })}\")
  }

exports
  main
";

  #[test]
  fn effect_polymorphic_functions_run_in_both_colours() {
    assert_eq!(lines(COLOURS), ["1 8", "12 17", "2 8", "4", "6"]);
  }

  #[test]
  fn polymorphic_functions_get_two_versions_and_a_dispatcher() {
    let js = compile_ok(
      &COLOURS.replace("exports\n  main\n", "exports\n  main\n  apply\n"),
      "colours.px",
    );

    assert!(js.contains("function apply$sync(f, x)"), "{js}");
    assert!(js.contains("async function apply$async(f, x)"), "{js}");
    assert!(
      js.contains(
        "return $a === $rt.ASYNC ? apply$async(f, x) : apply$sync(f, x);"
      ),
      "{js}"
    );
    assert!(
      js.contains(
        "$rt.poly(0, () => f() + 1, async () => await f($rt.ASYNC) + 1)"
      ),
      "{js}"
    );
  }

  #[test]
  fn pure_program_with_std_higher_order_functions_never_awaits() {
    let src = "uses\n  Std.List\n\nfunctions\n  main() {\n    let xs = List.map([1, 2, 3], function(x) { x * 2 })\n    Log.info(\"#{List.fold(xs, 0, function(a, b) { a + b })}\")\n  }\n\nexports\n  main\n";
    let js = compile_ok(src, "pure.px");

    assert!(!js.contains("await"), "{js}");
    assert!(!js.contains("async"), "{js}");
    assert!(
      js.contains("$List.map($rt.list([1, 2, 3]), (x) => x * 2)"),
      "{js}"
    );
    assert_eq!(lines(src), ["12"]);
  }

  #[test]
  fn unused_private_versions_are_dropped() {
    let src = "functions\n  total(n, f) {\n    if n == 0 { 0 } else { f(n) + total(n - 1, f) }\n  }\n\n  main() {\n    Log.info(\"#{total(3, function(x) { x + 1 })}\")\n  }\n\nexports\n  main\n";
    let js = compile_ok(src, "prune.px");

    assert!(js.contains("function total$sync"), "{js}");
    assert!(!js.contains("total$async"), "{js}");
    assert!(!js.contains("function total("), "{js}");
    assert_eq!(lines(src), ["9"]);
  }

  #[test]
  fn exported_polymorphic_functions_keep_every_version() {
    let src = "functions\n  apply(f, x) {\n    f(x)\n  }\n\nexports\n  apply\n";
    let js = compile_ok(src, "keep.px");

    assert!(js.contains("export function apply$sync"), "{js}");
    assert!(js.contains("export async function apply$async"), "{js}");
    assert!(js.contains("export function apply(f, x, $a)"), "{js}");
  }
}

mod inlining {
  use super::*;

  const HEADER: &str = "\
uses
  Std.Option

hosts
  Node

effects
  Db in Node {
    find(id: Int) -> String
  }

binds
  Db in Node {
    find(id) {
      \"row #{id}\"
    }
  }

functions
  local_map(option: Option<a>, f: function(a) -> b / {| e}) -> Option<b> / {| e} {
    match option {
      None -> None,
      Some(x) -> Some(f(x)),
    }
  }

  local_and_then(option: Option<a>, f: function(a) -> Option<b> / {| e}) -> Option<b> / {| e} {
    match option {
      None -> None,
      Some(x) -> f(x),
    }
  }
";

  fn program(fns: &str, main: &str) -> String {
    format!(
      "{HEADER}\n{fns}\n  main() {{\n    {main}\n  }}\n\nexports\n  main\n"
    )
  }

  fn function<'a>(js: &'a str, name: &str) -> &'a str {
    let start = js
      .find(&format!("function {name}("))
      .unwrap_or_else(|| panic!("no function {name} in\n{js}"));
    let rest = &js[start..];
    let end = rest.find("\n}\n").map_or(rest.len(), |i| i + 3);

    &rest[..end]
  }

  #[test]
  fn small_poly_call_is_inlined() {
    let src = program(
      "",
      "let a = local_map(Some(1), function(x) { x + 1 })\n    \
       let b = local_map(None, function(x) { x + 1 })\n    \
       Log.info(\"#{Option.with_default(a, 0)} #{Option.with_default(b, 0)}\")",
    );
    let js = compile_ok(&src, "inline.px");

    assert!(!js.contains("local_map"), "{js}");
    assert!(!js.contains("$rt.ASYNC"), "{js}");
    assert_eq!(lines(&src), ["2 0"]);
  }

  #[test]
  fn inlined_twice_in_one_caller() {
    let src = program(
      "  both(n: Int) -> Int {\n    \
         let a = local_map(Some(n), function(x) { x + 1 })\n    \
         let b = local_map(Some(n), function(x) { x * 10 })\n    \
         Option.with_default(a, 0) + Option.with_default(b, 0)\n  }\n",
      "Log.info(\"#{both(3)}\")",
    );
    let js = compile_ok(&src, "twice.px");

    assert!(!js.contains("local_map"), "{js}");
    assert_eq!(lines(&src), ["34"]);
  }

  #[test]
  fn inlined_with_async_lambda() {
    let src = program(
      "  load(n: Option<Int>) -> Option<String> / {Db} {\n    \
         local_map(n, function(id) { Db.find(id) })\n  }\n",
      "Log.info(Option.with_default(load(Some(4)), \"none\"))",
    );
    let js = compile_ok(&src, "load.px");
    let load = function(&js, "load");

    assert!(load.starts_with("function load(n) {"), "{js}");
    assert!(
      js.contains("export async function load")
        || js.contains("async function load"),
      "{js}"
    );
    assert!(load.contains("await $bind$Db.find("), "{js}");
    assert!(!js.contains("$rt.ASYNC"), "{js}");
    assert_eq!(lines(&src), ["row 4"]);
  }

  #[test]
  fn argument_order_is_kept() {
    let src = program(
      "  first() -> Option<Int> {\n    Log.info(\"first\")\n    Some(1)\n  }\n\n  \
       second() -> function(Int) -> Int {\n    Log.info(\"second\")\n    function(x) { x + 1 }\n  }\n",
      "Log.info(\"#{Option.with_default(local_map(first(), second()), 0)}\")",
    );
    let js = compile_ok(&src, "order.px");

    assert!(!js.contains("local_map"), "{js}");
    assert_eq!(lines(&src), ["first", "second", "2"]);
  }

  #[test]
  fn escaping_use_keeps_dispatcher() {
    let src = program(
      "",
      "let g = local_map\n    \
       Log.info(\"#{Option.with_default(g(Some(1), function(x) { x + 1 }), 0)}\")",
    );
    let js = compile_ok(&src, "escape.px");

    assert!(js.contains("function local_map("), "{js}");
    assert_eq!(lines(&src), ["2"]);
  }

  #[test]
  fn recursive_poly_not_inlined() {
    let src = program(
      "  total(n: Int, f: function(Int) -> Int / {| e}) -> Int / {| e} {\n    \
         if n == 0 { 0 } else { f(n) + total(n - 1, f) }\n  }\n",
      "Log.info(\"#{total(3, function(x) { x * 2 })}\")",
    );
    let js = compile_ok(&src, "recursive.px");

    assert!(js.contains("total$sync(3, "), "{js}");
    assert_eq!(lines(&src), ["12"]);
  }

  #[test]
  fn large_poly_not_inlined() {
    let src = program(
      "  spread(f: function(Int) -> Int / {| e}, x: Int) -> Int / {| e} {\n    \
         f(x) + f(x + 1) + f(x + 2) + f(x + 3) + f(x + 4) + f(x + 5) + f(x + 6) + f(x + 7) + f(x + 8)\n  }\n",
      "Log.info(\"#{spread(function(x) { x }, 0)}\")",
    );
    let js = compile_ok(&src, "large.px");

    assert!(js.contains("spread$sync("), "{js}");
    assert_eq!(lines(&src), ["36"]);
  }

  #[test]
  fn tail_call_through_inlined_body() {
    let src = program(
      "  go(n: Int) -> Option<Int> {\n    \
         local_and_then(Some(n - 1), function(m) { if m <= 0 { Some(m) } else { go(m) } })\n  }\n",
      "Log.info(\"#{Option.with_default(go(100000), 1)}\")",
    );
    let js = compile_ok(&src, "tail.px");

    assert!(function(&js, "go").contains("while (true)"), "{js}");
    assert_eq!(lines(&src), ["0"]);
  }

  #[test]
  fn lambda_argument_is_pasted() {
    let src = program(
      "  inc(n: Option<Int>) -> Option<Int> {\n    local_map(n, function(x) { x + 1 })\n  }\n",
      "Log.info(\"#{Option.with_default(inc(Some(1)), 0)}\")",
    );
    let js = compile_ok(&src, "paste.px");

    assert!(!function(&js, "inc").contains("=>"), "{js}");
    assert_eq!(lines(&src), ["2"]);
  }

  #[test]
  fn async_lambda_is_pasted_and_awaited() {
    let src = program(
      "  load(n: Option<Int>) -> Option<String> / {Db} {\n    \
         local_map(n, function(id) { Db.find(id) })\n  }\n",
      "Log.info(Option.with_default(load(None), \"none\"))",
    );
    let js = compile_ok(&src, "load.px");
    let load = function(&js, "load");

    assert!(
      load.contains("return { $: \"Some\", _0: await $bind$Db.find(x) };"),
      "{js}"
    );
    assert!(!load.contains("async ("), "{js}");
    assert_eq!(lines(&src), ["none"]);
  }

  #[test]
  fn lambda_used_twice_keeps_closure() {
    let src = program(
      "  both(option: Option<a>, f: function(a) -> a / {| e}) -> Option<a> / {| e} {\n    \
         match option {\n      None -> None,\n      Some(x) -> Some(f(f(x))),\n    }\n  }\n",
      "Log.info(\"#{Option.with_default(both(Some(1), function(x) { x * 3 }), 0)}\")",
    );
    let js = compile_ok(&src, "twice.px");

    assert!(!js.contains("both"), "{js}");
    assert!(js.contains("const f = (x) => x * 3;"), "{js}");
    assert_eq!(lines(&src), ["9"]);
  }

  #[test]
  fn lambda_passed_on_keeps_closure() {
    let src = program(
      "  forward(option: Option<a>, f: function(a) -> b / {| e}) -> Option<b> / {| e} {\n    \
         local_map(option, f)\n  }\n",
      "Log.info(\"#{Option.with_default(forward(Some(1), function(x) { x + 5 }), 0)}\")",
    );
    let js = compile_ok(&src, "forward.px");

    assert!(!js.contains("forward"), "{js}");
    assert!(js.contains("const f = (x) => x + 5;"), "{js}");
    assert_eq!(lines(&src), ["6"]);
  }

  #[test]
  fn lambda_captures_caller_local() {
    let src = program(
      "",
      "let k = 10\n    \
       Log.info(\"#{Option.with_default(local_map(Some(5), function(x) { x + k }), 0)}\")",
    );

    assert_eq!(lines(&src), ["15"]);
  }

  #[test]
  fn lambda_param_shadows_outer_name() {
    let src = program(
      "",
      "let x = 100\n    \
       let r = local_map(Some(7), function(x) { x * 2 })\n    \
       Log.info(\"#{x} #{Option.with_default(r, 0)}\")",
    );

    assert_eq!(lines(&src), ["100 14"]);
  }

  const HELPERS: &str = "module Helpers

functions
  apply(f: function(a) -> b / {| e}, x: a) -> b / {| e} {
    f(x)
  }

  twice(f: function(Int) -> Int / {| e}, x: Int) -> Int / {| e} {
    step(f, step(f, x))
  }

  step(f: function(Int) -> Int / {| e}, x: Int) -> Int / {| e} {
    let y = f(x)
    y
  }

exports
  apply
  twice
";

  fn helpers() -> Vec<ModuleSource> {
    vec![ModuleSource {
      path: "Helpers".to_string(),
      source: HELPERS.to_string(),
      specifier: "./helpers.js".to_string(),
      plugins: Vec::new(),
    }]
  }

  fn with_helpers(main: &str) -> (String, String) {
    let src = format!(
      "uses\n  Helpers\n\nfunctions\n  main() {{\n    {main}\n  }}\n\nexports\n  main\n"
    );
    let out = compile(
      &src,
      "main.px",
      &CompileOptions {
        runtime: runtime_url(),
        modules: helpers(),
        ..CompileOptions::default()
      },
    );

    assert!(out.diagnostics.is_empty(), "{:?}", out.diagnostics);

    let stdout = common::node::run_program(&src, "main.px", &helpers())
      .unwrap_or_else(|err| panic!("{err}"));

    (out.js, stdout)
  }

  #[test]
  fn std_option_map_is_inlined() {
    let src = program(
      "",
      "Log.info(\"#{Option.with_default(Option.map(Some(1), function(x) { x + 1 }), 0)}\")",
    );
    let js = compile_ok(&src, "std.px");

    assert!(!js.contains("$Option.map"), "{js}");
    assert!(!js.contains("$rt.ASYNC"), "{js}");
    assert_eq!(lines(&src), ["2"]);
  }

  #[test]
  fn std_option_map_inlined_async() {
    let src = program(
      "  load_std(n: Option<Int>) -> Option<String> / {Db} {\n    \
         Option.map(n, function(id) { Db.find(id) })\n  }\n",
      "Log.info(Option.with_default(load_std(Some(8)), \"none\"))",
    );
    let js = compile_ok(&src, "load_std.px");
    let load = function(&js, "load_std");

    assert!(load.contains("await $bind$Db.find(x)"), "{js}");
    assert!(!js.contains("$Option.map"), "{js}");
    assert!(!js.contains("$rt.ASYNC"), "{js}");
    assert_eq!(lines(&src), ["row 8"]);
  }

  #[test]
  fn std_and_then_inlined() {
    let src = program(
      "  positive(n: Option<Int>) -> Option<Int> {\n    \
         Option.and_then(n, function(x) { if x > 0 { Some(x) } else { None } })\n  }\n",
      "Log.info(\"#{Option.with_default(positive(Some(3)), 0)} #{Option.with_default(positive(Some(-3)), 0)} #{Option.with_default(positive(None), 9)}\")",
    );
    let js = compile_ok(&src, "and_then.px");

    assert!(!js.contains("$Option.and_then"), "{js}");
    assert_eq!(lines(&src), ["3 0 9"]);
  }

  #[test]
  fn std_list_map_not_inlined() {
    let src = format!(
      "uses\n  Std.List\n\n{}",
      program(
        "",
        "Log.info(\"#{List.fold(List.map([1, 2], function(x) { x * 2 }), 0, function(a, b) { a + b })}\")",
      )
      .replacen("uses\n", "", 1)
    );
    let js = compile_ok(&src, "list.px");

    assert!(js.contains("$List.map("), "{js}");
    assert_eq!(lines(&src), ["6"]);
  }

  #[test]
  fn std_map_as_value_not_inlined() {
    let src = program(
      "",
      "let g = Option.map\n    \
       Log.info(\"#{Option.with_default(g(Some(1), function(x) { x + 1 }), 0)}\")",
    );
    let js = compile_ok(&src, "value.px");

    assert!(js.contains("$Option.map"), "{js}");
    assert_eq!(lines(&src), ["2"]);
  }

  #[test]
  fn ctor_used_only_by_inlined_code() {
    let src = "uses\n  Std.List\n  Std.Option\n\nfunctions\n  main() {\n    \
      let found = List.find([1, 2, 3], function(x) { x > 1 })\n    \
      Log.info(\"#{Option.with_default(Option.map(found, function(x) { x * 10 }), 0)}\")\n  }\n\nexports\n  main\n";
    let js = compile_ok(src, "ctors.px");

    assert!(!js.contains("$Option.map"), "{js}");
    assert!(js.contains("\"None\""), "{js}");
    assert_eq!(lines(src), ["20"]);
  }

  #[test]
  fn user_module_poly_fn_inlined() {
    let (js, stdout) =
      with_helpers("Log.info(\"#{Helpers.apply(function(x) { x + 1 }, 41)}\")");

    assert!(!js.contains("$m$Helpers.apply("), "{js}");
    assert_eq!(stdout, "42\n");
  }

  #[test]
  fn private_helper_blocks_inlining() {
    let (js, stdout) =
      with_helpers("Log.info(\"#{Helpers.twice(function(x) { x * 3 }, 2)}\")");

    assert!(js.contains("$m$Helpers.twice("), "{js}");
    assert_eq!(stdout, "18\n");
  }
}

mod tail_calls {
  use super::*;

  fn fixture(name: &str) -> String {
    std::fs::read_to_string(
      PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/codegen")
        .join(name),
    )
    .unwrap()
  }

  fn program(fns: &str, main: &str) -> String {
    format!(
      "functions\n{fns}\n  main() {{\n    {main}\n  }}\n\nexports\n  main\n"
    )
  }

  #[test]
  fn recursion_depth_100000() {
    assert_eq!(
      run(&fixture("fold100k.px"), "fold100k.px"),
      "5000050000\n5000050000\n"
    );
  }

  #[test]
  fn self_tail_call_becomes_a_loop() {
    let js = compile_ok(&fixture("fold100k.px"), "fold100k.px");

    assert!(js.contains("function sum_to(n, acc) {\n  while (true) {"), "{js}");
    assert!(!js.contains("sum_to(n - 1"), "{js}");
    assert!(js.contains("continue;"), "{js}");
  }

  #[test]
  fn non_tail_recursion_still_recurses() {
    let js = compile_ok(&fixture("countdown.px"), "countdown.px");

    assert!(!js.contains("while"), "{js}");
    assert!(js.contains("1 + count(n - 1)"), "{js}");
  }

  #[test]
  fn later_argument_reads_an_earlier_parameter() {
    let src = program(
      "  swap(a, b, n) {\n    if n == 0 { \"#{a}#{b}\" } else { swap(b, a, n - 1) }\n  }\n",
      "Log.info(swap(\"x\", \"y\", 3))",
    );

    assert_eq!(lines(&src), ["yx"]);
  }

  #[test]
  fn closures_capture_each_iteration() {
    let src = "uses\n  Std.List\n\n".to_string()
      + &program(
        "  collect(n, acc) {\n    if n == 0 { acc } else { collect(n - 1, [function() { n }, ..acc]) }\n  }\n\n  \
       show(fs, out) {\n    match fs {\n      [] -> out,\n      [f, ..rest] -> show(rest, \"#{out}#{f()}\"),\n    }\n  }\n",
        "Log.info(show(collect(3, []), \"\"))",
      );

    assert_eq!(lines(&src), ["123"]);
  }

  #[test]
  fn call_inside_try_is_not_a_loop() {
    let src = format!(
      "types\n  Stop = Stop(Int)\n\n{}",
      program(
        "  down(n) -> Int / {Throws<Stop>} {\n    if n == 0 { throw Stop(0) } else { n }\n  }\n\n  \
         retry(n) -> Int {\n    try { down(n) } catch {\n      Stop(_) -> retry(n + 1),\n    }\n  }\n\n  \
         guarded(n) -> Int {\n    try { if n == 0 { down(n) } else { guarded(n - 1) } } catch {\n      Stop(_) -> -1,\n    }\n  }\n",
        "Log.info(\"#{retry(0)} #{guarded(3)}\")",
      )
    );
    let js = compile_ok(&src, "try.px");

    assert!(js.contains("function retry(n) {\n  while (true) {"), "{js}");
    assert!(!js.contains("function guarded(n) {\n  while"), "{js}");
    assert_eq!(lines(&src), ["1 -1"]);
  }

  #[test]
  fn mutual_recursion_is_unaffected() {
    let src = program(
      "  is_even(n) {\n    if n == 0 { true } else { is_odd(n - 1) }\n  }\n\n  \
       is_odd(n) {\n    if n == 0 { false } else { is_even(n - 1) }\n  }\n",
      "Log.info(\"#{is_even(10)}\")",
    );

    assert!(!compile_ok(&src, "mutual.px").contains("while"));
    assert_eq!(lines(&src), ["true"]);
  }

  #[test]
  fn call_from_a_lambda_is_not_a_self_tail_call() {
    let src = program(
      "  outer(n) {\n    let f = function(m) { outer(m) }\n    if n == 0 { 0 } else { f(n - 1) }\n  }\n",
      "Log.info(\"#{outer(3)}\")",
    );

    assert!(!compile_ok(&src, "lambda.px").contains("while"));
    assert_eq!(lines(&src), ["0"]);
  }

  #[test]
  fn shadowing_name_is_not_a_self_call() {
    let src = program(
      "  f(n) {\n    let f = function(x) { x + 1 }\n    f(n)\n  }\n",
      "Log.info(\"#{f(1)}\")",
    );

    assert!(!compile_ok(&src, "shadow.px").contains("while"));
    assert_eq!(lines(&src), ["2"]);
  }

  #[test]
  fn async_versions_loop_too() {
    let src = "hosts\n  Node\n\neffects\n  Clock in Node {\n    now() -> Int\n  }\n\nbinds\n  Clock in Node {\n    now() {\n      1\n    }\n  }\n\n\
functions\n  tick(n, acc) {\n    if n == 0 { acc } else { tick(n - 1, acc + Clock.now()) }\n  }\n\n  main() {\n    Log.info(\"#{tick(100000, 0)}\")\n  }\n\nexports\n  main\n";
    let js = compile_ok(src, "tick.px");

    assert!(
      js.contains("async function tick(n, acc) {\n  while (true) {"),
      "{js}"
    );
    assert_eq!(lines(src), ["100000"]);
  }
}
