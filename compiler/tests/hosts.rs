use std::{collections::BTreeSet, sync::Arc};

use polar_compiler::{
  CompileOptions, Stage,
  check::hosts::{effect_hosts, intersect, row_hosts, show},
  core::effects::{BindInfo, EffectInfo, EffectTable, HostInfo},
  dump_stage_with,
  shared::diagnostic::{Diagnostic, Severity},
  shared::modules::ModuleSource,
  shared::source::{SourceFile, Span},
  types::ty::{EffTail, Effects, Label},
};

const PREAMBLE: &str = "module App

uses
  Std.List

hosts
  DOM
  Node

types
  Missing = Missing(String)

effects
  Storage in DOM {
    get(key: String) -> String
  }

  Db in Node {
    load(id: Int) -> String
  }

  native Canvas in DOM {
    draw(s: String) -> {}
  }
";

const STORAGE_IN_NODE: &str =
  "  Storage in Node {\n    get(key) {\n      key\n    }\n  }\n";

fn with_hosts(fns: &str) -> String {
  format!("{PREAMBLE}\nbinds\n{STORAGE_IN_NODE}\nfunctions\n  {fns}\n")
}

fn with_binds(binds: &str) -> String {
  format!("{PREAMBLE}\nbinds\n{binds}")
}

fn stage(
  src: &str,
  stage: Stage,
  modules: Vec<ModuleSource>,
) -> (Option<String>, Vec<Diagnostic>) {
  let options = CompileOptions { modules, ..CompileOptions::default() };
  let result = dump_stage_with(src, "app.px", stage, &options);

  (result.output, result.diagnostics)
}

fn hosts_output(src: &str) -> String {
  let (output, diagnostics) = stage(src, Stage::Hosts, Vec::new());

  output.unwrap_or_else(|| panic!("{diagnostics:#?}"))
}

fn hosts_of(src: &str, name: &str) -> String {
  let all = hosts_output(src);

  all
    .lines()
    .find_map(|l| {
      let (n, hosts) = l.split_once(" : ")?;

      (n.trim() == name).then(|| hosts.to_string())
    })
    .unwrap_or_else(|| panic!("no `{name}` in {all}"))
}

fn errors(src: &str) -> Vec<Diagnostic> {
  stage(src, Stage::Types, Vec::new())
    .1
    .into_iter()
    .filter(|d| d.severity == Severity::Error)
    .collect()
}

fn only(src: &str) -> Diagnostic {
  let found = errors(src);

  assert_eq!(found.len(), 1, "{found:#?}");
  found.into_iter().next().unwrap()
}

fn slice(src: &str, span: &Span) -> String {
  SourceFile::new("app.px", src).slice(span).to_string()
}

fn span() -> Span {
  Span::empty(Arc::from("t.px"), 0)
}

fn table() -> EffectTable {
  let mut table = EffectTable::default();

  for host in ["DOM", "Node"] {
    table.hosts.insert(
      host.to_string(),
      HostInfo { local: true, exported: false, span: span() },
    );
  }

  for (name, host) in [("Storage", "DOM"), ("Db", "Node")] {
    table.effects.insert(
      name.to_string(),
      EffectInfo {
        native: false,
        host: Some(host.to_string()),
        ops: Vec::new(),
        params: Vec::new(),
        local: true,
        exported: false,
        span: span(),
        sigs: Vec::new(),
        schemes: Vec::new(),
        module: None,
      },
    );
  }

  table.binds.push(BindInfo {
    effect: "Storage".to_string(),
    host: "Node".to_string(),
    via: None,
    module: None,
    span: None,
  });

  table
}

fn set(hosts: &[&str]) -> BTreeSet<String> {
  hosts.iter().map(|h| (*h).to_string()).collect()
}

#[test]
fn set_functions() {
  let table = table();
  let row = |labels: Vec<Label>| Effects::new(labels, EffTail::Closed);

  assert_eq!(effect_hosts(&table, "Storage"), Some(set(&["DOM", "Node"])));
  assert_eq!(effect_hosts(&table, "Db"), Some(set(&["Node"])));
  assert_eq!(row_hosts(&table, &row(vec![])), None);
  assert_eq!(row_hosts(&table, &row(vec![Label::throws("App.Missing")])), None);
  assert_eq!(
    row_hosts(
      &table,
      &row(vec![Label::effect("Storage"), Label::effect("Db")])
    ),
    Some(set(&["Node"]))
  );
  assert_eq!(intersect(&None, &Some(set(&["DOM"]))), Some(set(&["DOM"])));
  assert_eq!(
    intersect(&Some(set(&["DOM"])), &Some(set(&["Node"]))),
    Some(set(&[]))
  );
  assert_eq!(show(&None), "every host");
  assert_eq!(show(&Some(set(&["Node", "DOM"]))), "{DOM, Node}");
}

