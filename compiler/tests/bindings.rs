mod common;

use common::node::run_program_with;
use polar_compiler::{
  CompileOptions, CompileOutput, HostOption, choose_host, compile,
  shared::modules::ModuleSource,
};

const NOTES: &str = "module Notes

hosts
  DOM
  Node

effects
  Storage in DOM {
    get(key: String) -> String
    set(key: String, value: String) -> {}
  }

externs
  local_get(key: String) -> String = \"./dom.js\" get_item

binds
  Storage in DOM {
    get(key) {
      local_get(key)
    }

    set(key, value) {
      {}
    }
  }

  Storage in Node {
    get(key) {
      \"node:#{key}\"
    }

    set(key, value) {
      {}
    }
  }

functions
  title(key) {
    Storage.get(key)
  }
";

fn compile_for(
  src: &str,
  host: HostOption,
  modules: Vec<ModuleSource>,
) -> CompileOutput {
  let options = CompileOptions { host, modules, ..CompileOptions::default() };

  compile(src, "notes.px", &options)
}

fn emit_for_host(src: &str, host: &str) -> String {
  let out = compile_for(src, HostOption::Flag(host.to_string()), Vec::new());

  assert!(out.diagnostics.is_empty(), "{:#?}", out.diagnostics);
  assert_eq!(out.host_error, None);
  out.js
}

fn codes_for_host(src: &str, host: &str) -> Vec<String> {
  compile_for(src, HostOption::Flag(host.to_string()), Vec::new())
    .diagnostics
    .iter()
    .map(|d| d.code.to_string())
    .collect()
}

fn hosts(names: &[&str]) -> Vec<String> {
  names.iter().map(|n| (*n).to_string()).collect()
}

#[test]
fn choose_explicit() {
  assert_eq!(
    choose_host(Some("Node"), &hosts(&["DOM", "Node"])),
    Ok(Some("Node".to_string()))
  );
}

#[test]
fn choose_unknown() {
  assert_eq!(
    choose_host(Some("Mars"), &hosts(&["DOM", "Node"])),
    Err("unknown host `Mars`; this program declares `DOM`, `Node`".to_string())
  );
}

#[test]
fn choose_single() {
  assert_eq!(
    choose_host(None, &hosts(&["Node"])),
    Ok(Some("Node".to_string()))
  );
}

#[test]
fn choose_none_declared() {
  assert_eq!(choose_host(None, &[]), Ok(None));
}

#[test]
fn choose_ambiguous() {
  assert_eq!(
    choose_host(None, &hosts(&["DOM", "Node"])),
    Err(
      "this program declares hosts `DOM` and `Node`; choose one with `--host`"
        .to_string()
    )
  );
}

#[test]
fn compile_reports_ambiguous_host() {
  let out = compile_for(NOTES, HostOption::Auto, Vec::new());

  assert!(out.js.is_empty());
  assert_eq!(
    out.host_error.as_deref(),
    Some(
      "this program declares hosts `DOM` and `Node`; choose one with `--host`"
    )
  );
}

#[test]
fn emits_dom_binding() {
  assert_eq!(
    emit_for_host(NOTES, "DOM"),
    "import * as $rt from \"./_polar/runtime.js\";
import { get_item as $ext$local_get } from \"./dom.js\";

export const $bind$Storage = {
  get: async (key) => await $ext$local_get(key),
  set: async (key, value) => ({}),
};

async function title(key) {
  return await $bind$Storage.get(key);
}
"
  );
}

#[test]
fn drops_other_host_binding() {
  let js = emit_for_host(NOTES, "Node");

  assert!(js.contains("export const $bind$Storage"));
  assert!(!js.contains("./dom.js"));
  assert!(js.contains("`node:"), "{js}");
}

const CANVAS: &str = "module App

hosts
  DOM
  Node

effects
  native Canvas in DOM {
    draw(s: String) -> {}
  }

  Db in Node {
    load(id: Int) -> String
  }

