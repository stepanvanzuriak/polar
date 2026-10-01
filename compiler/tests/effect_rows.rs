mod common;

use std::{path::PathBuf, sync::Arc};

use polar_compiler::{
  Stage, dump_stage,
  shared::diagnostic::{Diagnostic, Severity},
  shared::source::SourceFile,
  types::{
    generalise::{
      close_row, generalise, instantiate, instantiate_open, open_row,
    },
    print::print_scheme,
    store::Store,
    ty::{EffTail, Effects, Label, Scheme, Type},
    unify::{UnifyError, unify, unify_effects},
  },
};

const PREAMBLE: &str = "module M

uses
  Std.List

hosts
  DOM
  Node

types
  Missing = Missing(String)
  Invalid = | Invalid
  Status = Draft | Done

effects
  Db in Node {
    load(id: Int) -> String
  }

  Storage in DOM {
    get(key: String) -> String / {Throws<Missing>}
  }
";

fn with_effects(fns: &str) -> String {
  format!("{PREAMBLE}\nfunctions\n  {fns}\n")
}

fn types_of(src: &str) -> String {
  let result = dump_stage(src, "test.px", Stage::Types);

  result.output.unwrap_or_else(|| panic!("{:#?}", result.diagnostics))
}

fn type_of(src: &str, name: &str) -> String {
  let prefix = format!("{name} : ");
  let all = types_of(src);

  all
    .lines()
    .find_map(|l| l.strip_prefix(&prefix))
    .unwrap_or_else(|| panic!("no `{name}` in {all}"))
    .to_string()
}

fn diagnostics_of(src: &str) -> Vec<Diagnostic> {
  dump_stage(src, "test.px", Stage::Types)
    .diagnostics
    .into_iter()
    .filter(|d| d.severity == Severity::Error)
    .collect()
}

fn errors_of(src: &str) -> Vec<String> {
  let file = SourceFile::new("test.px", src);

  diagnostics_of(src)
    .iter()
    .map(|d| format!("{} at {:?}", d.code, file.slice(&d.primary.span)))
    .collect()
}

fn only(src: &str) -> Diagnostic {
  let diagnostics = diagnostics_of(src);

  assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
  diagnostics.into_iter().next().unwrap()
}

fn label(name: &str) -> Label {
  Label::effect(name)
}

fn row(labels: &[&str], tail: EffTail) -> Effects {
  Effects::new(labels.iter().map(|l| label(l)).collect(), tail)
}

mod row_types {
  use super::*;

  #[test]
  fn prints_row() {
    let src = with_effects(
      "f(g: function(Int) -> String / {Db | e}) -> {} / {| e} { {} }",
    );

    assert_eq!(
      type_of(&src, "f"),
      "function(function(Int) -> String / {Db | e}) -> {} / {| e}"
    );
  }

  #[test]
  fn pure_prints_nothing() {
    let src = with_effects("f(g: function(Int) -> String) -> Int { 1 }");

    assert_eq!(type_of(&src, "f"), "function(function(Int) -> String) -> Int");
  }

  #[test]
  fn labels_sorted() {
    let src =
      with_effects("f(g: function() -> {} / {Storage, Db}) -> {} { {} }");

    assert_eq!(
      type_of(&src, "f"),
      "function(function() -> {} / {Db, Storage}) -> {}"
    );
  }

  #[test]
  fn throws_label() {
    let src =
      with_effects("f(g: function() -> {} / {Throws<Missing>}) -> {} { {} }");

    assert_eq!(
      type_of(&src, "f"),
      "function(function() -> {} / {Throws<Missing>}) -> {}"
    );
  }

  #[test]
  fn two_throws() {
    let src = with_effects(
      "f(g: function() -> {} / {Throws<Missing>, Throws<Invalid>}) -> {} { {} }",
    );

    assert_eq!(
      type_of(&src, "f"),
      "function(function() -> {} / {Throws<Invalid>, Throws<Missing>}) -> {}"
    );
  }

  #[test]
  fn throws_needs_named_type() {
    let src =
      with_effects("f(g: function() -> {} / {Throws<Int>}) -> {} { {} }");

    assert_eq!(errors_of(&src), vec!["POLAR0809 at \"Int\""]);
  }