#[test]
fn pure_everywhere() {
  assert_eq!(
    hosts_of(&with_hosts("slug(s: String) -> String { s }"), "slug"),
    "every host"
  );
}

#[test]
fn bound_effect() {
  assert_eq!(
    hosts_of(&with_hosts("f() { Storage.get(\"k\") }"), "f"),
    "{DOM, Node}"
  );
}

#[test]
fn intersection() {
  let src = with_hosts("save(id) {\n    Db.load(id)\n    Log.info(\"x\")\n  }");

  assert_eq!(hosts_of(&src, "save"), "{Node}");
}

#[test]
fn native_only() {
  assert_eq!(hosts_of(&with_hosts("f() { Canvas.draw(\"x\") }"), "f"), "{DOM}");
}

#[test]
fn storage_and_canvas() {
  let src =
    with_hosts("f() {\n    Storage.get(\"k\")\n    Canvas.draw(\"x\")\n  }");

  assert_eq!(hosts_of(&src, "f"), "{DOM}");
}

#[test]
fn throws_is_everywhere() {
  assert_eq!(
    hosts_of(&with_hosts("f() { throw Missing(\"x\") }"), "f"),
    "every host"
  );
}

#[test]
fn no_host() {
  let src = with_hosts(
    "broken(id: Int) -> {} / {Db, Canvas} {\n    Canvas.draw(Db.load(id))\n  }",
  );
  let d = only(&src);

  assert_eq!(d.code.to_string(), "POLAR0811");
  assert_eq!(d.message, "`broken` can't run anywhere");
  assert_eq!(slice(&src, &d.primary.span), "broken");
  assert_eq!(
    d.primary.message.as_deref(),
    Some("no host provides both `Db` and `Canvas`")
  );

  let secondary: Vec<(String, String)> = d
    .secondary
    .iter()
    .map(|l| (slice(&src, &l.span), l.message.clone().unwrap_or_default()))
    .collect();

  assert_eq!(
    secondary,
    vec![
      ("Db.load".to_string(), "`Db` runs on {Node}".to_string()),
      ("Canvas.draw".to_string(), "`Canvas` runs on {DOM}".to_string()),
    ]
  );
}

#[test]
fn no_host_declared_unused() {
  let src = with_hosts("f() -> {} / {Db, Canvas} { {} }");
  let d = only(&src);

  assert_eq!(d.code.to_string(), "POLAR0811");
  assert!(d.secondary.iter().all(|l| slice(&src, &l.span) == "/ {Db, Canvas}"));
}

#[test]
fn no_host_lambda() {
  let src = with_hosts(
    "f() { function() {\n    Db.load(1)\n    Canvas.draw(\"x\")\n  } }",
  );
  let d = only(&src);

  assert_eq!(d.message, "this function can't run anywhere");
  assert_eq!(slice(&src, &d.primary.span), "function");
}

#[test]
fn no_host_three_way() {
  let src = "module App

hosts
  X
  Y
  Z

effects
  A in X {
    a() -> {}
  }

  B in Y {
    b() -> {}
  }

  C in Z {
    c() -> {}
  }

binds
  A in Y {
    a() {
      {}
    }
  }

  B in Z {
    b() {
      {}
    }
  }

  C in X {
    c() {
      {}
    }
  }

functions
  f() {
    A.a()
    B.b()
    C.c()
  }
";
  let d = only(src);

  assert_eq!(d.code.to_string(), "POLAR0811");
  assert_eq!(
    d.primary.message.as_deref(),
    Some("no host provides all of `A`, `B` and `C`")
  );
  assert_eq!(d.secondary.len(), 3);
}

#[test]
fn polymorphic_ok() {
  let src = with_hosts(
    "each(xs: List<a>, f: function(a) -> {} / {| e}) -> {} / {| e} { {} }",
  );

  assert_eq!(hosts_of(&src, "each"), "every host");
}

#[test]
fn polymorphic_filled() {
  let src = with_hosts(
    "each(xs: List<a>, f: function(a) -> {} / {| e}) -> {} / {| e} { {} }\n\n  f() {\n    each([1], function(x) {\n      Db.load(x)\n      {}\n    })\n    Canvas.draw(\"x\")\n  }",
  );
  let d = only(&src);

  assert_eq!(d.message, "`f` can't run anywhere");
}

#[test]
fn bind_uses_other_ok() {
  let src = with_binds(
    "  Storage in Node {\n    get(key) {\n      Db.load(1)\n    }\n  }\n",
  );

  assert!(errors(&src).is_empty(), "{:#?}", errors(&src));
}

