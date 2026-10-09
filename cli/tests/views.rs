use polar_compiler::{
  CompileOptions, CompileOutput, compile, format_plugins,
  shared::codes::DiagnosticCode::PluginError, shared::diagnostic::Diagnostic,
  shared::modules::ModuleSource,
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
  let html = fs::read_to_string(framework().join("src/html.px")).unwrap();

  CompileOptions {
    plugins: plugin(),
    modules: vec![ModuleSource {
      path: "Framework.Html".to_string(),
      source: html,
      specifier: "./html.js".to_string(),
      plugins: Vec::new(),
    }],
    ..CompileOptions::default()
  }
}

fn build(src: &str) -> CompileOutput {
  compile(src, "test.px", &options())
}

fn diagnostics(src: &str) -> Vec<Diagnostic> {
  build(src).diagnostics
}

const HEADER: &str = "uses\n  Framework.Html\n  Std.List\n\ntypes\n  Post = { id: Int, title: String }\n\n";

fn program(views: &str) -> String {
  format!("{HEADER}views\n{views}\nexports\n  Post\n  card\n")
}

#[track_caller]
fn js(src: &str) -> String {
  let out = build(src);

  assert!(out.diagnostics.is_empty(), "{:#?}", out.diagnostics);
  out.js
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
fn markup_becomes_static_strings_and_holes() {
  let out = js(&program(
    "  card(post: Post)\n    <article class=\"post\" data-id={post.id}>\n      <h2>{post.title}</h2>\n    </article>\n",
  ));

  assert!(out.contains(r#""<article class=\"post\" data-id=\"""#), "{out}");
  assert!(out.contains(r#""\"><h2>""#), "{out}");
  assert!(out.contains(r#""</h2></article>""#), "{out}");
  assert!(out.contains("$Render$String, post.title"), "{out}");
  assert!(out.contains("$Render$Int, post.id"), "{out}");
}

#[test]
fn text_keeps_spaces_on_a_line_and_joins_lines() {
  let out = js(&program(
    "  card(post: Post)\n    <p>\n      Written by {post.title} today,\n      not yesterday\n    </p>\n",
  ));

  assert!(out.contains(r#""<p>Written by ""#), "{out}");
  assert!(out.contains(r#"" today, not yesterday</p>""#), "{out}");
}

#[test]
fn text_may_hold_characters_polar_code_cannot() {
  let out = js(&program(
    "  card(post: Post)\n    <p>Don't; 50% off? Q&amp;A @me $5 — “ok”</p>\n",
  ));

  assert!(
    out.contains("<p>Don't; 50% off? Q&amp;A @me $5 — “ok”</p>"),
    "{out}"
  );
}

#[test]
fn a_comment_in_markup_is_dropped() {
  let out =
    js(&program("  card(post: Post)\n    <p>kept // dropped\n    </p>\n"));

  assert!(out.contains(r#""<p>kept</p>""#), "{out}");
}

#[test]
fn void_and_self_closing_elements() {
  let out = js(&program(
    "  card(post: Post)\n    <div>\n      <br />\n      <span />\n    </div>\n",
  ));

  assert!(out.contains(r#""<div><br><span></span></div>""#), "{out}");
}

#[test]
fn components_call_views_with_named_arguments_and_children() {
  let out = js(&program(
    "  frame(title: String, children: Html)\n    <section>\n      <h1>{title}</h1>\n      {children}\n    </section>\n\n  card(post: Post)\n    <Frame title={post.title}>\n      <p>body</p>\n    </Frame>\n",
  ));

  assert!(out.contains("frame(post.title, "), "{out}");
  assert!(out.contains(r#""<p>body</p>""#), "{out}");
}

#[test]
fn a_type_error_in_a_hole_points_into_the_view() {
  let src = program("  card(post: Post)\n    <h2>{post.missing}</h2>\n");
  let found = diagnostics(&src);
  let hole = src.find("post.missing").unwrap();

  let missing = found
    .iter()
    .find(|d| d.message.contains("missing field `missing`"))
    .unwrap_or_else(|| panic!("{found:#?}"));

  assert!(
    missing.primary.span.start >= hole
      && missing.primary.span.end <= hole + "post.missing".len(),
    "{found:#?}"
  );
}

#[test]
fn views_need_framework_html() {
  let src = program("  card(post: Post)\n    <p>x</p>\n")
    .replace("  Framework.Html\n", "");

  assert!(
    diagnostics(&src)
      .iter()
      .any(|d| d.message.contains("needs `Framework.Html`")),
    "{:#?}",
    diagnostics(&src)
  );
}

#[test]
fn a_closing_tag_that_does_not_match() {
  only(
    &program("  card(post: Post)\n    <div>\n      x\n    </p>\n"),
    "`</p>` closes `<div>`",
    "</p>",
  );
}

#[test]
fn a_tag_that_is_never_closed() {
  only(
    &program("  card(post: Post)\n    <div>\n      x\n"),
    "`<div>` is never closed",
    "<div",
  );
}

#[test]
fn an_empty_hole() {
  only(
    &program("  card(post: Post)\n    <p>{}</p>\n"),
    "has no expression",
    "{}",
  );
}

#[test]
fn a_void_element_with_children() {
  only(
    &program("  card(post: Post)\n    <br>x</br>\n"),
    "can't have children",
    "<br",
  );
}

#[test]
fn an_unknown_component() {
  only(
    &program("  card(post: Post)\n    <Missing />\n"),
    "no view `missing`",
    "<Missing",
  );
}

#[test]
fn a_component_missing_an_argument() {
  only(
    &program(
      "  frame(title: String)\n    <h1>{title}</h1>\n\n  card(post: Post)\n    <Frame />\n",
    ),
    "`<Frame>` needs `title`",
    "<Frame",
  );
}

#[test]
fn a_component_given_an_unknown_argument() {
  only(
    &program(
      "  frame(title: String)\n    <h1>{title}</h1>\n\n  card(post: Post)\n    <Frame title=\"x\" size={1} />\n",
    ),
    "has no parameter `size`",
    "size",
  );
}

#[test]
fn children_for_a_component_that_takes_none() {
  only(
    &program(
      "  frame(title: String)\n    <h1>{title}</h1>\n\n  card(post: Post)\n    <Frame title=\"x\">\n      y\n    </Frame>\n",
    ),
    "takes none",
    "<Frame",
  );
}

#[test]
fn a_view_named_like_a_function() {
  let src = format!(
    "{HEADER}views\n  card(post: Post)\n    <p>x</p>\n\nfunctions\n  card() {{\n    1\n  }}\n"
  );

  assert!(
    diagnostics(&src)
      .iter()
      .any(|d| d.message.contains("already a function `card`")),
    "{:#?}",
    diagnostics(&src)
  );
}

#[test]
fn formatter_reindents_markup() {
  let src = program(
    "  card(post: Post)\n      <div>\n         <p>x</p>\n      </div>\n",
  );
  let once =
    format_plugins(&src, "test.px", &plugin()).output.expect("formats");

  assert!(
    once.contains(
      "views\n  card(post: Post)\n    <div>\n      <p>x</p>\n    </div>\n"
    ),
    "{once}"
  );
  assert_eq!(format_plugins(&once, "test.px", &plugin()).output.unwrap(), once);
}
