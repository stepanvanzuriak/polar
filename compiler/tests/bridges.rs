use polar_compiler::{
  CompileOptions, Stage, dump_stage_with,
  shared::diagnostic::{Diagnostic, Severity},
  shared::source::SourceFile,
};

const PREAMBLE: &str = "module App

uses
  Std.Json

hosts
  Browser
  Node
  Worker

types
  Post = { id: Int, title: String } derive(Json)
  Draft = { body: String }
  Missing = Missing(Int) derive(Json)
  Opaque = Opaque(Int)

effects
  Db in Node {
    load(id: Int) -> String
  }

  Posts in Node {
    fetch(id: Int) -> Post / {Throws<Missing>}
    count() -> Int
    touch(id: Int) -> {}
  }

  native Clock in Node {
    now() -> Int
  }

  Dom in Browser {
    show(text: String) -> {}
  }
";

const NODE_BINDS: &str = "  Db in Node = \"./db.js\"
  Dom in Browser = \"./dom.js\"
  Clock in Node = \"./clock.js\"

  Posts in Node {
    fetch(id) { { id: id, title: Db.load(id) } }
    count() { 1 }
    touch(id) { {} }
  }
";

fn with_binds(binds: &str) -> String {
  format!("{PREAMBLE}\nbinds\n{NODE_BINDS}{binds}")
}

fn with_functions(binds: &str, fns: &str) -> String {
  format!("{}\nfunctions\n{fns}", with_binds(binds))
}

fn stage(src: &str, stage: Stage) -> (Option<String>, Vec<Diagnostic>) {
  let result =
    dump_stage_with(src, "app.px", stage, &CompileOptions::default());

  (result.output, result.diagnostics)
}

fn diagnostics(src: &str) -> Vec<Diagnostic> {
  stage(src, Stage::Types).1
}

fn errors(src: &str) -> Vec<Diagnostic> {
  diagnostics(src)
    .into_iter()
    .filter(|d| d.severity == Severity::Error)
    .collect()
}

fn codes(src: &str) -> Vec<String> {
  diagnostics(src).iter().map(|d| d.code.to_string()).collect()
}

fn only(src: &str) -> Diagnostic {
  let found = diagnostics(src);

  assert_eq!(found.len(), 1, "{found:#?}");
  found.into_iter().next().unwrap()
}

fn slice(src: &str, d: &Diagnostic) -> String {
  SourceFile::new("app.px", src).slice(&d.primary.span).to_string()
}

fn hosts(src: &str) -> String {
  let (output, diagnostics) = stage(src, Stage::Hosts);

  output.unwrap_or_else(|| panic!("{diagnostics:#?}"))
}

#[test]
fn bridge_type_checks() {
  let src = with_functions(
    "  Posts in Browser from Node\n",
    "  on_click(id: Int) -> {} / {Dom, Posts, Throws<Missing>} {\n    Dom.show(Posts.fetch(id).title)\n  }\n",
  );

  assert!(errors(&src).is_empty(), "{:#?}", errors(&src));
}

#[test]
fn bridge_adds_its_host() {
  let src = with_functions(
    "  Posts in Browser from Node\n",
    "  count() -> Int / {Posts} {\n    Posts.count()\n  }\n\n  on_click(id: Int) -> {} / {Dom, Posts} {\n    Dom.show(\"#{Posts.count()}\")\n  }\n",
  );
  let out = hosts(&src);

  assert!(out.contains("on_click : {Browser}"), "{out}");
  assert!(out.contains("count    : {Browser, Node}"), "{out}");
  assert!(out.contains("Posts in Browser from Node"), "{out}");
}

#[test]
fn without_bridge_still_rejected() {
  let src = with_functions(
    "",
    "  on_click(id: Int) -> {} / {Dom, Posts} {\n    Dom.show(\"#{Posts.count()}\")\n  }\n",
  );

  assert_eq!(codes(&src), vec!["POLAR0811"]);
}

#[test]
fn target_unbound() {
  let src = with_binds("  Posts in Browser from Worker\n");
  let d = only(&src);

  assert_eq!(d.code.to_string(), "POLAR0818");
  assert_eq!(slice(&src, &d), "Worker");
  assert!(d.message.contains("no binding in `Worker`"), "{}", d.message);
}

#[test]
fn target_unknown_host() {
  let src = with_binds("  Posts in Browser from Mars\n");

  assert_eq!(codes(&src), vec!["POLAR0319"]);
}

#[test]
fn chain_rejected() {
  let src =
    with_binds("  Posts in Worker from Node\n  Posts in Browser from Worker\n");
  let d = only(&src);

  assert_eq!(d.code.to_string(), "POLAR0820");
  assert!(d.message.contains("itself a bridge to `Node`"), "{}", d.message);
  assert_eq!(d.help.as_deref(), Some("write `Posts in Browser from Node`"));
}

#[test]
fn duplicate_bridge() {
  let src =
    with_binds("  Posts in Browser from Node\n  Posts in Browser from Node\n");

  assert_eq!(codes(&src), vec!["POLAR0804"]);
}

