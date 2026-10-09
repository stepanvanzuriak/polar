use crate::{
  TestFn,
  shared::{
    codes::DiagnosticCode::{NotRunAsTest, TestHasParams},
    diagnostic::{Diagnostic, DiagnosticBag, Label},
    source::SourceFile,
  },
  syntax::ast::{Decl, FnDecl, Module},
};

const SUFFIX: &str = "_test.px";
const PREFIX: &str = "test_";

pub(crate) fn check(
  module: &Module,
  file: &SourceFile,
  bag: &mut DiagnosticBag,
) -> Vec<TestFn> {
  if !file.name().ends_with(SUFFIX) {
    return Vec::new();
  }

  let decls: Vec<&Decl> = module.zones.iter().flat_map(|z| &z.decls).collect();
  let exported = |name: &str| {
    decls.iter().any(|d| matches!(d, Decl::Export(e) if e.name.text == name))
  };
  let mut tests = Vec::new();

  for decl in &decls {
    let Decl::Fn(f) = decl else { continue };

    if !exported(&f.name.text) {
      continue;
    }

    if !f.name.text.starts_with(PREFIX) {
      bag.push(Diagnostic::warning(
        NotRunAsTest,
        format!("`{}` is not run as a test", f.name.text),
        Label::new(f.name.span.clone())
          .with_message("exported from a test file, but not named `test_*`"),
      ));
      continue;
    }

    if check_signature(f, bag) {
      tests.push(TestFn {
        name: f.name.text.clone(),
        line: file.position_at(f.name.span.start).line + 1,
      });
    }
  }

  tests
}

fn check_signature(f: &FnDecl, bag: &mut DiagnosticBag) -> bool {
  let name = &f.name.text;
  let mut ok = true;

  if !f.params.is_empty() {
    bag.push(Diagnostic::error(
      TestHasParams,
      format!("test `{name}` takes parameters"),
      Label::new(f.name.span.clone())
        .with_message("a `test_*` function takes no parameters"),
    ));
    ok = false;
  }

  ok
}
