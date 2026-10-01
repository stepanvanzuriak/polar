use polar_compiler::{
  CompileOptions, Stage,
  core::lower::{Resolved, lower_with},
  dump_stage_with,
  shared::diagnostic::{Diagnostic, DiagnosticBag, Severity},
  shared::modules::ModuleSource,
  shared::source::SourceFile,
  syntax::lexer::lex,
  syntax::parser::parse,
};

const STORAGE: &str = "effects\n  Storage in DOM {\n    get(key: String) -> String\n    set(key: String, value: String) -> {}\n  }\n\n";

const CANVAS: &str =
  "effects\n  native Canvas in DOM {\n    draw(label: String) -> {}\n  }\n\n";

fn module(body: &str) -> String {
  format!("module M\n\nhosts\n  DOM\n  Node\n\n{body}")
}

fn source(path: &str, src: &str) -> ModuleSource {
  ModuleSource {
    path: path.to_string(),
    source: src.to_string(),
    specifier: format!("./{}.js", path.to_lowercase()),
    plugins: Vec::new(),
  }
}

fn diagnostics_with(src: &str, modules: Vec<ModuleSource>) -> Vec<Diagnostic> {
  let options = CompileOptions { modules, ..CompileOptions::default() };

  dump_stage_with(src, "test.px", Stage::Core, &options).diagnostics
}

fn codes_with(src: &str, modules: Vec<ModuleSource>) -> Vec<String> {
  diagnostics_with(src, modules).iter().map(|d| d.code.to_string()).collect()
}

fn codes(src: &str) -> Vec<String> {
  codes_with(src, Vec::new())
}

fn messages(src: &str) -> Vec<String> {
  diagnostics_with(src, Vec::new()).iter().map(|d| d.message.clone()).collect()
}

fn only(src: &str) -> Diagnostic {
  let diagnostics = diagnostics_with(src, Vec::new());

  assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
  diagnostics.into_iter().next().unwrap()
}

fn errors_only(src: &str, modules: Vec<ModuleSource>) -> Vec<String> {
  diagnostics_with(src, modules)
    .iter()
    .filter(|d| d.severity == Severity::Error)
    .map(|d| format!("{} {}", d.code, d.message))
    .collect()
}

#[test]
fn resolves_operation() {
  let src =
    module(&format!("{STORAGE}functions\n  f() {{ Storage.get(\"k\") }}\n"));
  let file = SourceFile::new("test.px", src.as_str());
  let mut bag = DiagnosticBag::default();
  let parsed = parse(&file, &lex(&file, &mut bag), &mut bag);
  let lowered = lower_with(&parsed, &[], &mut bag);

  assert!(!bag.has_errors(), "{:#?}", bag.into_sorted());

  let at = src.find("get(\"k\")").unwrap();
  let found = lowered
    .resolutions
    .0
    .iter()
    .find(|((start, _), _)| *start == at)
    .map(|(_, r)| r.clone());

  assert_eq!(
    found,
    Some(Resolved::Operation { effect: "Storage".into(), op: "get".into() })
  );
}

#[test]
fn unknown_operation() {
  let d = only(&module(&format!(
    "{STORAGE}functions\n  f() {{ Storage.gett(\"k\") }}\n"
  )));

  assert_eq!(d.code.to_string(), "POLAR0317");
  assert_eq!(d.message, "`Storage` has no operation `gett`");
  assert_eq!(d.help.as_deref(), Some("did you mean `get`?"));
}

#[test]
fn effect_as_value() {
  let d = only(&module(&format!("{STORAGE}functions\n  f() {{ Storage }}\n")));

  assert_eq!(d.code.to_string(), "POLAR0307");
  assert!(d.message.contains("effect"), "{}", d.message);
}

#[test]
fn unknown_host_in_effect() {
  assert_eq!(
    codes(&module(
      "effects\n  Storage in Mars {\n    get(key: String) -> String\n  }\n"
    )),
    vec!["POLAR0319"]
  );
}

#[test]
fn duplicate_effect() {
  assert_eq!(
    codes(&module(
      "effects\n  Storage in DOM {\n    get(key: String) -> String\n  }\n\n  Storage in Node {\n    get(key: String) -> String\n  }\n"
    )),
    vec!["POLAR0302"]
  );
}

