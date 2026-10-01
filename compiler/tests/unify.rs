mod common;

use std::sync::Arc;

use common::types::{Names, ty};
use polar_compiler::types::{
  generalise::{default_numbers, generalise, instantiate},
  print::print,
  store::Store,
  ty::{Constraint, Row, Scheme, Tail, Type},
  unify::{UnifyError, unify},
};

fn round(src: &str) -> String {
  let mut store = Store::default();
  let t = ty(src, &mut store, &mut Names::default());
  print(&t, &store)
}

#[test]
fn print_primitives() {
  assert_eq!(round("Int"), "Int");
  assert_eq!(round("Float"), "Float");
  assert_eq!(round("String"), "String");
  assert_eq!(round("Bool"), "Bool");
}

#[test]
fn print_generic() {
  assert_eq!(round("Result<String, List<Int>>"), "Result<String, List<Int>>");
}

#[test]
fn print_function() {
  assert_eq!(
    round("function(Int, String) -> Bool"),
    "function(Int, String) -> Bool"
  );
}

#[test]
fn print_no_params() {
  assert_eq!(round("function() -> {}"), "function() -> {}");
}

#[test]
fn print_letters_by_order() {
  assert_eq!(round("function(x, y) -> x"), "function(a, b) -> a");
}

#[test]
fn print_ignores_creation_order() {
  let mut store = Store::default();
  let mut names = Names::default();

  names.get("y", &mut store);
  names.get("x", &mut store);

  let t = ty("function(x, y) -> x", &mut store, &mut names);

  assert_eq!(print(&t, &store), "function(a, b) -> a");
}

#[test]
fn print_is_stable() {
  let mut store = Store::default();
  let t = ty(
    "function({ b: y, a: x | rest }) -> List<x>",
    &mut store,
    &mut Names::default(),
  );

  assert_eq!(print(&t, &store), print(&t, &store));
}

#[test]
fn print_record_sorted() {
  assert_eq!(round("{ title: String, id: Int }"), "{ id: Int, title: String }");
}

#[test]
fn print_open_record() {
  assert_eq!(round("{ title: String | rest }"), "{ title: String | r }");
}

#[test]
fn print_higher_order() {
  assert_eq!(
    round("function(List<a>, function(a) -> b) -> List<b>"),
    "function(List<a>, function(a) -> b) -> List<b>"
  );
}

#[test]
fn zonk_follows_links() {
  let mut store = Store::default();
  let v0 = store.fresh();
  let v1 = store.fresh();
  let v2 = store.fresh();

  store.set(v0, Type::Var(v1));
  store.set(v1, Type::Var(v2));
  store.set(v2, Type::int());

  assert_eq!(store.zonk(&Type::Var(v0)), Type::int());
}

#[test]
fn zonk_merges_rows() {
  let mut store = Store::default();
  let mut names = Names::default();
  let r = names.get("r", &mut store);

  store.set_row(
    r,
    Row::new(vec![(Arc::from("id"), Type::int())], Tail::anonymous()),
  );

  let t = ty("{ title: String | r }", &mut store, &mut names);

  assert_eq!(print(&t, &store), "{ id: Int, title: String }");
}

#[test]
fn zonk_twice_is_same() {
  let mut store = Store::default();
  let mut names = Names::default();
  let x = names.get("x", &mut store);
  let rest = names.get("rest", &mut store);
  let more = store.fresh();

  store.set(x, Type::string());
  store.set_row(
    rest,
    Row::new(vec![(Arc::from("id"), Type::Var(more))], Tail::anonymous()),
  );

  let t =
    ty("function({ title: x | rest }) -> List<y>", &mut store, &mut names);
  let once = store.zonk(&t);

  assert_eq!(store.zonk(&once), once);
}

