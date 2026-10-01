use polar_compiler::{
  CompileOptions, Stage, compile, dump_stage_with, format_plugins,
  shared::codes::DiagnosticCode::{PluginError, TypeMismatch},
  shared::diagnostic::Diagnostic,
};
use std::{
  fs,
  path::{Path, PathBuf},
  sync::OnceLock,
};

fn framework() -> PathBuf {
  PathBuf::from(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../projects/simple_framework"
  ))
}

fn plugin() -> Vec<String> {
  static PATH: OnceLock<String> = OnceLock::new();

  let path = PATH.get_or_init(|| {
    polar_cli::plugin::prepare(
      &framework().canonicalize().unwrap(),
      Path::new("projects/simple_framework"),
      "simple_framework",
      &["schema".to_string(), "routes".to_string(), "views".to_string()],
      &std::collections::BTreeMap::new(),
      "polar.toml",
    )
    .unwrap_or_else(|e| panic!("{e:?}"))
  });

  vec![path.clone()]
}

fn options() -> CompileOptions {
  CompileOptions { plugins: plugin(), ..CompileOptions::default() }
}

fn diagnostics(src: &str) -> Vec<Diagnostic> {
  compile(src, "test.px", &options()).diagnostics
}

const HEADER: &str = "uses\n  Std.Http\n  Std.List\n\n";

