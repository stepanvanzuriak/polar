mod common;

use std::{path::PathBuf, sync::Arc};

use common::invariants::check_invariants;
use polar_compiler::{
  Stage, dump_stage, format,
  shared::codes::DiagnosticCode,
  shared::diagnostic::{Diagnostic, DiagnosticBag},
  shared::source::{SourceFile, Span},
  syntax::ast::{
    BindDecl, Block, EffectDecl, Expr, HostDecl, IntLit, MatchArm, Module,
    Name, PCtor, PVar, Pattern, Throw, Try, Var, ZoneKind,
    dump::{dump_ast_shape, dump_node_shape},
    eq::ast_eq,
    fields::AsNode,
  },
  syntax::lexer::{lex, token::TokenKind},
  syntax::parser::parse,
};

fn sp(start: usize, end: usize) -> Span {
  Span::new(Arc::from("test.px"), start, end)
}

fn name(text: &str, start: usize, end: usize) -> Name {
  Name { text: text.to_string(), span: sp(start, end) }
}

fn parse_module(src: &str) -> (Module, Vec<Diagnostic>) {
  let file = SourceFile::new("test.px", src);
  let mut bag = DiagnosticBag::default();
  let lexed = lex(&file, &mut bag);
  let module = parse(&file, &lexed, &mut bag);

  (module, bag.into_sorted())
}

fn parse_ok(src: &str) -> Module {
  let (module, diagnostics) = parse_module(src);

  assert!(diagnostics.is_empty(), "{diagnostics:#?}");
  check_invariants(src, &module);
  module
}

fn ast_shape(src: &str) -> String {
  dump_ast_shape(&parse_ok(src))
}

fn errors(src: &str) -> Vec<(String, String, String)> {
  let (_, diagnostics) = parse_module(src);

  diagnostics
    .iter()
    .map(|d| {
      let span = &d.primary.span;

      (
        d.code.to_string(),
        src[span.start..span.end].to_string(),
        d.message.clone(),
      )
    })
    .collect()
}

fn codes(src: &str) -> Vec<String> {
  errors(src).into_iter().map(|(code, _, _)| code).collect()
}

fn formatted(src: &str) -> String {
  let result = format(src, "test.px");

  result.output.unwrap_or_else(|| panic!("{:#?}", result.diagnostics))
}

fn assert_stable(src: &str) {
  let once = formatted(src);

  assert_eq!(formatted(&once), once, "formatting is not idempotent");
}

const README: &str = r#"module Notes

hosts
  DOM
  Node

types
  Missing = Missing(String)

effects
  Storage in DOM {
    get(key: String) -> String / {Throws<Missing>}
    set(key: String, value: String) -> {}
  }

  native Canvas in DOM {
    draw(label: String) -> {}
  }

externs
  local_get(key: String) -> String = "./dom.js" get_item
  local_set(key: String, value: String) -> {} = "./dom.js" set_item

binds
  Storage in DOM {
    get(key) {
      local_get(key)
    }

    set(key, value) {
      local_set(key, value)
    }
  }

  force Canvas in Node {
    draw(label) {
      Log.info("canvas: #{label}")
    }
  }

functions
  load(key: String) -> String / {Storage} {
    try {
      Storage.get(key)
    } catch {
      Missing(k) -> "no #{k}",
    }
  }

  require(key: String) -> String / {Storage, Throws<Missing>} {
    let value = Storage.get(key)

    if value == "" { throw Missing(key) } else { value }
  }
"#;

