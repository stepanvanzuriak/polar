use polar_compiler::{
  Stage, dump_stage, shared::diagnostic::Diagnostic, shared::source::SourceFile,
};

const PRELUDE: &str = "traits
  Same<a> {
    eq(x: a, y: a) -> Bool
  }

types
  List<a> = Nil | Cons(a, List<a>)

  Post = { id: Int, title: String }

  Draft = { id: Int, title: String }
";

const SAME_FOR_POST: &str = "impls
  Same for Post {
    eq(x, y) {
      x.id == y.id
    }
  }
";

fn with_post(fns: &str) -> String {
  let body: String = fns.lines().flat_map(|l| ["  ", l, "\n"]).collect();

  format!("{PRELUDE}\nfunctions\n{body}\n{SAME_FOR_POST}")
}

fn diagnostics_of(src: &str) -> Vec<Diagnostic> {
  dump_stage(src, "test.px", Stage::Types).diagnostics
}

fn errors_of(src: &str) -> Vec<String> {
  let file = SourceFile::new("test.px", src);

  diagnostics_of(src)
    .iter()
    .map(|d| format!("{} at {:?}", d.code, file.slice(&d.primary.span)))
    .collect()
}

fn no_errors(src: &str) {
  assert_eq!(
    errors_of(src),
    Vec::<String>::new(),
    "{:#?}",
    diagnostics_of(src)
  );
}

fn types_of(src: &str) -> String {
  let result = dump_stage(src, "test.px", Stage::Types);

  assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
  result.output.unwrap()
}

fn type_of(src: &str, name: &str) -> String {
  let prefix = format!("{name} : ");

  types_of(src)
    .lines()
    .find_map(|l| l.strip_prefix(&prefix))
    .expect("no such declaration")
    .to_string()
}

#[test]
fn branded_has_impl() {
  no_errors(&with_post("f(p: Post, q: Post) -> Bool {\n  eq(p, q)\n}\n"));
}

#[test]
fn same_shape_other_alias() {
  let src = with_post("g(d: Draft, e: Draft) -> Bool {\n  eq(d, e)\n}\n");
  let diagnostics = diagnostics_of(&src);

  assert_eq!(errors_of(&src), ["POLAR0707 at \"eq\""]);
  assert!(diagnostics[0].message.contains("`Draft`"), "{diagnostics:#?}");
}

#[test]
fn anonymous_has_none() {
  let src = with_post("h() -> Bool {\n  eq({ a: 1 }, { a: 1 })\n}\n");

  assert_eq!(errors_of(&src), ["POLAR0707 at \"eq\""]);
}

#[test]
fn brands_differ() {
  let src = with_post("k(p: Post, d: Draft) -> Bool {\n  eq(p, d)\n}\n");
  let diagnostics = diagnostics_of(&src);

  assert_eq!(errors_of(&src), ["POLAR0501 at \"d\""]);

  let label = diagnostics[0].primary.message.clone().unwrap_or_default();

  assert!(label.contains("expected `Post`, found `Draft`"), "{diagnostics:#?}");
}

#[test]
fn literal_adopts_brand() {
  no_errors(&with_post(
    "save(p: Post) -> Bool {\n  eq(p, p)\n}\n\nf() {\n  save({ id: 1, title: \"x\" })\n}\n",
  ));
}

#[test]
fn order_independent() {
  no_errors(&with_post(
    "f(p: Post) -> Bool {\n  match [{ id: 1, title: \"x\" }, p] {\n    [first, .._] -> eq(first, p),\n    _ -> false,\n  }\n}\n",
  ));
}

#[test]
fn rows_keep_brand() {
  no_errors(&with_post(
    "keep(r: { title: String | rest }) -> { title: String | rest } {\n  r\n}\n\nf(p: Post) -> Bool {\n  eq(keep(p), p)\n}\n",
  ));
}

#[test]
fn update_keeps_brand() {
  no_errors(&with_post(
    "f(p: Post) -> Bool {\n  eq({ ..p, title: \"y\" }, p)\n}\n",
  ));
}

#[test]
fn update_drops_brand() {
  let src = with_post(
    "f(p: Post) -> Bool {\n  eq({ ..p, extra: 1 }, { ..p, extra: 1 })\n}\n",
  );

  assert_eq!(errors_of(&src), ["POLAR0707 at \"eq\""]);
}

#[test]
fn anonymous_param_accepts_post() {
  no_errors(&with_post(
    "size(r: { id: Int, title: String }) -> Int {\n  r.id\n}\n\nf(p: Post) {\n  size(p)\n}\n",
  ));
}

#[test]
fn generic_alias_derive() {
  let src = "types\n  Pair<a> = { l: a, r: a } derive(Same)\n\ntraits\n  Same<a> {\n    eq(x: a, y: a) -> Bool\n  }\n";
  let src = src.replace(
    "types\n  Pair<a> = { l: a, r: a } derive(Same)\n\ntraits\n  Same<a> {\n    eq(x: a, y: a) -> Bool\n  }\n",
    "traits\n  Same<a> {\n    eq(x: a, y: a) -> Bool\n  }\n\ntypes\n  Pair<a> = { l: a, r: a } derive(Eq)\n",
  );
  let diagnostics = diagnostics_of(&src);

  assert_eq!(errors_of(&src), ["POLAR0710 at \"derive(Eq)\""]);
  assert!(
    diagnostics[0].message.contains("generic record alias"),
    "{diagnostics:#?}"
  );
}

#[test]
fn emit_types_brand() {
  let src = with_post("f(p: Post) {\n  p\n}\n");

  assert_eq!(
    type_of(&src, "f"),
    "function({ id: Int, title: String } as Post) -> { id: Int, title: String } as Post"
  );
}

#[test]
fn literals_default_to_anonymous() {
  let src = with_post("f() {\n  { id: 1, title: \"x\" }\n}\n");

  assert_eq!(type_of(&src, "f"), "function() -> { id: Int, title: String }");
}

#[test]
fn blog_still_checks() {
  let src = std::fs::read_to_string(
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
      .join("tests/fixtures/programs/blog.px"),
  )
  .unwrap();

  assert!(
    type_of(&src, "publish").ends_with("} as Post"),
    "{}",
    type_of(&src, "publish")
  );
}
