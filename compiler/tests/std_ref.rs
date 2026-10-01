mod common;

use polar_compiler::{
  CompileOptions, Stage, compile, dump_stage, fmt::format,
  shared::source::SourceFile,
};

fn module(uses: &str, rest: &str) -> String {
  format!("module M\n\nuses\n{uses}\n{rest}\nexports\n  main\n")
}

fn run(src: &str) -> String {
  common::node::run_program(src, "test.px", &[])
    .unwrap_or_else(|err| panic!("{err}"))
}

fn errors_of(src: &str) -> Vec<String> {
  let file = SourceFile::new("test.px", src);

  dump_stage(src, "test.px", Stage::Types)
    .diagnostics
    .iter()
    .map(|d| format!("{} at {:?}", d.code, file.slice(&d.primary.span)))
    .collect()
}

fn js(src: &str) -> String {
  let out = compile(src, "test.px", &CompileOptions::default());

  assert!(out.diagnostics.is_empty(), "{:#?}", out.diagnostics);
  out.js
}

const COUNTER: &str = "constants
  visits: Ref<Int> = Ref.new(0)

functions
  visit() -> Int / {Mut} {
    Ref.update(visits, function(n) { n + 1 })
    Ref.get(visits)
  }

  main() {
    visit()
    visit()
    Log.info(\"visits: #{visit()}\")
  }
";

#[test]
fn counter_in_a_constant() {
  let out = run(&module("  Std.Ref", COUNTER));

  assert_eq!(out.trim(), "visits: 3");
}

#[test]
fn mut_does_not_make_code_async() {
  let out = js(&module("  Std.Ref", COUNTER));

  assert!(!out.contains("async"), "{out}");
  assert!(!out.contains("await"), "{out}");
}

#[test]
fn reserved_names_are_escaped_on_import() {
  let out = js(&module("  Std.Ref", COUNTER));

  assert!(out.contains("$Ref.new$(0)"), "{out}");
}

#[test]
fn set_get_and_modify() {
  let src = module(
    "  Std.Ref",
    "functions
  main() {
    let cell = Ref.new(\"a\")

    Ref.set(cell, \"b\")

    let old = Ref.modify(cell, function(s) { { value: \"#{s}c\", result: s } })

    Log.info(\"#{old} #{Ref.get(cell)} #{cell}\")
  }
",
  );

  assert_eq!(run(&src).trim(), "b bc <ref>");
}

#[test]
fn reading_needs_mut_in_the_row() {
  let src = module(
    "  Std.Ref",
    "constants
  cell: Ref<Int> = Ref.new(0)

functions
  peek() -> Int {
    Ref.get(cell)
  }

  main() {
    Log.info(\"\")
  }
",
  );

  assert_eq!(
    errors_of(&src),
    vec!["POLAR0808 at \"Ref.get(cell)\"".to_string()]
  );
}

#[test]
fn refs_have_no_eq() {
  let src = module(
    "  Std.Ref",
    "functions
  main() {
    Log.info(\"#{Ref.new(1) == Ref.new(1)}\")
  }
",
  );

  assert_eq!(errors_of(&src), vec!["POLAR0707 at \"==\"".to_string()]);
}

#[test]
fn update_takes_a_pure_function() {
  let src = module(
    "  Std.Ref",
    "constants
  cell: Ref<Int> = Ref.new(0)

functions
  main() {
    Ref.update(cell, function(n) { n + Ref.get(cell) })
  }
",
  );

  assert!(!errors_of(&src).is_empty());
}

#[test]
fn mut_runs_on_every_host() {
  let src = "module M

uses
  Std.Ref

hosts
  Browser
  Node

effects
  Db in Node {
    load() -> Int
  }

binds
  Db in Node {
    load() {
      1
    }
  }

functions
  both() -> Int / {Db, Mut} {
    Db.load() + Ref.get(Ref.new(1))
  }

  bump() -> Int / {Mut} {
    Ref.get(Ref.new(1))
  }
";
  let hosts = dump_stage(src, "test.px", Stage::Hosts);

  assert!(hosts.diagnostics.is_empty(), "{:#?}", hosts.diagnostics);

  let out = hosts.output.unwrap();

  assert!(out.contains("both : {Node}"), "{out}");
  assert!(out.contains("bump : every host"), "{out}");
}

#[test]
fn table_round_trip() {
  let src = module(
    "  Std.List\n  Std.Option\n  Std.Table",
    "types
  Note = { id: Int, text: String }

constants
  notes: Table<Note> = Table.new()

functions
  main() {
    let a = Table.insert(notes, function(n) { { id: n, text: \"a\" } })
    let b = Table.insert(notes, function(n) { { id: n, text: \"b\" } })
    let updated = Table.update(notes, b.id, { ..b, text: \"B\" })
    let deleted = Table.delete(notes, a.id)
    let again = Table.delete(notes, a.id)
    let texts = List.map(Table.all(notes), function(note) { note.text })

    Log.info(\"#{updated} #{deleted} #{again} #{Table.size(notes)} #{List.join(texts, \",\")}\")
    Log.info(Option.with_default(Option.map(Table.find(notes, b.id), function(n) { n.text }), \"none\"))
  }
",
  );

  assert_eq!(run(&src).trim(), "true true false 1 B\nB");
}

#[test]
fn opaque_types_parse_and_format() {
  let src = "module M\n\ntypes\n  Handle\n\n  Box<a>\n\n  Pair = { a: Int }\n";
  let out = format(src, "test.px").output.unwrap();

  assert_eq!(out, src);
  assert!(errors_of(src).is_empty(), "{:?}", errors_of(src));
}

#[test]
fn opaque_types_have_no_constructors() {
  let src =
    "module M\n\ntypes\n  Handle\n\nfunctions\n  make() {\n    Handle\n  }\n";

  assert!(!errors_of(src).is_empty());
}