#[test]
fn keywords() {
  let src = "hosts effects externs binds in native force throw try catch host effect bind extern";
  let file = SourceFile::new("test.px", src);
  let mut bag = DiagnosticBag::default();
  let tokens = lex(&file, &mut bag).tokens;
  let kinds: Vec<TokenKind> = tokens.iter().map(|t| t.kind).collect();

  assert_eq!(
    kinds,
    vec![
      TokenKind::KwHosts,
      TokenKind::KwEffects,
      TokenKind::KwExterns,
      TokenKind::KwBinds,
      TokenKind::KwIn,
      TokenKind::KwNative,
      TokenKind::KwForce,
      TokenKind::KwThrow,
      TokenKind::KwTry,
      TokenKind::KwCatch,
      TokenKind::KwHost,
      TokenKind::KwEffect,
      TokenKind::KwBind,
      TokenKind::KwExtern,
      TokenKind::Eof,
    ]
  );
}

#[test]
fn zone_order() {
  assert!(ZoneKind::USES < ZoneKind::HOSTS);
  assert!(ZoneKind::HOSTS < ZoneKind::TRAITS);
  assert!(ZoneKind::CONSTANTS < ZoneKind::EFFECTS);
  assert!(ZoneKind::EFFECTS < ZoneKind::EXTERNS);
  assert!(ZoneKind::EXTERNS < ZoneKind::BINDS);
  assert!(ZoneKind::BINDS < ZoneKind::FUNCTIONS);
}

#[test]
fn dump_host() {
  let decl = HostDecl { span: sp(0, 3), docs: vec![], name: name("DOM", 0, 3) };

  assert_eq!(dump_node_shape(decl.as_node()), "(HostDecl DOM)\n");
}

#[test]
fn dump_native_effect() {
  let decl = EffectDecl {
    span: sp(0, 30),
    docs: vec![],
    native: true,
    name: name("Canvas", 7, 13),
    host: Some(name("DOM", 17, 20)),
    ops: vec![],
  };

  assert_eq!(
    dump_node_shape(decl.as_node()),
    "(EffectDecl native=true Canvas DOM)\n"
  );
}

#[test]
fn dump_force_bind() {
  let decl = BindDecl {
    span: sp(0, 30),
    docs: vec![],
    force: true,
    effect: name("Canvas", 6, 12),
    host: name("Node", 16, 20),
    from: None,
    module: None,
    ops: vec![],
  };

  assert_eq!(
    dump_node_shape(decl.as_node()),
    "(BindDecl force=true Canvas Node)\n"
  );
}

fn var(text: &str, at: usize) -> Expr {
  Expr::Var(Var {
    span: sp(at, at + text.len()),
    name: name(text, at, at + text.len()),
  })
}

fn sample_try(offset: usize) -> Try {
  let body = Block {
    span: sp(offset + 4, offset + 9),
    stmts: vec![],
    result: Box::new(var("g", offset + 6)),
  };
  let arm = MatchArm {
    span: sp(offset + 18, offset + 33),
    rows: vec![vec![Pattern::Ctor(PCtor {
      span: sp(offset + 18, offset + 28),
      name: name("Missing", offset + 18, offset + 25),
      args: vec![Pattern::Var(PVar {
        span: sp(offset + 26, offset + 27),
        name: name("k", offset + 26, offset + 27),
      })],
    })]],
    guard: None,
    body: var("k", offset + 32),
  };

  Try {
    span: sp(offset, offset + 35),
    body: Box::new(body),
    catch_span: sp(offset + 10, offset + 15),
    arms: vec![arm],
  }
}

#[test]
fn dump_try() {
  assert_eq!(
    dump_node_shape(sample_try(0).as_node()),
    "(Try\n  (Block\n    (Var g))\n  (MatchArm\n    (PCtor Missing\n      (PVar k))\n    (Var k)))\n"
  );
}

#[test]
fn eq_ignores_spans() {
  let throw = |at: usize| {
    let value = Expr::Int(IntLit { span: sp(at + 6, at + 7), raw: "1".into() });

    Throw { span: sp(at, at + 7), value: Box::new(value) }
  };
  let wrap = |t: Throw| Module {
    span: sp(0, 100),
    name: None,
    zones: vec![polar_compiler::syntax::ast::Zone {
      span: sp(0, 100),
      kind: ZoneKind::CONSTANTS,
      decls: vec![polar_compiler::syntax::ast::Decl::Const(
        polar_compiler::syntax::ast::ConstDecl {
          span: sp(0, 100),
          docs: vec![],
          name: name("x", 0, 1),
          ty: None,
          value: Expr::Throw(t),
        },
      )],
    }],
  };

  assert!(ast_eq(&wrap(throw(0)), &wrap(throw(40))));
}