fn unify_src(
  expected: &str,
  found: &str,
) -> (Result<(), UnifyError>, String, Store) {
  let mut store = Store::default();
  let mut names = Names::default();
  let e = ty(expected, &mut store, &mut names);
  let f = ty(found, &mut store, &mut names);
  let result = unify(&mut store, &e, &f);
  let printed = print(&e, &store);
  (result, printed, store)
}

fn parse(src: &str) -> Type {
  ty(src, &mut Store::default(), &mut Names::default())
}

fn is_label(result: &Result<(), UnifyError>, want: &str) -> bool {
  match result {
    Err(
      UnifyError::MissingField { label, .. }
      | UnifyError::ExtraField { label, .. },
    ) => &**label == want,
    _ => false,
  }
}

#[test]
fn unify_same_con() {
  assert_eq!(unify_src("Int", "Int").0, Ok(()));
}

#[test]
fn unify_con_mismatch() {
  assert_eq!(
    unify_src("Int", "String").0,
    Err(UnifyError::Mismatch { expected: Type::int(), found: Type::string() })
  );
}

#[test]
fn unify_generic_args() {
  let (result, printed, _) = unify_src("List<a>", "List<Int>");
  assert_eq!(result, Ok(()));
  assert_eq!(printed, "List<Int>");
}

#[test]
fn unify_whole_pair_on_error() {
  assert_eq!(
    unify_src("List<Int>", "List<String>").0,
    Err(UnifyError::Mismatch {
      expected: parse("List<Int>"),
      found: parse("List<String>"),
    })
  );
}

#[test]
fn unify_function() {
  let (result, printed, _) =
    unify_src("function(a) -> a", "function(Int) -> b");
  assert_eq!(result, Ok(()));
  assert_eq!(printed, "function(Int) -> Int");
}

#[test]
fn unify_param_count() {
  assert_eq!(
    unify_src("function(a) -> a", "function(a, b) -> a").0,
    Err(UnifyError::ParamCount { expected: 1, found: 2 })
  );
}

#[test]
fn occurs_check() {
  assert!(matches!(
    unify_src("a", "List<a>").0,
    Err(UnifyError::Occurs { .. })
  ));
}

#[test]
fn occurs_check_in_record() {
  assert!(matches!(
    unify_src("a", "{ next: a }").0,
    Err(UnifyError::Occurs { .. })
  ));
}

#[test]
fn unify_chain() {
  let mut store = Store::default();
  let mut names = Names::default();
  let a = ty("a", &mut store, &mut names);
  let b = ty("b", &mut store, &mut names);

  assert_eq!(unify(&mut store, &a, &b), Ok(()));
  assert_eq!(unify(&mut store, &b, &Type::int()), Ok(()));
  assert_eq!(print(&a, &store), "Int");
}

#[test]
fn fresh_uses_current_level() {
  let mut store = Store::default();
  store.enter_level();
  store.enter_level();
  let v = store.fresh();

  assert_eq!(store.level_of(v), Some(2));
}

#[test]
fn link_keeps_lower_level() {
  let mut store = Store::default();
  store.enter_level();
  let a = store.fresh();
  store.enter_level();
  store.enter_level();
  let b = store.fresh();
  let c = store.fresh();

  assert_eq!(unify(&mut store, &Type::Var(b), &Type::Var(a)), Ok(()));
  assert_eq!(unify(&mut store, &Type::Var(c), &Type::Var(b)), Ok(()));

  let Type::Var(survivor) = store.resolve(&Type::Var(c)) else {
    panic!("expected a placeholder");
  };

  assert_eq!(survivor, a);
  assert_eq!(store.level_of(survivor), Some(1));
}

#[test]
fn set_lowers_levels_inside() {
  let mut store = Store::default();
  store.enter_level();
  let a = store.fresh();
  store.enter_level();
  store.enter_level();
  let b = store.fresh();
  let list = Type::Con { name: Arc::from("List"), args: vec![Type::Var(b)] };

  assert_eq!(unify(&mut store, &Type::Var(a), &list), Ok(()));
  assert_eq!(store.level_of(b), Some(1));
}

