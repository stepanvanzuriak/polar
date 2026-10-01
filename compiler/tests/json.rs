mod common;

use common::node::run_program;
use polar_compiler::{
  Stage, dump_stage, shared::diagnostic::Diagnostic, shared::source::SourceFile,
};

const TYPES: &str = "  Status = Draft | Published(Int) derive(Json, Eq)
  Post = { id: Int, title: String, status: Status } derive(Json, Eq)";

fn program(main: &str) -> String {
  let main: String = main.lines().flat_map(|l| ["    ", l, "\n"]).collect();

  format!(
    "uses\n  Std.Json\n  Std.List\n  Std.Option\n  Std.Result\n\ntypes\n{TYPES}\n\nfunctions\n  main() {{\n{main}  }}\n\n  report(result: Result<JsonError, a>) -> String {{\n    match result {{\n      Ok(_) -> \"ok\",\n      Err(e) -> Json.message(e),\n    }}\n  }}\n\nexports\n  main\n"
  )
}

fn run(main: &str) -> Vec<String> {
  run_program(&program(main), "test.px", &[])
    .unwrap_or_else(|e| panic!("{e}"))
    .lines()
    .map(str::to_string)
    .collect()
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

fn decoded(json: &str, ty: &str) -> String {
  let literal = json.replace('\\', "\\\\").replace('"', "\\\"");
  let out = run(&format!(
    "let result: Result<JsonError, {ty}> = Json.decode(\"{literal}\")\nLog.info(report(result))"
  ));

  out.join("\n")
}

#[test]
fn encode_primitives() {
  assert_eq!(
    run(
      "Log.info(Json.encode(1))\nLog.info(Json.encode(2.5))\nLog.info(Json.encode(\"a\"))\nLog.info(Json.encode(true))"
    ),
    ["1", "2.5", "\"a\"", "true"]
  );
}

#[test]
fn encode_record() {
  assert_eq!(
    run(
      "let post: Post = { id: 1, title: \"Hi\", status: Draft }\nLog.info(Json.encode(post))"
    ),
    ["{\"id\":1,\"title\":\"Hi\",\"status\":{\"$\":\"Draft\"}}"]
  );
}

#[test]
fn encode_variant() {
  assert_eq!(
    run("Log.info(Json.encode(Draft))\nLog.info(Json.encode(Published(2026)))"),
    ["{\"$\":\"Draft\"}", "{\"$\":\"Published\",\"_0\":2026}"]
  );
}

#[test]
fn encode_list_is_array() {
  assert_eq!(run("Log.info(Json.encode([1, 2]))"), ["[1,2]"]);
}

#[test]
fn round_trip_post() {
  assert_eq!(
    run(
      "let post: Post = { id: 1, title: \"Hi\", status: Published(2026) }\nlet back: Result<JsonError, Post> = Json.decode(Json.encode(post))\nmatch back {\n  Ok(p) -> Log.info(\"#{p == post}\"),\n  Err(e) -> Log.info(Json.message(e)),\n}"
    ),
    ["true"]
  );
}

#[test]
fn decode_wrong_type() {
  assert_eq!(
    decoded(
      "{\"id\":\"1\",\"title\":\"Hi\",\"status\":{\"$\":\"Draft\"}}",
      "Post"
    ),
    ".id: expected Int, found a string"
  );
}

#[test]
fn decode_missing_field() {
  assert_eq!(decoded("{\"id\":1}", "Post"), ".title: missing field");
}

#[test]
fn decode_unknown_ctor() {
  assert_eq!(
    decoded("{\"$\":\"Archived\"}", "Status"),
    ".$: expected Draft or Published, found \"Archived\""
  );
}

#[test]
fn decode_not_json() {
  let out = decoded("{", "Post");

  assert!(out.starts_with("expected JSON, found "), "{out}");
}

#[test]
fn decode_int_not_float() {
  assert_eq!(decoded("2.5", "Int"), "expected Int, found 2.5");
}

#[test]
fn decode_nested_path() {
  assert_eq!(
    decoded(
      "{\"id\":1,\"title\":\"Hi\",\"status\":{\"$\":\"Published\",\"_0\":\"x\"}}",
      "Post"
    ),
    ".status._0: expected Int, found a string"
  );
}

#[test]
fn decode_needs_annotation() {
  let src = "uses\n  Std.Json\n\nfunctions\n  f(t: String) {\n    Json.decode(t)\n  }\n\n  g(t: String) -> String {\n    Json.message(Json.decode(t))\n  }\n";
  let src = src.replace(
    "  g(t: String) -> String {\n    Json.message(Json.decode(t))\n  }\n",
    "",
  );
  let found = errors_of(&format!(
    "{src}\n  h(t: String) -> String {{\n    match Json.decode(t) {{\n      _ -> \"\",\n    }}\n  }}\n"
  ));

  assert_eq!(found.len(), 1, "{found:?}");
  assert!(found[0].starts_with("POLAR0708 at \"decode\""), "{found:?}");

  let diagnostic = diagnostics_of(&format!(
    "{src}\n  h(t: String) -> String {{\n    match Json.decode(t) {{\n      _ -> \"\",\n    }}\n  }}\n"
  ))
  .remove(0);

  assert!(diagnostic.help.iter().any(|h| h.contains("type annotation")));
}

#[test]
fn derive_json_needs_uses() {
  let src = "types\n  Status = A | B derive(Json)\n";
  let diagnostics = diagnostics_of(src);

  assert_eq!(errors_of(src), ["POLAR0701 at \"Json\""]);
  assert_eq!(diagnostics[0].help.as_deref(), Some("add `uses Std.Json`"));
}

#[test]
fn json_on_function_field() {
  let src = "uses\n  Std.Json\n\ntypes\n  Button = { label: String, on_click: function() -> {} } derive(Json)\n";
  let diagnostics = diagnostics_of(src);

  assert!(
    errors_of(src)
      .iter()
      .all(|e| e == "POLAR0707 at \"on_click: function() -> {}\""),
    "{:?}",
    errors_of(src)
  );
  assert_eq!(diagnostics[0].message, "`Button` can't derive `Json`");
}

#[test]
fn generic_round_trip() {
  assert_eq!(
    run(
      "let post: Post = { id: 1, title: \"Hi\", status: Draft }\nlet some: Option<Post> = Some(post)\nlet back: Result<JsonError, Option<Post>> = Json.decode(Json.encode(some))\nLog.info(Json.encode(some))\nmatch back {\n  Ok(p) -> Log.info(\"#{p == some}\"),\n  Err(e) -> Log.info(Json.message(e)),\n}\nlet statuses = [Draft, Published(1)]\nlet listed: Result<JsonError, List<Status>> = Json.decode(Json.encode(statuses))\nmatch listed {\n  Ok(xs) -> Log.info(\"#{xs == statuses}\"),\n  Err(e) -> Log.info(Json.message(e)),\n}\nlet none: Option<Int> = None\nLog.info(Json.encode(none))"
    ),
    [
      "{\"id\":1,\"title\":\"Hi\",\"status\":{\"$\":\"Draft\"}}",
      "true",
      "true",
      "null"
    ]
  );
}

#[test]
fn result_round_trip() {
  assert_eq!(
    run(
      "let r: Result<String, Int> = Err(\"no\")\nLog.info(Json.encode(r))\nlet back: Result<JsonError, Result<String, Int>> = Json.decode(Json.encode(r))\nmatch back {\n  Ok(x) -> Log.info(\"#{x == r}\"),\n  Err(e) -> Log.info(Json.message(e)),\n}"
    ),
    ["{\"$\":\"Err\",\"_0\":\"no\"}", "true"]
  );
}

#[test]
fn blog_runs() {
  let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
  let src = std::fs::read_to_string(
    root.join("compiler/tests/fixtures/programs/blog.px"),
  )
  .unwrap();
  let expected = std::fs::read_to_string(
    root.join("compiler/tests/fixtures/programs/blog.expected.txt"),
  )
  .unwrap();

  assert_eq!(run_program(&src, "blog.px", &[]).unwrap(), expected);
  assert!(expected.contains("round trip: true"));
}

#[test]
fn json_raw_is_internal() {
  let src = "functions\n  f() {\n    JsonRaw.print(1)\n  }\n";

  assert_eq!(errors_of(src), ["POLAR0305 at \"JsonRaw\""]);
}

#[test]
fn derive_json_without_result_in_scope() {
  let src = "uses\n  Std.Json\n\ntypes\n  Verdict = Ok | Err(String) derive(Json)\n  Post = { id: Int, verdict: Verdict } derive(Json)\n\nfunctions\n  main() {\n    let post: Post = { id: 1, verdict: Err(\"no\") }\n    Log.info(Json.encode(post))\n  }\n\nexports\n  main\n";

  assert_eq!(errors_of(src), Vec::<String>::new());
  assert_eq!(
    run_program(src, "test.px", &[]).unwrap().trim(),
    "{\"id\":1,\"verdict\":{\"$\":\"Err\",\"_0\":\"no\"}}"
  );
}