#[test]
fn parses_hosts() {
  assert_eq!(
    ast_shape("module M\n\nhosts\n  DOM\n  Node\n"),
    "(Module M\n  (Zone hosts\n    (HostDecl DOM)\n    (HostDecl Node)))\n"
  );
}

#[test]
fn parses_effect() {
  assert_eq!(
    ast_shape(
      "module M\n\neffects\n  Storage in DOM { get(key: String) -> String / {Throws<Missing>} }\n"
    ),
    "(Module M\n  (Zone effects\n    (EffectDecl native=false Storage DOM\n      (MethodSig get\n        (Param key\n          (TypeRef String))\n        (TypeRef String)\n        (EffectRow\n          (TypeRef Throws\n            (TypeRef Missing)))))))\n"
  );
}

#[test]
fn parses_native_effect() {
  let module = parse_ok(
    "module M\n\neffects\n  native Canvas in DOM { draw(s: String) -> {} }\n",
  );
  let polar_compiler::syntax::ast::Decl::Effect(effect) =
    &module.zones[0].decls[0]
  else {
    panic!("an effect");
  };

  assert!(effect.native);
  assert_eq!(effect.name.text, "Canvas");
  assert_eq!(effect.host.as_ref().unwrap().text, "DOM");
}

#[test]
fn parses_hostless_effect() {
  assert_eq!(
    ast_shape(
      "module M\n\neffects\n  Storage { get(key: String) -> String }\n"
    ),
    "(Module M\n  (Zone effects\n    (EffectDecl native=false Storage\n      (MethodSig get\n        (Param key\n          (TypeRef String))\n        (TypeRef String)))))\n"
  );
}

#[test]
fn effect_op_needs_types() {
  let found =
    errors("module M\n\neffects\n  Storage in DOM { get(key) -> String }\n");

  assert_eq!(found.len(), 1, "{found:#?}");
  assert_eq!(found[0].0, "POLAR0223");
  assert_eq!(found[0].1, "key");
}

#[test]
fn parses_bind() {
  assert_eq!(
    ast_shape("module M\n\nbinds\n  Storage in Node { get(key) { key } }\n"),
    "(Module M\n  (Zone binds\n    (BindDecl force=false Storage Node\n      (FnDecl get\n        (Param key)\n        (Block\n          (Var key))))))\n"
  );
}

#[test]
fn parses_force_bind() {
  let module =
    parse_ok("module M\n\nbinds\n  force Canvas in Node { draw(s) { {} } }\n");
  let polar_compiler::syntax::ast::Decl::Bind(bind) = &module.zones[0].decls[0]
  else {
    panic!("a bind");
  };

  assert!(bind.force);
  assert_eq!(bind.ops.len(), 1);
}

#[test]
fn parses_module_bind() {
  let module = parse_ok(
    "module M\n\nbinds\n  Storage in Node = \"./node.js\"\n  Db in Node { find(id) { id } }\n",
  );
  let polar_compiler::syntax::ast::Decl::Bind(bind) = &module.zones[0].decls[0]
  else {
    panic!("a bind");
  };

  assert_eq!(bind.effect.text, "Storage");
  assert!(bind.ops.is_empty());
  assert!(bind.module.is_some());
  assert_eq!(module.zones[0].decls.len(), 2);
}

#[test]
fn parses_bridge() {
  assert_eq!(
    ast_shape("module M\n\nbinds\n  Posts in Browser from Node\n"),
    "(Module M\n  (Zone binds\n    (BindDecl force=false Posts Browser from=Node)))\n"
  );
}