#[test]
fn bridge_and_binding_on_same_host() {
  let src = with_binds(
    "  Posts in Browser from Node\n  Posts in Browser {\n    fetch(id) { { id: id, title: \"\" } }\n    count() { 0 }\n    touch(id) { {} }\n  }\n",
  );

  assert_eq!(codes(&src), vec!["POLAR0804"]);
}

#[test]
fn native_needs_no_force() {
  let src = with_binds("  Clock in Browser from Node\n");

  assert!(diagnostics(&src).is_empty(), "{:#?}", diagnostics(&src));
}

#[test]
fn force_on_bridge_is_needless() {
  let src = with_binds("  force Clock in Browser from Node\n");
  let d = only(&src);

  assert_eq!(d.code.to_string(), "POLAR0806");
  assert_eq!(d.severity, Severity::Warning);
}

#[test]
fn bridge_skips_missing_operations() {
  let src = with_binds("  Posts in Browser from Node\n");

  assert!(diagnostics(&src).is_empty(), "{:#?}", diagnostics(&src));
}

#[test]
fn empty_body_still_missing_operations() {
  let src = with_binds("  Clock in Browser { }\n");

  assert!(codes(&src).contains(&"POLAR0801".to_string()), "{:?}", codes(&src));
}

#[test]
fn unit_result_crosses() {
  let src = with_binds("  Posts in Browser from Node\n");

  assert!(errors(&src).is_empty());
}

fn wire(effect: &str, ops: &str, binds: &str) -> String {
  format!(
    "{PREAMBLE}\n  {effect} in Node {{\n{ops}  }}\n\nbinds\n{NODE_BINDS}{binds}  {effect} in Browser from Node\n"
  )
}

#[test]
fn parameter_without_json() {
  let src = wire(
    "Drafts",
    "    save(draft: Draft) -> Int\n",
    "  Drafts in Node {\n    save(draft) { 1 }\n  }\n",
  );
  let d = only(&src);

  assert_eq!(d.code.to_string(), "POLAR0819");
  assert!(
    d.message.contains("`Drafts.save`")
      && d.message.contains("parameter `draft` is `Draft`"),
    "{}",
    d.message
  );
  assert_eq!(slice(&src, &d), "Drafts in Browser from Node");
  assert_eq!(
    d.help.as_deref(),
    Some("add `derive(Json)` to `Draft`, or give it a `Json` impl")
  );
}

#[test]
fn result_without_json() {
  let src = wire(
    "Drafts",
    "    latest() -> Draft\n",
    "  Drafts in Node {\n    latest() { { body: \"\" } }\n  }\n",
  );
  let d = only(&src);

  assert!(d.message.contains("result is `Draft`"), "{}", d.message);
}

#[test]
fn error_without_json() {
  let src = wire(
    "Drafts",
    "    risky() -> Int / {Throws<Opaque>}\n",
    "  Drafts in Node {\n    risky() { throw Opaque(1) }\n  }\n",
  );
  let d = only(&src);

  assert!(d.message.contains("error is `Opaque`"), "{}", d.message);
}

#[test]
fn nested_types_cross() {
  let src = format!(
    "module App\n\nuses\n  Std.Json\n  Std.List\n  Std.Option\n\n{}",
    wire(
      "Feed",
      "    page(ids: List<Int>) -> List<Option<Post>>\n",
      "  Feed in Node {\n    page(ids) { [] }\n  }\n",
    )
    .replacen("module App\n\nuses\n  Std.Json\n", "", 1)
  );

  assert!(errors(&src).is_empty(), "{:#?}", errors(&src));
}

#[test]
fn nested_without_json() {
  let src = format!(
    "module App\n\nuses\n  Std.Json\n  Std.List\n\n{}",
    wire(
      "Feed",
      "    save(drafts: List<Draft>) -> Int\n",
      "  Feed in Node {\n    save(drafts) { 1 }\n  }\n",
    )
    .replacen("module App\n\nuses\n  Std.Json\n", "", 1)
  );
  let d = only(&src);

  assert!(d.message.contains("`List<Draft>`"), "{}", d.message);
}

#[test]
fn generic_operation_rejected() {
  let src = wire(
    "Echo",
    "    echo(x: a) -> a\n",
    "  Echo in Node {\n    echo(x) { x }\n  }\n",
  );
  let found = errors(&src);

  assert_eq!(found.len(), 2, "{found:#?}");
  assert!(found.iter().all(|d| d.code.to_string() == "POLAR0819"));
  assert_eq!(
    found[0].help.as_deref(),
    Some("a bridged operation can't be generic: give it a concrete type")
  );
}

#[test]
fn needs_std_json() {
  let src = "module App

hosts
  Browser
  Node

effects
  Posts in Node {
    count() -> Int
  }

binds
  Posts in Node {
    count() { 1 }
  }

  Posts in Browser from Node
";
  let d = only(src);

  assert_eq!(d.code.to_string(), "POLAR0819");
  assert_eq!(d.help.as_deref(), Some("add `uses Std.Json`"));
}