  #[test]
  fn throws_needs_parameterless_type() {
    let src =
      with_effects("f(g: function() -> {} / {Throws<List<a>>}) -> {} { {} }");
    let d = only(&src);

    assert_eq!(d.code.to_string(), "POLAR0809");
    assert_eq!(d.message, "`Throws` needs a type without parameters");
  }

  #[test]
  fn bare_throws() {
    let src = with_effects("f(g: function() -> {} / {Throws}) -> {} { {} }");
    let d = only(&src);

    assert_eq!(d.message, "`Throws` needs an error type: `Throws<E>`");
  }

  #[test]
  fn unknown_effect() {
    let d = only(&with_effects("f(g: function() -> {} / {Dbb}) -> {} { {} }"));

    assert_eq!(d.code.to_string(), "POLAR0316");
    assert_eq!(d.message, "there is no effect `Dbb`");
    assert_eq!(d.help.as_deref(), Some("did you mean `Db`?"));
  }

  #[test]
  fn duplicate_label() {
    let src = with_effects("f(g: function() -> {} / {Db, Db}) -> {} { {} }");

    assert_eq!(errors_of(&src), vec!["POLAR0302 at \"Db\""]);
  }

  #[test]
  fn row_var_letters() {
    let src = with_effects(
      "f(x: a, g: function(a) -> b / {| e}) -> b / {| e} { g(x) }",
    );

    assert_eq!(
      type_of(&src, "f"),
      "function(a, function(a) -> b / {| e}) -> b / {| e}"
    );
  }

  #[test]
  fn type_letters_skip_e() {
    let src =
      with_effects("f(a, b, c, d, x) { { a: a, b: b, c: c, d: d, x: x } }");

    assert_eq!(
      type_of(&src, "f"),
      "function(a, b, c, d, f) -> { a: a, b: b, c: c, d: d, x: f }"
    );
  }

  #[test]
  fn row_var_clash() {
    let src = with_effects("f(x: e, g: function() -> {} / {| e}) -> {} { {} }");
    let d = only(&src);

    assert_eq!(d.code.to_string(), "POLAR0501");
    assert_eq!(d.message, "`e` is used as both a type and an effect row");
  }
}

mod engine {
  use super::*;

  #[test]
  fn open_absorbs_label() {
    let mut store = Store::default();
    let r = store.fresh();
    let open = Effects::new(vec![], EffTail::Open(r));
    let db = row(&["Db"], EffTail::Closed);

    unify_effects(&mut store, &open, &db).unwrap();

    assert_eq!(store.zonk_effects(&open), db);
  }

  #[test]
  fn closed_rejects_extra() {
    let mut store = Store::default();
    let error = unify_effects(
      &mut store,
      &row(&["Db"], EffTail::Closed),
      &row(&["Db", "Storage"], EffTail::Closed),
    )
    .unwrap_err();

    assert!(
      matches!(&error, UnifyError::ExtraEffect { label, .. } if *label == super::label("Storage")),
      "{error:?}"
    );
  }

  #[test]
  fn closed_missing() {
    let mut store = Store::default();
    let error = unify_effects(
      &mut store,
      &row(&["Db", "Storage"], EffTail::Closed),
      &row(&["Db"], EffTail::Closed),
    )
    .unwrap_err();

    assert!(
      matches!(&error, UnifyError::MissingEffect { label, .. } if *label == super::label("Storage")),
      "{error:?}"
    );
  }

  #[test]
  fn both_open() {
    let mut store = Store::default();
    let (r1, r2) = (store.fresh(), store.fresh());
    let a = row(&["Db"], EffTail::Open(r1));
    let b = row(&["Storage"], EffTail::Open(r2));

    unify_effects(&mut store, &a, &b).unwrap();

    let (za, zb) = (store.zonk_effects(&a), store.zonk_effects(&b));

    assert_eq!(za, zb);
    assert_eq!(za.labels, vec![label("Db"), label("Storage")]);
    assert!(matches!(za.tail, EffTail::Open(r3) if r3 != r1 && r3 != r2));
  }