#[test]
fn parses_bridges_in_sequence() {
  let module = parse_ok(
    "module M\n\nbinds\n  Posts in Browser from Node\n  Db in Node = \"./db.js\"\n  Clock in Browser from Node\n",
  );
  let binds: Vec<(String, Option<String>)> = module.zones[0]
    .decls
    .iter()
    .map(|d| match d {
      polar_compiler::syntax::ast::Decl::Bind(b) => {
        (b.effect.text.clone(), b.from.as_ref().map(|v| v.text.clone()))
      }
      _ => panic!("a bind"),
    })
    .collect();

  assert_eq!(
    binds,
    vec![
      ("Posts".to_string(), Some("Node".to_string())),
      ("Db".to_string(), None),
      ("Clock".to_string(), Some("Node".to_string())),
    ]
  );
}

#[test]
fn bridge_with_module() {
  let found =
    errors("module M\n\nbinds\n  Posts in Browser from Node = \"./p.js\"\n");

  assert_eq!(found.len(), 1, "{found:#?}");
  assert_eq!(found[0].0, "POLAR0226");
  assert!(found[0].2.contains("can't have a body"), "{}", found[0].2);
}

#[test]
fn bridge_with_body() {
  assert_eq!(
    codes(
      "module M\n\nbinds\n  Posts in Browser from Node { fetch(id) { id } }\n  Db in Node = \"./db.js\"\n"
    ),
    vec!["POLAR0226"]
  );
}

#[test]
fn bridge_needs_a_target() {
  assert_eq!(
    codes("module M\n\nbinds\n  Posts in Browser from\n"),
    vec!["POLAR0201"]
  );
}

#[test]
fn bridge_outside_binds() {
  assert_eq!(
    codes("module M\n\neffects\n  Posts in Browser from Node\n"),
    vec!["POLAR0207"]
  );
}

#[test]
fn module_bind_needs_a_module() {
  assert_eq!(
    codes("module M\n\nbinds\n  Storage in Node = node\n"),
    vec!["POLAR0225"]
  );
}

#[test]
fn parses_extern() {
  let src = "module M\n\nexterns\n  get(key: String) -> String = \"./dom.js\" get_item\n";

  assert_eq!(
    ast_shape(src),
    "(Module M\n  (Zone externs\n    (ExternDecl get get_item\n      (Param key\n        (TypeRef String))\n      (TypeRef String)\n      (StringLit\n        (StringText raw=\"./dom.js\" value=\"./dom.js\")))))\n"
  );
}

#[test]
fn extern_needs_target() {
  assert_eq!(
    codes("module M\n\nexterns\n  get(key: String) -> String\n"),
    vec!["POLAR0225"]
  );
}

#[test]
fn extern_no_interpolation() {
  assert_eq!(
    codes("module M\n\nexterns\n  get(k: String) -> String = \"#{x}.js\" f\n"),
    vec!["POLAR0225"]
  );
}

#[test]
fn extern_upper_export() {
  let module =
    parse_ok("module M\n\nexterns\n  make() -> Int = \"./m.js\" Default\n");
  let polar_compiler::syntax::ast::Decl::Extern(e) = &module.zones[0].decls[0]
  else {
    panic!("an extern");
  };

  assert_eq!(e.export.text, "Default");
}

#[test]
fn force_on_effect() {
  let found = errors("module M\n\neffects\n  force Canvas in DOM { }\n");

  assert_eq!(found.len(), 1, "{found:#?}");
  assert_eq!(found[0].0, "POLAR0224");
  assert!(found[0].2.contains("`force` goes on a binding"), "{}", found[0].2);
}

#[test]
fn native_on_bind() {
  let found = errors("module M\n\nbinds\n  native Canvas in Node { }\n");

  assert_eq!(found.len(), 1, "{found:#?}");
  assert_eq!(found[0].0, "POLAR0224");
  assert!(found[0].2.contains("`native` goes on an effect"), "{}", found[0].2);
}