fn deep_function(escape: bool) -> (Scheme, Store) {
  let mut store = Store::default();
  store.enter_level();
  let x = store.fresh();
  store.enter_level();
  let y = store.fresh();
  let f = Type::func(vec![Type::Var(y)], Type::Var(x));

  if escape {
    assert_eq!(unify(&mut store, &Type::Var(y), &Type::Var(x)), Ok(()));
  }

  store.leave_level();
  (generalise(&store, &f), store)
}

#[test]
fn generalise_deep_only() {
  let (scheme, store) = deep_function(false);

  assert_eq!(scheme.count, 1);
  assert!(matches!(
    &scheme.ty,
    Type::Fn { params, ret, .. }
      if params[..] == [Type::Gen(0)] && matches!(**ret, Type::Var(_))
  ));
  assert_eq!(print(&scheme.ty, &store), "function(a) -> b");
}

#[test]
fn generalise_after_escape() {
  let (scheme, store) = deep_function(true);

  assert_eq!(scheme.count, 0);
  assert_eq!(print(&scheme.ty, &store), "function(a) -> a");
}

fn id_scheme() -> Scheme {
  Scheme::new(1, Type::func(vec![Type::Gen(0)], Type::Gen(0)))
}

fn param(ty: &Type) -> Type {
  match ty {
    Type::Fn { params, .. } => params[0].clone(),
    _ => panic!("expected a function"),
  }
}

#[test]
fn instantiate_twice_independent() {
  let mut store = Store::default();
  let scheme = id_scheme();
  let first = instantiate(&mut store, &scheme);
  let second = instantiate(&mut store, &scheme);

  assert_eq!(unify(&mut store, &param(&first), &Type::int()), Ok(()));
  assert_eq!(print(&first, &store), "function(Int) -> Int");
  assert_eq!(print(&second, &store), "function(a) -> a");
}

#[test]
fn instantiate_shares_slot() {
  let mut store = Store::default();
  let t = instantiate(&mut store, &id_scheme());

  assert_eq!(unify(&mut store, &param(&t), &Type::int()), Ok(()));
  assert_eq!(print(&t, &store), "function(Int) -> Int");
}

#[test]
fn closed_same_order() {
  assert_eq!(
    unify_src("{ a: Int, b: String }", "{ a: Int, b: String }").0,
    Ok(())
  );
}

#[test]
fn closed_any_order() {
  assert_eq!(
    unify_src("{ a: Int, b: String }", "{ b: String, a: Int }").0,
    Ok(())
  );
}

#[test]
fn field_type_mismatch() {
  assert_eq!(
    unify_src("{ a: Int }", "{ a: String }").0,
    Err(UnifyError::Mismatch { expected: Type::int(), found: Type::string() })
  );
}

#[test]
fn closed_missing() {
  let (result, _, _) = unify_src("{ a: Int, b: Int }", "{ a: Int }");
  assert!(matches!(result, Err(UnifyError::MissingField { .. })));
  assert!(is_label(&result, "b"));
}

#[test]
fn closed_extra() {
  let (result, _, _) = unify_src("{ a: Int }", "{ a: Int, b: Int }");
  assert!(matches!(result, Err(UnifyError::ExtraField { .. })));
  assert!(is_label(&result, "b"));
}

#[test]
fn missing_is_alphabetical() {
  let (result, _, _) = unify_src("{ a: Int, c: Int, b: Int }", "{ a: Int }");
  assert!(matches!(result, Err(UnifyError::MissingField { .. })));
  assert!(is_label(&result, "b"));
}

#[test]
fn missing_names_found_record() {
  let (result, _, _) = unify_src("{ title: String | r }", "{ body: String }");
  assert_eq!(
    result,
    Err(UnifyError::MissingField {
      label: Arc::from("title"),
      record: parse("{ body: String }"),
    })
  );
}

