use polar_compiler::{
  CompileOptions, CompileOutput, HostOption, compile,
  shared::diagnostic::Severity, stdlib,
};

const APP: &str = "module App

uses
  Std.Dom

hosts
  Browser
  Node

functions
  show(text: String) -> {} / {Dom} {
    Dom.append_text(\"p\", text)
  }

  render(html: String) -> {} / {Dom} {
    if Dom.has(\"app\") { Dom.set_html(\"app\", html) } else { Dom.append_html(html) }
  }

  total() -> Int {
    1
  }

exports
  render
  show
  total
";

fn emitted(src: &str, host: &str) -> CompileOutput {
  let options = CompileOptions {
    host: HostOption::Fixed(Some(host.to_string())),
    ..CompileOptions::default()
  };
  let out = compile(src, "app.px", &options);

  assert!(
    out.diagnostics.iter().all(|d| d.severity != Severity::Error),
    "{:#?}",
    out.diagnostics
  );
  out
}

#[test]
fn a_program_declaring_browser_can_use_std_dom() {
  emitted(APP, "Browser");
}

#[test]
fn browser_calls_go_through_the_std_binding() {
  let out = emitted(APP, "Browser");

  assert!(out.js.contains("$Dom.$bind$Dom.append_text("), "{}", out.js);
  assert!(out.std_imports.contains(&"Dom".to_string()));
}

#[test]
fn node_does_not_import_std_dom() {
  let out = emitted(APP, "Node");

  assert!(!out.js.contains("Dom"), "{}", out.js);
  assert!(!out.std_imports.contains(&"Dom".to_string()));
}

#[test]
fn std_dom_ships_its_binding() {
  let dom = stdlib::module("Dom").expect("Std.Dom exists");
  let out = compile(
    dom.source,
    &stdlib::filename("Dom"),
    &CompileOptions {
      runtime: "../runtime.js".to_string(),
      ..CompileOptions::default()
    },
  );

  assert!(out.js.contains("from \"./bindings/Dom.js\""), "{}", out.js);
  assert!(dom.files.iter().any(|(path, _)| *path == "bindings/Dom.js"));
}

#[test]
fn the_dom_binding_has_no_branches() {
  let dom = stdlib::module("Dom").expect("Std.Dom exists");

  for (path, text) in dom.files {
    assert!(!text.contains("if ") && !text.contains('?'), "{path}: {text}");
  }
}

#[test]
fn a_host_declared_twice_locally_is_still_an_error() {
  let src = "module App\n\nhosts\n  Browser\n  Browser\n";
  let out = compile(src, "app.px", &CompileOptions::default());

  assert!(
    out.diagnostics.iter().any(|d| d.message.contains("more than once")),
    "{:#?}",
    out.diagnostics
  );
}