#[test]
fn redundant_effect_keyword() {
  let src = "module M\n\neffects\n  effect Storage in DOM { }\n";
  let (module, diagnostics) = parse_module(src);

  assert_eq!(
    diagnostics.iter().map(|d| d.code).collect::<Vec<_>>(),
    vec![DiagnosticCode::RedundantDeclarationKeyword]
  );
  assert!(matches!(
    module.zones[0].decls.as_slice(),
    [polar_compiler::syntax::ast::Decl::Effect(_)]
  ));
}

#[test]
fn zone_order_new() {
  assert_eq!(
    codes("module M\n\nbinds\n  S in N { }\n\neffects\n  S in N { }\n"),
    vec!["POLAR0208"]
  );
}

#[test]
fn effect_in_functions_zone() {
  let found = errors("module M\n\nfunctions\n  Storage in DOM { }\n");

  assert_eq!(found.len(), 1, "{found:#?}");
  assert_eq!(found[0].0, "POLAR0207");
  assert!(
    found[0].2.contains("belongs in the `effects` zone"),
    "{}",
    found[0].2
  );
}

#[test]
fn parses_throw() {
  assert_eq!(
    ast_shape("module M\n\nfunctions\n  f() { throw Missing(\"k\") }\n"),
    "(Module M\n  (Zone functions\n    (FnDecl f\n      (Block\n        (Throw\n          (Call\n            (Var Missing)\n            (StringLit\n              (StringText raw=\"k\" value=\"k\"))))))))\n"
  );
}

#[test]
fn throw_is_loose() {
  let shape = ast_shape("module M\n\nfunctions\n  f() { throw a + b }\n");

  assert!(shape.contains("(Throw\n          (Binary +"), "{shape}");
}

#[test]
fn parses_try() {
  let shape = ast_shape(
    "module M\n\nfunctions\n  f() { try { g() } catch { Missing(k) -> k, _ -> \"\" } }\n",
  );

  assert!(shape.contains("(Try\n"), "{shape}");
  assert_eq!(shape.matches("(MatchArm").count(), 2, "{shape}");
}

#[test]
fn try_needs_catch() {
  let found = errors("module M\n\nfunctions\n  f() { try { g() } }\n");

  assert!(
    found
      .iter()
      .any(|(_, _, m)| m
        .contains("expected keyword `catch` after the `try` block")),
    "{found:#?}"
  );
}

#[test]
fn empty_catch() {
  let found = codes("module M\n\nfunctions\n  f() { try { g() } catch { } }\n");

  assert_eq!(found, vec!["POLAR0215"]);
}

#[test]
fn readme_module_checks() {
  let result = dump_stage(README, "notes.px", Stage::Types);

  assert!(
    result.diagnostics.iter().all(
      |d| d.severity != polar_compiler::shared::diagnostic::Severity::Error
    ),
    "{:#?}",
    result.diagnostics
  );
}

#[test]
fn formats_effect() {
  assert_eq!(
    formatted(
      "module M\n\neffects\n  native  Canvas in DOM{draw(s:String)->{}}\n"
    ),
    "module M\n\neffects\n  native Canvas in DOM {\n    draw(s: String) -> {}\n  }\n"
  );
}

#[test]
fn formats_bind() {
  let src = "module M\n\nbinds\n  force   Canvas in Node{draw(s){ {} }}\n";

  assert_eq!(
    formatted(src),
    "module M\n\nbinds\n  force Canvas in Node {\n    draw(s) {\n      {}\n    }\n  }\n"
  );
}

#[test]
fn formats_hostless_effect() {
  let src = "module M\n\neffects\n  Storage{get(key:String)->String}\n";

  assert_eq!(
    formatted(src),
    "module M\n\neffects\n  Storage {\n    get(key: String) -> String\n  }\n"
  );
  assert_stable(src);
}