const HANDLERS: &str = "functions
  ok(body: String) -> Response {
    { status: 200, headers: [], body: body }
  }

  index() -> Response {
    ok(\"index\")
  }

  show(id: Int) -> Response {
    ok(\"#{id}\")
  }

  named(name: String, request: Request) -> Response {
    ok(\"#{name} #{request.method}\")
  }
";

fn program(routes: &str) -> String {
  format!("{HEADER}routes\n{routes}\n{HANDLERS}")
}

#[track_caller]
fn only(src: &str, message: &str, at: &str) {
  let found = diagnostics(src);

  assert_eq!(found.len(), 1, "{found:#?}");
  assert_eq!(found[0].code, PluginError, "{found:#?}");
  assert!(found[0].message.contains(message), "{found:#?}");

  let start = src.find(at).unwrap_or_else(|| panic!("`{at}` not in source"));
  let span = &found[0].primary.span;

  assert!(
    span.start >= start && span.end <= start + at.len(),
    "should point at `{at}`: {found:#?}"
  );
}

#[test]
fn a_router_type_checks_and_is_exported() {
  let src =
    program("  GET  /posts      -> index\n  GET  /posts/:id  -> show\n");
  let out = dump_stage_with(&src, "test.px", Stage::Types, &options());
  let types = out.output.unwrap_or_else(|| panic!("{:#?}", out.diagnostics));

  assert!(types.contains("router : function({"), "{types}");
  assert!(types.contains("} as Request) -> Option<{"), "{types}");

  let js = compile(&src, "test.px", &options()).js;

  assert!(js.contains("export function router("), "{js}");
}

#[test]
fn the_router_row_is_the_handlers_rows() {
  let src = format!(
    "{HEADER}hosts\n  Node\n\neffects\n  Store {{\n    count() -> Int\n  }}\n\nbinds\n  Store in Node {{\n    count() {{\n      1\n    }}\n  }}\n\nroutes\n  GET  /count  -> count\n\nfunctions\n  count() -> Response / {{Store}} {{\n    {{ status: 200, headers: [], body: \"#{{Store.count()}}\" }}\n  }}\n"
  );
  let out = dump_stage_with(&src, "test.px", Stage::Types, &options());
  let types = out.output.unwrap_or_else(|| panic!("{:#?}", out.diagnostics));
  let router = types.lines().find(|l| l.starts_with("router :")).unwrap();

  assert!(router.ends_with("/ {Store}"), "{router}");
}

#[test]
fn a_handler_returning_the_wrong_type_is_blamed_on_its_route() {
  let src = format!(
    "{HEADER}routes\n  GET  /text  -> text\n\nfunctions\n  text() -> String {{\n    \"no\"\n  }}\n"
  );
  let found = diagnostics(&src);

  assert_eq!(found.len(), 1, "{found:#?}");
  assert_eq!(found[0].code, TypeMismatch, "{found:#?}");

  let entry = src.find("GET  /text  -> text").unwrap();

  assert!(
    found[0].primary.span.start >= entry
      && found[0].primary.span.end <= entry + "GET  /text  -> text".len(),
    "{found:#?}"
  );
}

#[test]
fn unknown_handler() {
  only(&program("  GET  /x  -> missing\n"), "no function `missing`", "missing");
}

#[test]
fn path_param_without_a_parameter() {
  only(
    &program("  GET  /posts/:slug  -> index\n"),
    "has no parameter `slug`",
    ":slug",
  );
}

#[test]
fn path_param_of_the_wrong_type() {
  let src = format!(
    "{HEADER}routes\n  GET  /posts/:id  -> show\n\nfunctions\n  show(id: Float) -> Response {{\n    {{ status: 200, headers: [], body: \"\" }}\n  }}\n"
  );

  only(&src, "must be `Int` or `String`", "GET  /posts/:id  -> show");
}

#[test]
fn a_parameter_outside_the_path_must_be_the_request() {
  only(
    &program("  GET  /posts  -> show\n"),
    "isn't in the path",
    "GET  /posts  -> show",
  );
}

#[test]
fn duplicate_routes() {
  only(
    &program("  GET  /posts/:id    -> show\n  GET  /posts/:name  -> named\n"),
    "already matches this path",
    "GET  /posts/:name  -> named",
  );
}

#[test]
fn the_same_path_with_another_method_is_fine() {
  let src =
    program("  GET     /posts/:id  -> show\n  DELETE  /posts/:id  -> show\n");

  assert!(diagnostics(&src).is_empty(), "{:#?}", diagnostics(&src));
}

#[test]
fn spaces_in_a_path() {
  only(&program("  GET  / posts  -> index\n"), "no spaces", "posts");
}

#[test]
fn an_unknown_method() {
  only(
    &program("  FETCH  /x  -> index\n"),
    "`FETCH` is not an HTTP method",
    "FETCH",
  );
}

#[test]
fn routes_need_std_http() {
  let src = format!("uses\n  Std.List\n\nroutes\n  GET  /  -> index\n\n{HANDLERS}")
    .replace("Response", "{ status: Int, headers: List<{ name: String, value: String }>, body: String }")
    .replace("request: Request", "request: Int");
  let found = diagnostics(&src);

  assert!(
    found.iter().any(|d| d.message.contains("needs `Std.Http`")),
    "{found:#?}"
  );
}

#[test]
fn formatter_aligns_routes() {
  let src =
    program("  GET /posts -> index\n  DELETE   /posts/:id ->   show // gone\n");
  let once =
    format_plugins(&src, "test.px", &plugin()).output.expect("formats");

  assert!(
    once.contains("routes\n  GET     /posts      -> index\n  DELETE  /posts/:id  -> show // gone\n"),
    "{once}"
  );
  assert_eq!(format_plugins(&once, "test.px", &plugin()).output.unwrap(), once);
}

#[test]
fn web_posts_mistakes_point_into_schema_and_routes() {
  let root = Path::new(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../projects/web_posts/src"
  ));
  let broken = |file: &str, from: &str, to: &str| {
    let src = fs::read_to_string(root.join(file)).unwrap().replace(from, to);
    let found = diagnostics(&src);

    assert_eq!(found.len(), 1, "{file}: {found:#?}");
    assert_eq!(
      &src[found[0].primary.span.start..found[0].primary.span.end],
      to.rsplit(' ').next().unwrap()
    );
  };

  broken("post.px", "references users", "references people");
  broken("routes.px", "-> remove", "-> destroy");
}