const EXTERN: &str =
  "externs\n  local_get(key: String) -> String = \"./dom.js\" get_item\n\n";

#[test]
fn extern_in_bind() {
  let src = module(&format!(
    "{STORAGE}{EXTERN}binds\n  Storage in DOM {{\n    get(key) {{\n      local_get(key)\n    }}\n\n    set(key, value) {{\n      {{}}\n    }}\n  }}\n"
  ));

  assert!(codes(&src).is_empty(), "{:#?}", diagnostics_with(&src, Vec::new()));
}

#[test]
fn extern_in_function() {
  let src = module(&format!(
    "{STORAGE}{EXTERN}binds\n  Storage in DOM {{\n    get(key) {{\n      local_get(key)\n    }}\n\n    set(key, value) {{\n      {{}}\n    }}\n  }}\n\nfunctions\n  f() {{\n    local_get(\"k\")\n  }}\n"
  ));

  assert!(codes(&src).is_empty(), "{:#?}", diagnostics_with(&src, Vec::new()));
}

#[test]
fn extern_in_lambda_in_bind() {
  let src = module(&format!(
    "{STORAGE}{EXTERN}binds\n  Storage in DOM {{\n    get(key) {{\n      let f = function() {{ local_get(\"k\") }}\n      f()\n    }}\n\n    set(key, value) {{\n      {{}}\n    }}\n  }}\n"
  ));

  assert!(codes(&src).is_empty(), "{:#?}", diagnostics_with(&src, Vec::new()));
}

#[test]
fn missing_operation() {
  let src = module(&format!(
    "{STORAGE}binds\n  Storage in Node {{\n    get(key) {{\n      key\n    }}\n  }}\n"
  ));

  assert_eq!(codes(&src), vec!["POLAR0801"]);
  assert!(messages(&src)[0].contains("missing `set`"));

  let d = only(&src);

  assert_eq!(
    d.help.as_deref(),
    Some(
      "a binding provides every operation of its effect: add `set(key, value) { … }`"
    )
  );
}

#[test]
fn extra_operation() {
  let src = module(&format!(
    "{STORAGE}binds\n  Storage in Node {{\n    get(key) {{\n      key\n    }}\n\n    set(key, value) {{\n      {{}}\n    }}\n\n    clear() {{\n      {{}}\n    }}\n  }}\n"
  ));
  let d = only(&src);

  assert_eq!(d.code.to_string(), "POLAR0802");
  assert_eq!(&src[d.primary.span.start..d.primary.span.end], "clear");
}

#[test]
fn operation_param_count() {
  let src = module(&format!(
    "{STORAGE}binds\n  Storage in Node {{\n    get(key) {{\n      key\n    }}\n\n    set(key) {{\n      {{}}\n    }}\n  }}\n"
  ));

  assert_eq!(codes(&src), vec!["POLAR0807"]);
}

#[test]
fn duplicate_bind() {
  let bind = "  Storage in Node {\n    get(key) {\n      key\n    }\n\n    set(key, value) {\n      {}\n    }\n  }\n";
  let src = module(&format!("{STORAGE}binds\n{bind}\n{bind}"));
  let d = only(&src);

  assert_eq!(d.code.to_string(), "POLAR0804");
  assert_eq!(d.secondary.len(), 1);
}

#[test]
fn native_bind_needs_force() {
  let d = only(&module(&format!(
    "{CANVAS}binds\n  Canvas in Node {{\n    draw(label) {{\n      {{}}\n    }}\n  }}\n"
  )));

  assert_eq!(d.code.to_string(), "POLAR0805");
  assert_eq!(
    d.message,
    "`Canvas` is native to `DOM`; binding it in `Node` needs `force`"
  );
  assert_eq!(
    d.help.as_deref(),
    Some("write `force Canvas in Node` if an emulation is acceptable")
  );
}

#[test]
fn force_bind_ok() {
  let src = module(&format!(
    "{CANVAS}binds\n  force Canvas in Node {{\n    draw(label) {{\n      {{}}\n    }}\n  }}\n"
  ));

  assert!(codes(&src).is_empty(), "{:#?}", diagnostics_with(&src, Vec::new()));
}