  #[test]
  fn same_tail_different_labels() {
    let mut store = Store::default();
    let r = store.fresh();
    let error = unify_effects(
      &mut store,
      &row(&["Db"], EffTail::Open(r)),
      &row(&["Storage"], EffTail::Open(r)),
    )
    .unwrap_err();

    assert!(matches!(error, UnifyError::EffectMismatch { .. }), "{error:?}");
  }

  #[test]
  fn rigid_tail() {
    let mut store = Store::default();
    let e = store.fresh();
    let error = unify_effects(
      &mut store,
      &Effects::new(vec![], EffTail::Rigid(e)),
      &row(&["Db"], EffTail::Closed),
    )
    .unwrap_err();

    assert!(matches!(error, UnifyError::EffectEscape { .. }), "{error:?}");
  }

  #[test]
  fn throws_distinct() {
    let mut store = Store::default();
    let a = Effects::new(vec![Label::throws("M.A")], EffTail::Closed);
    let b = Effects::new(vec![Label::throws("M.B")], EffTail::Closed);
    let error = unify_effects(&mut store, &a, &b).unwrap_err();

    assert!(
      matches!(&error, UnifyError::ExtraEffect { label, .. } | UnifyError::MissingEffect { label, .. } if label.is_throws()),
      "{error:?}"
    );
  }

  #[test]
  fn fn_rows_unify() {
    let mut store = Store::default();
    let r = store.fresh();
    let expected =
      Type::func_with(vec![], Type::int(), row(&["Db"], EffTail::Closed));
    let found = Type::func_with(vec![], Type::int(), Effects::open(r));

    unify(&mut store, &expected, &found).unwrap();

    assert_eq!(
      store.zonk_effects(&Effects::open(r)),
      row(&["Db"], EffTail::Closed)
    );
  }

  #[test]
  fn level_lowered_through_row() {
    let mut store = Store::default();

    store.enter_level();

    let outer = store.fresh();

    store.enter_level();

    let tail = store.fresh();

    store.leave_level();
    store.leave_level();

    let f = Type::func_with(vec![], Type::int(), Effects::open(tail));

    unify(&mut store, &Type::Var(outer), &f).unwrap();

    assert_eq!(store.level_of(tail), Some(1));
  }

  #[test]
  fn generalise_row_tail() {
    let mut store = Store::default();

    store.enter_level();

    let tail = store.fresh();
    let func =
      Type::func_with(vec![], Type::unit(), row(&["Db"], EffTail::Open(tail)));

    store.leave_level();

    let scheme = generalise(&store, &func);

    assert!(matches!(
      &scheme.ty,
      Type::Fn { effects, .. } if matches!(effects.tail, EffTail::Gen(_))
    ));

    let tail = |ty: Type| match ty {
      Type::Fn { effects, .. } => effects.tail,
      other => panic!("{other:?}"),
    };
    let first = tail(instantiate(&mut store, &scheme));
    let second = tail(instantiate(&mut store, &scheme));

    assert!(
      matches!((first, second), (EffTail::Open(x), EffTail::Open(y)) if x != y)
    );
  }

  #[test]
  fn instantiate_opens() {
    let mut store = Store::default();
    let scheme = Scheme::new(
      0,
      Type::func_with(vec![], Type::unit(), row(&["Db"], EffTail::Closed)),
    );
    let (ty, _) = instantiate_open(&mut store, &scheme);

    assert!(matches!(
      ty,
      Type::Fn { effects, .. }
        if effects.labels == vec![label("Db")]
          && matches!(effects.tail, EffTail::Open(_))
    ));
  }

  #[test]
  fn open_is_shallow() {
    let mut store = Store::default();
    let inner = Type::func(vec![], Type::unit());
    let ty =
      open_row(&mut store, Type::func(vec![inner.clone()], Type::unit()));

    let Type::Fn { params, effects, .. } = ty else { panic!("a function") };

    assert!(matches!(effects.tail, EffTail::Open(_)));
    assert_eq!(params[0], inner);
  }