binds
  Canvas in DOM {
    draw(s) {
      Log.info(\"draw #{s}\")
    }
  }

functions
  paint() {
    Canvas.draw(\"x\")
  }

  greet() -> String {
    \"hi\"
  }
";

#[test]
fn drops_unrunnable_function() {
  let src = format!("{CANVAS}\nexports\n  paint\n  greet\n");
  let js = emit_for_host(&src, "Node");

  assert!(!js.contains("paint"), "{js}");
  assert!(js.contains("export function greet()"), "{js}");
  assert!(!js.contains("$bind$Canvas"), "{js}");
}

#[test]
fn main_cant_run() {
  let src = format!("{CANVAS}\n  main() {{\n    paint()\n  }}\n");
  let out = compile_for(&src, HostOption::Flag("Node".into()), Vec::new());
  let d = &out.diagnostics[0];

  assert_eq!(out.diagnostics.len(), 1, "{:#?}", out.diagnostics);
  assert_eq!(d.code.to_string(), "POLAR0815");
  assert_eq!(
    d.message,
    "`main` can't run on `Node`: it uses `Canvas`, which runs on {DOM}"
  );
  assert_eq!(d.secondary.len(), 1);
}

#[test]
fn missing_binding() {
  let src = format!("{CANVAS}\n  main() {{\n    Log.info(Db.load(1))\n  }}\n");

  assert_eq!(codes_for_host(&src, "Node"), vec!["POLAR0816"]);

  let out = compile_for(&src, HostOption::Flag("Node".into()), Vec::new());

  assert_eq!(out.diagnostics[0].message, "`Db` has no binding for `Node`");
}

fn run(
  src: &str,
  host: &str,
  modules: &[ModuleSource],
  files: &[(&str, &str)],
) -> String {
  run_program_with(src, "app.px", modules, Some(host), files)
    .unwrap_or_else(|e| panic!("{e}"))
}

#[test]
fn op_as_value() {
  let src = format!(
    "{}\n  main() {{\n    Log.info(List.join(List.map([\"a\", \"b\"], Storage.get), \",\"))\n  }}\n\nexports\n  main\n",
    NOTES.replace("module Notes\n", "module Notes\n\nuses\n  Std.List\n")
  );

  assert_eq!(run(&src, "Node", &[], &[]), "node:a,node:b\n");
}

#[test]
fn dom_binding_calls_extern() {
  let src = format!(
    "{NOTES}\n  main() {{\n    Log.info(title(\"k\"))\n  }}\n\nexports\n  main\n"
  );
  let dom = "export function get_item(key) {\n  return `dom:${key}`;\n}\n";

  assert_eq!(run(&src, "DOM", &[], &[("dom.js", dom)]), "dom:k\n");
  assert_eq!(run(&src, "Node", &[], &[]), "node:k\n");
}

#[test]
fn cross_module_bind() {
  let store = "module Store\n\nhosts\n  Node\n\neffects\n  Storage in Node {\n    get(key: String) -> String\n  }\n\nbinds\n  Storage in Node {\n    get(key) {\n      \"stored #{key}\"\n    }\n  }\n\nexports\n  Node\n  Storage\n";
  let main = "module Main\n\nuses\n  Store\n\nfunctions\n  main() {\n    Log.info(Storage.get(\"k\"))\n  }\n\nexports\n  main\n";
  let modules = vec![ModuleSource {
    path: "Store".to_string(),
    source: store.to_string(),
    specifier: "./store.js".to_string(),
    plugins: Vec::new(),
  }];
  let out = compile_for(main, HostOption::Flag("Node".into()), modules.clone());

  assert!(out.js.contains("$m$Store.$bind$Storage.get"), "{}", out.js);
  assert_eq!(run(main, "Node", &modules, &[]), "stored k\n");
}

#[test]
fn relative_specifier_build() {
  let options = CompileOptions {
    host: HostOption::Flag("DOM".into()),
    source_dir: Some("/work/app/src".into()),
    output_dir: Some("/work/app/dist".into()),
    ..CompileOptions::default()
  };
  let js = compile(NOTES, "notes.px", &options).js;

  assert!(js.contains("from \"../src/dom.js\""), "{js}");
}

#[test]
fn extern_inside_the_source_root_is_copied() {
  let options = CompileOptions {
    host: HostOption::Flag("DOM".into()),
    source_dir: Some("/work/app/src".into()),
    output_dir: Some("/work/app/dist".into()),
    source_root: Some("/work/app/src".into()),
    ..CompileOptions::default()
  };
  let out = compile(NOTES, "notes.px", &options);

  assert!(out.js.contains("from \"./dom.js\""), "{}", out.js);
  assert_eq!(
    out.extern_files,
    vec![("/work/app/src/dom.js".to_string(), "dom.js".to_string())]
  );

  let node = compile(
    NOTES,
    "notes.px",
    &CompileOptions { host: HostOption::Flag("Node".into()), ..options },
  );

  assert!(node.extern_files.is_empty());
}

#[test]
fn relative_specifier_run() {
  let options = CompileOptions {
    host: HostOption::Flag("DOM".into()),
    source_dir: Some("/work/my app/src".into()),
    ..CompileOptions::default()
  };
  let js = compile(NOTES, "notes.px", &options).js;

  assert!(js.contains("from \"file:///work/my%20app/src/dom.js\""), "{js}");
}

#[test]
fn bare_specifier() {
  let src = NOTES.replace("\"./dom.js\" get_item", "\"node:os\" hostname");
  let options = CompileOptions {
    host: HostOption::Flag("DOM".into()),
    source_dir: Some("/work/app/src".into()),
    output_dir: Some("/work/app/dist".into()),
    ..CompileOptions::default()
  };
  let js = compile(&src, "notes.px", &options).js;

  assert!(
    js.contains("import { hostname as $ext$local_get } from \"node:os\""),
    "{js}"
  );
}

#[test]
fn no_host_program_unchanged() {
  let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
    .join("tests/fixtures/programs");

  for name in ["hello.px", "blog.px"] {
    let src = std::fs::read_to_string(root.join(name)).unwrap();
    let auto = compile(&src, name, &CompileOptions::default());
    let fixed = compile(
      &src,
      name,
      &CompileOptions {
        host: HostOption::Fixed(None),
        ..CompileOptions::default()
      },
    );

    assert!(auto.diagnostics.is_empty(), "{:#?}", auto.diagnostics);
    assert_eq!(auto.js, fixed.js);
    assert!(!auto.js.contains("$bind$") && !auto.js.contains("$ext$"));
  }
}

const PORTABLE: &str = "module Kv

uses
  Std.Option
  Std.List

hosts
  DOM
  Node

effects
  Kv {
    get(key: String) -> Option<String>
    set(key: String, value: String) -> {}
    keys() -> List<String>
  }

binds
  Kv in DOM = \"./dom.js\"

  Kv in Node {
    get(key) {
      Some(\"node:#{key}\")
    }

    set(key, value) {
      {}
    }

    keys() {
      [\"a\"]
    }
  }

functions
  main() {
    Kv.set(\"k\", \"v\")
    Log.info(Option.with_default(Kv.get(\"k\"), \"none\"))
    Log.info(Option.with_default(Kv.get(\"absent\"), \"none\"))
    Log.info(List.join(Kv.keys(), \",\"))
  }

exports
  main
";

const KV_JS: &str = "const data = new Map();

export function get(key) {
  return data.get(key);
}

export function set(key, value) {
  data.set(key, value);
}

export function keys() {
  return [...data.keys(), \"b\"];
}
";

#[test]
fn hostless_effect_runs_where_bound() {
  let db = PORTABLE.replace("\n  DOM\n  Node\n", "\n  DOM\n  Node\n  Edge\n");
  let out = compile_for(&db, HostOption::Flag("Edge".into()), Vec::new());

  assert_eq!(codes_for_host(PORTABLE, "Node"), Vec::<String>::new());
  assert_eq!(
    out.diagnostics[0].message,
    "`main` can't run on `Edge`: it uses `Kv`, which runs on {DOM, Node}"
  );
}

#[test]
fn native_effect_needs_a_host() {
  let src = PORTABLE.replace("  Kv {", "  native Kv {");

  assert!(codes_for_host(&src, "Node").contains(&"POLAR0817".to_string()));
}

#[test]
fn force_on_hostless_effect_is_needless() {
  let src = PORTABLE.replace("  Kv in Node {", "  force Kv in Node {");

  assert_eq!(codes_for_host(&src, "Node"), vec!["POLAR0806".to_string()]);
}

#[test]
fn module_bind_imports_every_operation() {
  let js = emit_for_host(PORTABLE, "DOM");

  assert!(
    js.contains("import { get as $js$Kv$DOM$get, set as $js$Kv$DOM$set, keys as $js$Kv$DOM$keys } from \"./dom.js\""),
    "{js}"
  );
  assert!(
    js.contains("$rt.extern($js$Kv$DOM$get, [null], [\"option\", null])"),
    "{js}"
  );
  assert!(
    js.contains("$rt.extern($js$Kv$DOM$set, [null, null], \"unit\")"),
    "{js}"
  );
  assert!(
    js.contains("$rt.extern($js$Kv$DOM$keys, [], [\"list\", null])"),
    "{js}"
  );
  assert!(!emit_for_host(PORTABLE, "Node").contains("dom.js"));
}

#[test]
fn module_bind_converts_at_the_boundary() {
  assert_eq!(run(PORTABLE, "DOM", &[], &[("dom.js", KV_JS)]), "v\nnone\nk,b\n");
  assert_eq!(run(PORTABLE, "Node", &[], &[]), "node:k\nnode:absent\na\n");
}

#[test]
fn extern_returning_option_is_converted() {
  let src = "module Env

uses
  Std.Option

hosts
  Node

effects
  Env {
    read(name: String) -> Option<String>
  }

externs
  lookup(name: String) -> Option<String> = \"./env.js\" lookup

binds
  Env in Node {
    read(name) {
      lookup(name)
    }
  }

functions
  main() {
    Log.info(Option.with_default(Env.read(\"HOME\"), \"unset\"))
    Log.info(Option.with_default(Env.read(\"NOPE\"), \"unset\"))
  }

exports
  main
";
  let env = "export function lookup(name) {\n  return name === \"HOME\" ? \"/home\" : undefined;\n}\n";

  assert_eq!(run(src, "Node", &[], &[("env.js", env)]), "/home\nunset\n");
}

#[test]
fn module_bind_of_an_imported_effect() {
  let kv = "module Kv\n\nuses\n  Std.Option\n\neffects\n  Kv {\n    get(key: String) -> Option<String>\n  }\n\nexports\n  Kv\n";
  let main = "module Main\n\nuses\n  Std.Option\n  Kv\n\nhosts\n  Node\n\nbinds\n  Kv in Node = \"./kv.js\"\n\nfunctions\n  main() {\n    Log.info(Option.with_default(Kv.get(\"k\"), \"none\"))\n  }\n\nexports\n  main\n";
  let modules = vec![ModuleSource {
    path: "Kv".to_string(),
    source: kv.to_string(),
    specifier: "./kv_module.js".to_string(),
    plugins: Vec::new(),
  }];
  let js = "export function get(key) {\n  return key === \"k\" ? \"found\" : null;\n}\n";

  assert_eq!(run(main, "Node", &modules, &[("kv.js", js)]), "found\n");
}