#[test]
fn formats_module_bind() {
  let src = "module M\n\nbinds\n  Storage   in Node   =   \"./node.js\"\n";

  assert_eq!(
    formatted(src),
    "module M\n\nbinds\n  Storage in Node = \"./node.js\"\n"
  );
  assert_stable(src);
}

#[test]
fn formats_bridge() {
  let src = "module M\n\nbinds\n  Posts   in Browser   from   Node\n  Db in Node = \"./db.js\"\n";

  assert_eq!(
    formatted(src),
    "module M\n\nbinds\n  Posts in Browser from Node\n\n  Db in Node = \"./db.js\"\n"
  );
  assert_stable(src);
}

#[test]
fn formats_long_extern() {
  let src = "module M\n\nexterns\n  local_get(key: String, fallback: String) -> String / {Throws<Missing>} = \"./dom.js\" get_item\n";

  assert_eq!(
    formatted(src),
    "module M\n\nexterns\n  local_get(key: String, fallback: String) -> String / {Throws<Missing>}\n    = \"./dom.js\" get_item\n"
  );
  assert_stable(src);
}

#[test]
fn formats_try() {
  assert_eq!(
    formatted("module M\n\nfunctions\n  f() { try{g()}catch{A->1,B->2} }\n"),
    "module M\n\nfunctions\n  f() {\n    try {\n      g()\n    } catch {\n      A -> 1,\n      B -> 2,\n    }\n  }\n"
  );
}

#[test]
fn keeps_comments() {
  let src = "module M\n\neffects\n  Storage in DOM {\n    // effect comment\n    get(key: String) -> String\n  }\n\nbinds\n  Storage in Node {\n    // bind comment\n    get(key) {\n      key\n    }\n  }\n\nfunctions\n  f() {\n    try {\n      // body comment\n      g()\n    } catch {\n      // arm comment\n      A -> 1,\n    }\n  }\n";
  let out = formatted(src);

  for comment in [
    "// effect comment",
    "// bind comment",
    "// body comment",
    "// arm comment",
  ] {
    assert!(out.contains(comment), "{comment} lost:\n{out}");
  }

  assert_eq!(out, src);
  assert_stable(src);
}

#[test]
fn throw_in_operand() {
  let src = "module M\n\nfunctions\n  f() { (throw x) + 1 }\n";
  let out = formatted(src);

  assert!(out.contains("(throw x) + 1"), "{out}");
  assert_stable(src);
}

#[test]
fn readme_module() {
  assert_eq!(formatted(README), README);
}

fn px_files(dir: &std::path::Path, out: &mut Vec<PathBuf>) {
  for entry in std::fs::read_dir(dir).expect("read a directory") {
    let path = entry.expect("a directory entry").path();

    if path.is_dir() {
      px_files(&path, out);
    } else if path.extension().is_some_and(|e| e == "px") {
      out.push(path);
    }
  }
}

#[test]
fn corpus_unchanged() {
  let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
  let mut files = Vec::new();

  for dir in ["compiler/tests/fixtures/programs", "std", "projects"] {
    px_files(&root.join(dir), &mut files);
  }

  assert!(!files.is_empty());

  for path in files {
    let src = std::fs::read_to_string(&path).expect("read a .px file");
    let uses_a_plugin = format(&src, "test.px").diagnostics.iter().any(|d| {
      d.help.as_deref().is_some_and(|h| h.contains("is a plugin zone"))
    });

    if uses_a_plugin {
      continue;
    }

    assert_eq!(formatted(&src), src, "{} is not canonical", path.display());
  }
}

#[test]
fn from_is_still_a_name() {
  let module = parse_ok(
    "module M\n\nbinds\n  Posts in Browser from Node\n\nfunctions\n  slice(from: Int) -> Int {\n    from\n  }\n",
  );

  assert_eq!(module.zones.len(), 2);
}
