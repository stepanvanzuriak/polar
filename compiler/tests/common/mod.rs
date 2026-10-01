#![allow(dead_code, unused_imports, unused_macros)]

pub mod arbitrary;
pub mod effectful;
pub mod invariants;
pub mod node;
pub mod sexp;
pub mod typed;
pub mod types;

use std::panic::{self, AssertUnwindSafe};

use polar_compiler::{
  shared::diagnostic::{Diagnostic, DiagnosticBag},
  shared::ice::InternalCompilerError,
  shared::source::SourceFile,
  syntax::lexer::lex,
  syntax::parser::Parser,
};

pub fn drive(src: &str, f: impl FnOnce(&mut Parser)) -> Vec<Diagnostic> {
  let file = SourceFile::new("test.px", src);
  let mut bag = DiagnosticBag::default();
  let lexed = lex(&file, &mut bag);

  {
    let mut parser =
      Parser::new(&file, &lexed.tokens, &lexed.comments, &mut bag);

    f(&mut parser);
  }

  bag.into_sorted()
}

pub fn codes(diagnostics: &[Diagnostic]) -> Vec<String> {
  diagnostics
    .iter()
    .map(|d| {
      format!("{}@{}..{}", d.code, d.primary.span.start, d.primary.span.end)
    })
    .collect()
}

pub fn expect_ice(f: impl FnOnce()) -> InternalCompilerError {
  let prev_hook = panic::take_hook();
  panic::set_hook(Box::new(|_| {}));

  let result = panic::catch_unwind(AssertUnwindSafe(f));

  panic::set_hook(prev_hook);

  match result {
    Ok(()) => {
      panic!("expected an ICE panic, but the closure returned normally")
    }
    Err(payload) => *payload
      .downcast::<InternalCompilerError>()
      .expect("panic payload was not an InternalCompilerError"),
  }
}

pub fn cdylib(manifest: &std::path::Path) -> String {
  let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
  let out = std::process::Command::new(cargo)
    .args(["build", "--message-format=json", "--manifest-path"])
    .arg(manifest)
    .output()
    .expect("run cargo");

  assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));

  String::from_utf8_lossy(&out.stdout)
    .lines()
    .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
    .filter(|m| m["reason"] == "compiler-artifact")
    .filter(|m| {
      m["target"]["kind"]
        .as_array()
        .is_some_and(|k| k.iter().any(|k| k == "cdylib"))
    })
    .find_map(|m| m["filenames"][0].as_str().map(ToString::to_string))
    .expect("a cdylib artifact")
}

pub fn notes_plugin() -> String {
  static PATH: std::sync::OnceLock<String> = std::sync::OnceLock::new();

  PATH
    .get_or_init(|| {
      cdylib(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
          .join("tests/fixtures/notes_plugin/Cargo.toml"),
      )
    })
    .clone()
}