#[test]
fn open_accepts_bigger_closed() {
  let (result, printed, _) =
    unify_src("{ title: String | r }", "{ id: Int, title: String }");
  assert_eq!(result, Ok(()));
  assert_eq!(printed, "{ id: Int, title: String }");
}

#[test]
fn open_rejects_missing() {
  let (result, _, _) = unify_src("{ title: String | r }", "{ body: String }");
  assert!(matches!(result, Err(UnifyError::MissingField { .. })));
  assert!(is_label(&result, "title"));
}

#[test]
fn closed_vs_open_found() {
  let mut store = Store::default();
  let mut names = Names::default();
  let e = ty("{ a: Int, b: Int }", &mut store, &mut names);
  let f = ty("{ a: Int | r }", &mut store, &mut names);
  let r = names.get("r", &mut store);

  assert_eq!(unify(&mut store, &e, &f), Ok(()));

  let rest = Type::Record(Row::new(Vec::new(), Tail::Open(r)));
  assert_eq!(print(&rest, &store), "{ b: Int }");
}

#[test]
fn two_open_rows() {
  let mut store = Store::default();
  let mut names = Names::default();
  let e = ty("{ a: Int | r1 }", &mut store, &mut names);
  let f = ty("{ b: String | r2 }", &mut store, &mut names);

  assert_eq!(unify(&mut store, &e, &f), Ok(()));
  assert_eq!(print(&e, &store), "{ a: Int, b: String | r }");
  assert_eq!(print(&f, &store), "{ a: Int, b: String | r }");
}

#[test]
fn same_tail_different_fields() {
  assert!(matches!(
    unify_src("{ a: Int | r }", "{ b: Int | r }").0,
    Err(UnifyError::Mismatch { .. })
  ));
}

#[test]
fn same_tail_same_fields() {
  assert_eq!(unify_src("{ a: Int | r }", "{ a: Int | r }").0, Ok(()));
}

#[test]
fn unit_vs_open() {
  let mut store = Store::default();
  let mut names = Names::default();
  let e = ty("{}", &mut store, &mut names);
  let f = ty("{ | r }", &mut store, &mut names);

  assert_eq!(unify(&mut store, &e, &f), Ok(()));
  assert_eq!(store.zonk(&f), Type::unit());
}

#[test]
fn row_occurs() {
  assert!(matches!(
    unify_src("{ a: Int | r }", "{ a: Int, b: { x: Int | r } }").0,
    Err(UnifyError::Occurs { .. })
  ));
}

#[test]
fn keeps_the_rest() {
  let mut store = Store::default();
  let mut names = Names::default();
  let e = ty("{ title: String | r }", &mut store, &mut names);
  let f =
    ty("{ id: Int, title: String, body: String }", &mut store, &mut names);
  let r = names.get("r", &mut store);

  assert_eq!(unify(&mut store, &e, &f), Ok(()));

  let rest = Type::Record(Row::new(Vec::new(), Tail::Open(r)));
  assert_eq!(print(&rest, &store), "{ body: String, id: Int }");
}

#[test]
fn row_link_lowers_levels() {
  let mut store = Store::default();
  store.enter_level();
  let r1 = store.fresh();
  store.enter_level();
  let r2 = store.fresh();
  let e = Type::Record(Row::new(Vec::new(), Tail::Open(r1)));
  let f =
    Type::Record(Row::new(vec![(Arc::from("a"), Type::int())], Tail::Open(r2)));

  assert_eq!(unify(&mut store, &e, &f), Ok(()));

  let Type::Record(row) = store.zonk(&f) else {
    panic!("expected a record");
  };
  let Tail::Open(r3) = row.tail else {
    panic!("expected an open row");
  };

  assert_eq!(store.level_of(r3), Some(1));
}

fn constrained(store: &mut Store, c: Constraint) -> Type {
  Type::Var(store.fresh_constrained(c))
}