fn emitted(src: &str, host: &str) -> polar_compiler::CompileOutput {
  let options = CompileOptions {
    host: polar_compiler::HostOption::Fixed(Some(host.to_string())),
    ..CompileOptions::default()
  };
  let out = polar_compiler::compile(src, "app.px", &options);

  assert!(
    out.diagnostics.iter().all(|d| d.severity != Severity::Error),
    "{:#?}",
    out.diagnostics
  );
  out
}

fn bridged() -> String {
  with_functions(
    "  Posts in Browser from Node\n",
    "  on_click(id: Int) -> {} / {Dom, Posts, Throws<Missing>} {\n    Dom.show(Posts.fetch(id).title)\n  }\n",
  )
}

#[test]
fn client_stub_calls_the_bridge() {
  let js = emitted(&bridged(), "Browser").js;

  assert!(
    js.contains("$rt.bridge.op(\"Posts\", \"fetch\", [$Json.$Json$Int], $Std$Json$Json$Post, [[\"App.Missing\", $Std$Json$Json$Missing]])"),
    "{js}"
  );
  assert!(
    js.contains(
      "touch: $rt.bridge.op(\"Posts\", \"touch\", [$Json.$Json$Int], null, [])"
    ),
    "{js}"
  );
  assert!(js.contains("import * as $Json from"), "{js}");
}

#[test]
fn client_holds_no_server_code() {
  let js = emitted(&bridged(), "Browser").js;

  for leak in ["./db.js", "$bind$Db", "$bridge$", "Db.load"] {
    assert!(!js.contains(leak), "`{leak}` in {js}");
  }
}

#[test]
fn server_exports_a_table() {
  let out = emitted(&bridged(), "Node");

  assert!(
    out.js.contains(
      "export const $bridge$Posts = $rt.bridge.table(\"Posts\", $bind$Posts, {"
    ),
    "{}",
    out.js
  );
  assert!(out.js.contains("count: [[], $Json.$Json$Int, []]"), "{}", out.js);
  assert_eq!(out.bridges, vec!["Posts".to_string()]);
}

#[test]
fn server_bind_stays_real() {
  let js = emitted(&bridged(), "Node").js;

  assert!(js.contains("fetch: async (id) =>"), "{js}");
  assert!(!js.contains("bridge.op"), "{js}");
}

#[test]
fn unrelated_host_gets_neither_end() {
  let out = emitted(&bridged(), "Worker");

  assert!(!out.js.contains("bridge."), "{}", out.js);
  assert!(out.bridges.is_empty());
}

#[test]
fn two_clients_share_one_table() {
  let src =
    with_binds("  Posts in Browser from Node\n  Posts in Worker from Node\n");
  let out = emitted(&src, "Node");

  assert_eq!(
    out.js.matches("export const $bridge$Posts").count(),
    1,
    "{}",
    out.js
  );
  assert_eq!(out.bridges, vec!["Posts".to_string()]);
}

#[test]
fn server_serves_an_effect_it_never_calls() {
  let src = with_binds("  Clock in Browser from Node\n");
  let js = emitted(&src, "Node").js;

  assert!(js.contains("$rt.bridge.table(\"Clock\", $bind$Clock"), "{js}");
}

#[test]
fn host_builds_drop_embedded_source() {
  let out = emitted(&bridged(), "Browser");

  assert!(
    out.sourcemap.contains("\"sourcesContent\":[null]"),
    "{}",
    out.sourcemap
  );
}

#[test]
fn single_host_programs_keep_embedded_source() {
  let src =
    "module App\n\nhosts\n  Node\n\nfunctions\n  f() -> Int {\n    1\n  }\n";
  let out = emitted(src, "Node");

  assert!(out.sourcemap.contains("f() -> Int"), "{}", out.sourcemap);
}

const CONSTANTS: &str = "module App

uses
  Std.List

hosts
  Browser
  Node

constants
  secret: List<Int> = [41, 42]
  base: Int = 7
  shared: Int = base + 1
  public: Int = 99

effects
  Db in Node {
    total() -> Int
  }

binds
  Db in Node {
    total() { List.length(secret) }
  }

functions
  label() -> Int {
    shared
  }

exports
  label
  public
";

#[test]
fn constants_only_a_server_bind_uses_stay_on_the_server() {
  let browser = emitted(CONSTANTS, "Browser").js;
  let node = emitted(CONSTANTS, "Node").js;

  assert!(!browser.contains("secret"), "{browser}");
  assert!(!browser.contains("41"), "{browser}");
  assert!(node.contains("const secret"), "{node}");
}

#[test]
fn constants_a_kept_function_uses_stay_with_their_dependencies() {
  let browser = emitted(CONSTANTS, "Browser").js;

  assert!(browser.contains("const shared"), "{browser}");
  assert!(browser.contains("const base"), "{browser}");
}

#[test]
fn exported_constants_stay_on_every_host() {
  for host in ["Browser", "Node"] {
    let js = emitted(CONSTANTS, host).js;

    assert!(js.contains("public"), "{host}: {js}");
  }
}