#[test]
fn bind_uses_wrong_host() {
  let src = with_binds(
    "  Storage in Node {\n    get(key) {\n      Canvas.draw(key)\n      key\n    }\n  }\n",
  );
  let d = only(&src);

  assert_eq!(d.code.to_string(), "POLAR0813");
  assert_eq!(
    d.message,
    "the binding of `Storage` in `Node` uses `Canvas`, which isn't available on `Node`"
  );
}

#[test]
fn bind_self() {
  let src = with_binds(
    "  Storage in Node {\n    get(key) {\n      Storage.get(key)\n    }\n  }\n",
  );
  let d = only(&src);

  assert_eq!(d.code.to_string(), "POLAR0812");
  assert_eq!(
    d.message,
    "the binding of `Storage` in `Node` calls `Storage` itself"
  );
}

#[test]
fn bind_cycle() {
  let src = format!(
    "{}\n  Cache in Node {{\n    peek(key: String) -> String\n  }}\n\nbinds\n  Storage in Node {{\n    get(key) {{\n      Cache.peek(key)\n    }}\n  }}\n\n  Cache in Node {{\n    peek(key) {{\n      Storage.get(key)\n    }}\n  }}\n",
    PREAMBLE.trim_end()
  );
  let found = errors(&src);

  assert_eq!(found.len(), 1, "{found:#?}");
  assert_eq!(found[0].code.to_string(), "POLAR0812");
  assert!(
    found[0].message.contains("`Storage` → `Cache` → `Storage`")
      || found[0].message.contains("`Cache` → `Storage` → `Cache`"),
    "{}",
    found[0].message
  );
}

fn source(path: &str, src: &str) -> ModuleSource {
  ModuleSource {
    path: path.to_string(),
    source: src.to_string(),
    specifier: format!("./{}.js", path.to_lowercase()),
    plugins: Vec::new(),
  }
}

#[test]
fn imported_bind_counts() {
  let a = "module A\n\nhosts\n  Node\n\neffects\n  Db in Node {\n    load(id: Int) -> String\n  }\n\nexports\n  Node\n  Db\n";
  let b = "module B\n\nuses\n  A\n\nhosts\n  Worker\n\nbinds\n  Db in Worker {\n    load(id) {\n      \"worker\"\n    }\n  }\n\nexports\n  Worker\n";
  let c = "module C\n\nuses\n  A\n  B\n\nfunctions\n  title(id) {\n    Db.load(id)\n  }\n";
  let (output, diagnostics) =
    stage(c, Stage::Hosts, vec![source("A", a), source("B", b)]);
  let output = output.unwrap_or_else(|| panic!("{diagnostics:#?}"));

  assert_eq!(output, "title : {Node, Worker}\n");
}

#[test]
fn emit_hosts_output() {
  let src = with_hosts(
    "slug(r: { title: String | rest }) -> String {\n    r.title\n  }\n\n  read_draft(id: Int) -> String / {Storage} {\n    Storage.get(\"draft\")\n  }\n\n  save_post(id: Int) -> {} / {Db} {\n    Db.load(id)\n    Log.info(\"saved\")\n  }\n\n  render(id: Int) -> {} / {Canvas, Storage} {\n    Canvas.draw(Storage.get(\"x\"))\n  }",
  );

  assert_eq!(
    hosts_output(&src),
    "read_draft : {DOM, Node}\nrender     : {DOM}\nsave_post  : {Node}\nslug       : every host\n"
  );
}

#[test]
fn roadmap_exit() {
  let example = std::fs::read_to_string(
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
      .join("tests/fixtures/programs/effects.px"),
  )
  .unwrap();

  assert!(errors(&example).is_empty(), "{:#?}", errors(&example));

  let broken = "\n  broken(id: Int) -> {} / {Db, Canvas} {\n    Canvas.draw(Db.find(id))\n  }\n";
  let declarations = &example[..example.find("externs\n").unwrap()];
  let copy = format!("{declarations}functions{broken}");
  let d = only(&copy);
  let labels: Vec<String> =
    d.secondary.iter().filter_map(|l| l.message.clone()).collect();

  assert_eq!(d.code.to_string(), "POLAR0811");
  assert_eq!(
    labels,
    vec![
      "`Db` runs on {Node}".to_string(),
      "`Canvas` runs on {DOM}".to_string()
    ]
  );

  let forced =
    example.replacen("functions\n", &format!("functions{broken}\n"), 1);

  assert!(errors(&forced).is_empty(), "{:#?}", errors(&forced));
  assert_eq!(hosts_of(&forced, "broken"), "{Node}");
}