fn not_numeric(result: &Result<(), UnifyError>, want: Constraint) -> bool {
  matches!(result, Err(UnifyError::NotNumeric { constraint, .. }) if *constraint == want)
}

#[test]
fn num_accepts_int() {
  let mut store = Store::default();
  let n = constrained(&mut store, Constraint::Num);
  assert_eq!(unify(&mut store, &n, &Type::int()), Ok(()));
}

#[test]
fn num_accepts_float() {
  let mut store = Store::default();
  let n = constrained(&mut store, Constraint::Num);
  assert_eq!(unify(&mut store, &n, &Type::float()), Ok(()));
}

#[test]
fn num_rejects_string() {
  let mut store = Store::default();
  let n = constrained(&mut store, Constraint::Num);
  let result = unify(&mut store, &n, &Type::string());
  assert!(not_numeric(&result, Constraint::Num));
}

#[test]
fn ord_accepts_string() {
  let mut store = Store::default();
  let o = constrained(&mut store, Constraint::Ord);
  assert_eq!(unify(&mut store, &o, &Type::string()), Ok(()));
}

#[test]
fn ord_rejects_bool() {
  let mut store = Store::default();
  let o = constrained(&mut store, Constraint::Ord);
  let result = unify(&mut store, &o, &Type::bool());
  assert!(not_numeric(&result, Constraint::Ord));
}

#[test]
fn num_rejects_record() {
  let mut store = Store::default();
  let n = constrained(&mut store, Constraint::Num);
  let result = unify(&mut store, &n, &parse("{ a: Int }"));
  assert!(not_numeric(&result, Constraint::Num));
}

#[test]
fn meet_when_linking() {
  let mut store = Store::default();
  let n = constrained(&mut store, Constraint::Num);
  let o = constrained(&mut store, Constraint::Ord);

  assert_eq!(unify(&mut store, &n, &o), Ok(()));

  let result = unify(&mut store, &o, &Type::string());
  assert!(not_numeric(&result, Constraint::Num));
}

#[test]
fn meet_keeps_lower_survivor() {
  let mut store = Store::default();
  let plain = Type::Var(store.fresh());
  store.enter_level();
  let n = constrained(&mut store, Constraint::Num);

  assert_eq!(unify(&mut store, &n, &plain), Ok(()));

  let result = unify(&mut store, &plain, &Type::string());
  assert!(not_numeric(&result, Constraint::Num));
}

#[test]
fn int_float_mismatch() {
  let mut store = Store::default();
  let n = constrained(&mut store, Constraint::Num);

  assert_eq!(unify(&mut store, &n, &Type::int()), Ok(()));
  assert!(matches!(
    unify(&mut store, &n, &Type::float()),
    Err(UnifyError::Mismatch { .. })
  ));
}

fn binary(n: &Type) -> Type {
  Type::func(vec![n.clone(), n.clone()], n.clone())
}

#[test]
fn not_generalised() {
  let mut store = Store::default();
  store.enter_level();
  let n = constrained(&mut store, Constraint::Num);
  let f = binary(&n);
  store.leave_level();

  let scheme = generalise(&store, &f);

  assert_eq!(scheme.count, 0);
  assert_eq!(scheme.ty, f);
}

#[test]
fn defaults_to_int() {
  let mut store = Store::default();
  let n = constrained(&mut store, Constraint::Num);
  let f = binary(&n);

  default_numbers(&mut store, &f);

  assert_eq!(print(&f, &store), "function(Int, Int) -> Int");
}

#[test]
fn ord_defaults_to_int() {
  let mut store = Store::default();
  let o = constrained(&mut store, Constraint::Ord);

  default_numbers(&mut store, &o);

  assert_eq!(store.zonk(&o), Type::int());
}

#[test]
fn plain_not_defaulted() {
  let mut store = Store::default();
  let f = ty("function(a) -> a", &mut store, &mut Names::default());

  default_numbers(&mut store, &f);

  assert_eq!(print(&f, &store), "function(a) -> a");
}