#[test]
fn native_in_native_host() {
  let src = module(&format!(
    "{CANVAS}binds\n  Canvas in DOM {{\n    draw(label) {{\n      {{}}\n    }}\n  }}\n"
  ));

  assert!(codes(&src).is_empty(), "{:#?}", diagnostics_with(&src, Vec::new()));
}

#[test]
fn needless_force() {
  let src = module(&format!(
    "{STORAGE}binds\n  force Storage in Node {{\n    get(key) {{\n      key\n    }}\n\n    set(key, value) {{\n      {{}}\n    }}\n  }}\n"
  ));
  let d = only(&src);

  assert_eq!(d.code.to_string(), "POLAR0806");
  assert_eq!(d.severity, Severity::Warning);
  assert_eq!(d.message, "`force` is not needed here");
}

const A: &str = "module A\n\nhosts\n  DOM\n\neffects\n  Storage in DOM {\n    get(key: String) -> String\n  }\n\nexports\n  DOM\n  Storage\n";

#[test]
fn orphan_bind() {
  let src = "module B\n\nuses\n  A\n\nbinds\n  Storage in DOM {\n    get(key) {\n      key\n    }\n  }\n";

  assert_eq!(codes_with(src, vec![source("A", A)]), vec!["POLAR0803"]);
}

#[test]
fn bind_own_host() {
  let src = "module B\n\nuses\n  A\n\nhosts\n  Worker\n\nbinds\n  Storage in Worker {\n    get(key) {\n      key\n    }\n  }\n";

  assert!(errors_only(src, vec![source("A", A)]).is_empty());
}

#[test]
fn imported_effect_operation() {
  let src = "module B\n\nuses\n  A\n\nfunctions\n  f() {\n    Storage.get(\"k\")\n  }\n";

  assert!(errors_only(src, vec![source("A", A)]).is_empty());
}

#[test]
fn effect_export_needs_host() {
  let src = "module A\n\nhosts\n  DOM\n\neffects\n  Storage in DOM {\n    get(key: String) -> String\n  }\n\nexports\n  Storage\n";

  assert_eq!(codes(src), vec!["POLAR0706"]);
  assert!(messages(src)[0].contains("effect"));
}

#[test]
fn core_dump() {
  let src = "module Notes

hosts
  DOM
  Node

types
  Missing = Missing(String)

effects
  Storage in DOM {
    get(key: String) -> String / {Throws<Missing>}
  }

externs
  local_get(key: String) -> String = \"./dom.js\" get_item

binds
  Storage in DOM {
    get(key) {
      local_get(key)
    }
  }

  Storage in Node {
    get(key) {
      key
    }
  }

functions
  load(key: String) -> String / {Storage} {
    Storage.get(key)
  }
";
  let options = CompileOptions::default();
  let result = dump_stage_with(src, "notes.px", Stage::Core, &options);
  let dump = result.output.expect("a core dump");

  assert!(dump.contains("(extern local_get \"./dom.js\" get_item)"), "{dump}");
  assert!(dump.contains("(bind Storage DOM"), "{dump}");
  assert!(dump.contains("(bind Storage Node"), "{dump}");
  assert!(dump.contains("(extern local_get)"), "{dump}");
  assert!(dump.contains("(op Storage get)"), "{dump}");
}

#[test]
fn extern_named_like_function() {
  let src = module(&format!(
    "{EXTERN}functions\n  local_get(key: String) -> String {{\n    key\n  }}\n"
  ));

  assert_eq!(codes(&src), vec!["POLAR0302"]);
}

#[test]
fn bind_cannot_use_functions() {
  let src = module(&format!(
    "{STORAGE}binds\n  Storage in Node {{\n    get(key) {{\n      helper(key)\n    }}\n\n    set(key, value) {{\n      {{}}\n    }}\n  }}\n\nfunctions\n  helper(key: String) -> String {{\n    key\n  }}\n"
  ));

  assert_eq!(codes(&src), vec!["POLAR0313"]);
}

#[test]
fn effect_named_like_builtin() {
  let src =
    module("effects\n  Log in DOM {\n    write(line: String) -> {}\n  }\n");

  assert_eq!(codes(&src), vec!["POLAR0302"]);
}