  #[test]
  fn closes_row() {
    let mut scheme = Scheme::new(
      1,
      Type::func_with(vec![], Type::unit(), row(&["Db"], EffTail::Gen(0))),
    );

    close_row(&mut scheme);

    assert_eq!(print_scheme(&scheme), "function() -> {} / {Db}");
  }

  #[test]
  fn close_row_keeps_shared() {
    let param = Type::func_with(
      vec![],
      Type::unit(),
      Effects::new(vec![], EffTail::Gen(0)),
    );
    let mut scheme = Scheme::new(
      1,
      Type::func_with(
        vec![param],
        Type::unit(),
        Effects::new(vec![], EffTail::Gen(0)),
      ),
    );
    let before = print_scheme(&scheme);

    close_row(&mut scheme);

    assert_eq!(print_scheme(&scheme), before);
    assert_eq!(before, "function(function() -> {} / {| e}) -> {} / {| e}");
  }

  #[test]
  fn close_row_leaves_negative_rows() {
    let param = Type::func_with(
      vec![],
      Type::unit(),
      Effects::new(vec![], EffTail::Gen(0)),
    );
    let mut scheme = Scheme::new(1, Type::func(vec![param], Type::unit()));

    close_row(&mut scheme);

    assert_eq!(
      print_scheme(&scheme),
      "function(function() -> {} / {| e}) -> {}"
    );
  }

  #[test]
  fn arc_labels_compare_by_value() {
    let a = Label { name: Arc::from("Db"), arg: None };

    assert_eq!(a, label("Db"));
  }
}

mod inference {
  use super::*;

  #[test]
  fn pure_then_effect() {
    let src = with_effects(
      "f(id) {\n    let s = String.uppercase(\"x\")\n    Db.load(id)\n  }",
    );

    assert_eq!(type_of(&src, "f"), "function(Int) -> String / {Db}");
  }

  #[test]
  fn inferred_row() {
    let src = with_effects("f(id) { Db.load(id) }");

    assert_eq!(type_of(&src, "f"), "function(Int) -> String / {Db}");
  }

  #[test]
  fn declared_row_ok() {
    let src = with_effects("f(id: Int) -> String / {Db} { Db.load(id) }");

    assert!(errors_of(&src).is_empty(), "{:?}", errors_of(&src));
  }

  #[test]
  fn declared_pure_rejects() {
    let d = only(&with_effects("f(id: Int) -> String { Db.load(id) }"));

    assert_eq!(d.code.to_string(), "POLAR0808");
    assert!(d.message.contains("`Db`"), "{}", d.message);
  }

  #[test]
  fn pipe_performs() {
    let src = with_effects("f(id) { id |> Db.load }");

    assert_eq!(type_of(&src, "f"), "function(Int) -> String / {Db}");
  }

  #[test]
  fn recursive_group() {
    let src = with_effects(
      "f(n) { if n == 0 { Db.load(0) } else { g(n) } }\n\n  g(n) { f(n - 1) }",
    );

    assert_eq!(type_of(&src, "f"), "function(Int) -> String / {Db}");
    assert_eq!(type_of(&src, "g"), "function(Int) -> String / {Db}");
  }

  #[test]
  fn lambda_does_not_perform() {
    let src = with_effects("f() { function() { Db.load(1) } }");

    assert_eq!(type_of(&src, "f"), "function() -> function() -> String / {Db}");
  }

  #[test]
  fn calling_lambda_performs() {
    let src = with_effects(
      "f() {\n    let g = function() { Db.load(1) }\n    g()\n  }",
    );

    assert_eq!(type_of(&src, "f"), "function() -> String / {Db}");
  }

  #[test]
  fn map_polymorphic() {
    let src =
      with_effects("f(xs) { List.map(xs, function(x) { Db.load(x) }) }");

    assert_eq!(
      type_of(&src, "f"),
      "function(List<Int>) -> List<String> / {Db}"
    );
  }

  #[test]
  fn map_pure() {
    let src = with_effects("f(xs) { List.map(xs, function(x) { x + 1 }) }");

    assert_eq!(type_of(&src, "f"), "function(List<Int>) -> List<Int>");
  }

