use crate::{
  TestFn,
  shared::{
    codes::DiagnosticCode::{AroundEachParams, NotRunAsTest, TestHasParams},
    diagnostic::{Diagnostic, DiagnosticBag, Label},
    source::SourceFile,
  },
  syntax::ast::{Builtin, Decl, ExportDecl, FnDecl, Module, Zone, ZoneKind},
};

const SUFFIX: &str = "_test.px";
const PREFIX: &str = "test_";
const EACH: &str = "around_each";

pub(crate) struct Checked {
  pub tests: Vec<TestFn>,
  pub each: bool,
}

pub(crate) fn auto_export(module: &mut Module, filename: &str) {
  if !filename.ends_with(SUFFIX) {
    return;
  }

  let decls: Vec<&Decl> = module.zones.iter().flat_map(|z| &z.decls).collect();
  let missing: Vec<ExportDecl> = decls
    .iter()
    .filter_map(|decl| match decl {
      Decl::Fn(f)
        if (f.name.text.starts_with(PREFIX) || f.name.text == EACH)
          && !decls.iter().any(
            |d| matches!(d, Decl::Export(e) if e.name.text == f.name.text),
          ) =>
      {
        Some(ExportDecl {
          span: f.name.span.clone(),
          name: f.name.clone(),
          methods: None,
        })
      }
      _ => None,
    })
    .collect();

  let Some(first) = missing.first() else { return };

  module.zones.push(Zone {
    span: first.span.clone(),
    kind: ZoneKind::Builtin(Builtin::Exports),
    decls: missing.into_iter().map(Decl::Export).collect(),
  });
}

pub(crate) fn check(
  module: &Module,
  file: &SourceFile,
  bag: &mut DiagnosticBag,
) -> Checked {
  if !file.name().ends_with(SUFFIX) {
    return Checked { tests: Vec::new(), each: false };
  }

  let decls: Vec<&Decl> = module.zones.iter().flat_map(|z| &z.decls).collect();
  let exported = |name: &str| {
    decls.iter().any(|d| matches!(d, Decl::Export(e) if e.name.text == name))
  };
  let mut tests = Vec::new();
  let mut each = false;

  for decl in &decls {
    let Decl::Fn(f) = decl else { continue };

    if !exported(&f.name.text) {
      continue;
    }

    if f.name.text == EACH {
      if f.params.len() == 1 {
        each = true;
      } else {
        bag.push(Diagnostic::error(
          AroundEachParams,
          format!("`{EACH}` takes {} parameters", f.params.len()),
          Label::new(f.name.span.clone()).with_message(
            "it takes the test to run: `around_each(run: function() -> {} / {| e})`",
          ),
        ));
      }

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

  Checked { tests, each }
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
