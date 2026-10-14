use polar_compiler::{CompileOptions, compile, fmt::format};

const HEAD: &str = "module Main

uses
  Std.Json
  Std.Option
  Std.Result

hosts
  Node

types
  Post = { id: Int, title: String } derive(Json, Eq)
  Plain = { n: Int }

effects
  Store in Node {
    get(key: String) -> Option<a> where Json<a>
    put(key: String, value: a) -> {} where Json<a>
    pick(left: a, right: a) -> a where Eq<a>, Json<a>
    size() -> Int
  }

binds
  Store in Node {
    get(key) {
      match Json.decode(key) {
        Ok(v) -> Some(v),
        Err(e) -> None,
      }
    }

    put(key, value) {
      {}
    }

    pick(left, right) {
      if left == right { left } else { right }
    }

    size() {
      0
    }
  }

functions
";

fn build(functions: &str) -> polar_compiler::CompileOutput {
  compile(&format!("{HEAD}{functions}"), "main.px", &CompileOptions::default())
}

#[test]
fn constrained_operations_compile() {
  let out = build(
    "  main() {
    let post: Post = { id: 1, title: \"a\" }
    let found: Option<Post> = Store.get(\"k\")
    Store.put(\"k\", post)
    Store.size()
  }
",
  );

  assert!(out.diagnostics.is_empty(), "{:#?}", out.diagnostics);
  assert!(out.js.contains("get: async ($d0, key)"), "{}", out.js);
  assert!(out.js.contains("pick: async ($d0, $d1, left, right)"), "{}", out.js);
  assert!(
    out.js.contains("$bind$Store.get($Std$Json$Json$Post, \"k\")"),
    "{}",
    out.js
  );
}

#[test]
fn unconstrained_operation_is_unchanged() {
  let out = build("  main() {\n    Store.size()\n  }\n");

  assert!(out.diagnostics.is_empty(), "{:#?}", out.diagnostics);
  assert!(out.js.contains("size: async () =>"), "{}", out.js);
}

#[test]
fn missing_impl_is_reported_at_the_call() {
  let out = build(
    "  main() {\n    let plain: Plain = { n: 1 }\n    Store.put(\"k\", plain)\n  }\n",
  );

  assert_eq!(codes_of(&out), vec!["POLAR0707"]);
  assert!(out.diagnostics[0].message.contains("Plain"));
}

fn codes_of(out: &polar_compiler::CompileOutput) -> Vec<String> {
  out.diagnostics.iter().map(|d| d.code.to_string()).collect()
}

#[test]
fn unbound_variable_in_where() {
  let src = "module Main

uses
  Std.Json

hosts
  Node

effects
  Store in Node {
    get(key: String) -> Int where Json<b>
  }
";

  let out = compile(src, "main.px", &CompileOptions::default());

  assert_eq!(codes_of(&out), vec!["POLAR0511"]);
}

#[test]
fn formatter_prints_the_clause() {
  let src = "module Main\n\neffects\n  Store {\n    get(key: String) -> Option<a> where Json<a>\n    pick(l: a, r: a) -> a where Eq<a>, Json<a>\n  }\n";
  let formatted = format(src, "main.px");

  assert!(formatted.diagnostics.is_empty(), "{:#?}", formatted.diagnostics);
  assert_eq!(formatted.output.as_deref(), Some(src));
}