  #[test]
  fn pure_named_as_callback() {
    let src = with_effects("f(xs) { List.map(xs, String.uppercase) }");

    assert_eq!(type_of(&src, "f"), "function(List<String>) -> List<String>");
  }

  #[test]
  fn pure_param_rejects_effect() {
    let src = with_effects(
      "h(g: function() -> Int) -> Int { g() }\n\n  f() {\n    h(function() {\n      Db.load(1)\n      1\n    })\n  }",
    );
    let d = only(&src);

    assert_eq!(d.code.to_string(), "POLAR0501");
    assert!(
      d.notes.iter().any(|n| n.contains("differ only in their effects")),
      "{:?}",
      d.notes
    );
  }

  #[test]
  fn rigid_row_blocks_extra() {
    let src = with_effects(
      "each(xs: List<a>, f: function(a) -> {} / {| e}) -> {} / {| e} {\n    Db.load(1)\n    {}\n  }",
    );
    let d = only(&src);

    assert_eq!(d.code.to_string(), "POLAR0808");
    assert!(d.message.contains("`Db`"), "{}", d.message);
  }

  #[test]
  fn operation_row() {
    let src = with_effects("f() { Storage.get(\"k\") }");

    assert_eq!(
      type_of(&src, "f"),
      "function() -> String / {Storage, Throws<Missing>}"
    );
  }

  #[test]
  fn constant_must_be_pure() {
    let src = "module M\n\nhosts\n  Node\n\nconstants\n  x: String = Db.load(1)\n\neffects\n  Db in Node {\n    load(id: Int) -> String\n  }\n";
    let d = only(src);

    assert_eq!(d.code.to_string(), "POLAR0808");
    assert_eq!(d.message, "the constant `x` uses `Db`; constants must be pure");
  }

  #[test]
  fn bind_throws_undeclared() {
    let src = format!(
      "{PREAMBLE}\nexterns\n  throw_invalid() -> String / {{Throws<Invalid>}} = \"./x.js\" boom\n\nbinds\n  Db in Node {{\n    load(id) {{\n      throw_invalid()\n    }}\n  }}\n"
    );
    let d = only(&src);

    assert_eq!(d.code.to_string(), "POLAR0808");
    assert_eq!(
      d.message,
      "the binding of `load` can throw `Invalid`, but `Db.load` doesn't declare it"
    );
  }

  #[test]
  fn bind_body_checked_against_operation() {
    let src = format!(
      "{PREAMBLE}\nbinds\n  Db in Node {{\n    load(id) {{\n      id\n    }}\n  }}\n"
    );

    assert_eq!(errors_of(&src), vec!["POLAR0501 at \"id\""]);
  }

  #[test]
  fn bind_may_use_other_effects() {
    let src = format!(
      "{PREAMBLE}\nbinds\n  Db in Node {{\n    load(id) {{\n      Storage.get(\"k\")\n    }}\n  }}\n"
    );
    let errors: Vec<String> = errors_of(&src)
      .into_iter()
      .filter(|e| !e.starts_with("POLAR0813") && !e.starts_with("POLAR0808"))
      .collect();

    assert!(errors.is_empty(), "{errors:?}");
  }

  #[test]
  fn impl_method_pure() {
    let src = "module M\n\nhosts\n  Node\n\ntraits\n  Describe<a> {\n    describe(x: a) -> String\n  }\n\ntypes\n  Status = Draft | Done\n\neffects\n  Db in Node {\n    load(id: Int) -> String\n  }\n\nimpls\n  Describe for Status {\n    describe(s) {\n      Db.load(1)\n    }\n  }\n";
    let d = only(src);

    assert_eq!(d.code.to_string(), "POLAR0808");
    assert_eq!(
      d.message,
      "`describe` uses `Db`, but `Describe.describe` is pure"
    );
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
  fn examples_and_std() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut files = Vec::new();

    for dir in ["compiler/tests/fixtures/programs", "std"] {
      px_files(&root.join(dir), &mut files);
    }

    for path in files {
      let src = std::fs::read_to_string(&path).expect("read a .px file");
      let errors = errors_of(&src);

      assert!(errors.is_empty(), "{}: {errors:?}", path.display());
    }
  }
}
