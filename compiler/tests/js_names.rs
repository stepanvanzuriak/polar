mod common;

use std::{borrow::Cow, fmt::Write as _};

use common::node::node_check_module;
use polar_compiler::backend::js::{
  ast::{member, number, object, prop, var},
  names::{RESERVED, js_ident},
  print::print_expr,
};

#[test]
fn ordinary_identifier() {
  assert!(matches!(js_ident("create_post"), Cow::Borrowed("create_post")));
}

#[test]
fn reserved_word() {
  assert_eq!(js_ident("class"), "class$");
}

#[test]
fn strict_mode_word() {
  assert_eq!(js_ident("arguments"), "arguments$");
}

#[test]
fn non_writable_global() {
  assert_eq!(js_ident("NaN"), "NaN$");
}

#[test]
fn compiler_name_passes_through() {
  assert!(matches!(js_ident("x$1"), Cow::Borrowed("x$1")));
  assert!(matches!(js_ident("$match"), Cow::Borrowed("$match")));
}

#[test]
fn runtime_namespace() {
  assert!(matches!(js_ident("$rt"), Cow::Borrowed("$rt")));
}

#[test]
fn reserved_list_is_sorted() {
  assert!(RESERVED.windows(2).all(|w| w[0] < w[1]));
}

#[test]
fn property_names_are_untouched() {
  assert_eq!(print_expr(&member(var("post"), "class")), "post.class");
  assert_eq!(
    print_expr(&object(vec![prop("class", number(1.0))])),
    "{ class: 1 }"
  );
  assert_eq!(print_expr(&member(var("class"), "class")), "class$.class");
}

#[test]
fn whole_reserved_list_is_bindable() {
  let mut module = String::new();

  for name in RESERVED {
    let _ = writeln!(module, "let {} = 1;", js_ident(name));
  }

  node_check_module(&module).unwrap();

  assert!(node_check_module("let class = 1;\n").is_err());
}
