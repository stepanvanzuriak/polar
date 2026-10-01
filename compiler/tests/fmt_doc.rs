use polar_compiler::fmt::doc::{
  Doc, anchor, closer, concat, group, hardline, if_break, line, line_suffix,
  nest, render, softline, text,
};

mod common;

use common::expect_ice;

fn braces() -> Doc {
  group(concat([text("{"), line(), text("a: 1"), line(), text("}")]))
}

fn call() -> Doc {
  group(concat([text("f("), softline(), text("x"), softline(), text(")")]))
}

fn trailing_comma() -> Doc {
  group(concat([text("a"), if_break(text(","), text(""))]))
}

#[test]
fn flat_when_it_fits() {
  assert_eq!(render(&braces(), 40), "{ a: 1 }");
}

#[test]
fn breaks_when_it_does_not() {
  assert_eq!(render(&braces(), 6), "{\na: 1\n}");
}

#[test]
fn softline_is_empty_when_flat() {
  assert_eq!(render(&call(), 40), "f(x)");
}

#[test]
fn softline_breaks_when_wide() {
  assert_eq!(render(&call(), 3), "f(\nx\n)");
}

#[test]
fn nest_indents_broken_lines() {
  let doc = group(concat([
    text("["),
    nest(2, concat([line(), text("a")])),
    line(),
    text("]"),
  ]));

  assert_eq!(render(&doc, 3), "[\n  a\n]");
}

#[test]
fn outer_breaks_inner_stays_flat() {
  let inner =
    group(concat([text("("), softline(), text("x, y"), softline(), text(")")]));
  let doc = group(concat([
    text("outer"),
    nest(2, concat([line(), text("inner"), inner])),
    line(),
    text("a-long-enough-tail"),
  ]));

  assert_eq!(render(&doc, 20), "outer\n  inner(x, y)\na-long-enough-tail");
}

#[test]
fn hard_line_forces_the_parent() {
  let doc =
    group(concat([text("a"), line(), text("b"), hardline(), text("c")]));

  assert_eq!(render(&doc, 1000), "a\nb\nc");
}

#[test]
fn hard_line_propagates_through_nesting() {
  let innermost =
    group(concat([text("x"), line(), text("y"), hardline(), text("z")]));
  let middle = group(concat([text("m"), line(), innermost]));
  let outer = group(concat([text("o"), line(), middle]));

  assert!(outer.has_hard_line());
  assert_eq!(render(&outer, 1000), "o\nm\nx\ny\nz");
}

#[test]
fn if_break_flat() {
  assert_eq!(render(&trailing_comma(), 40), "a");
}

#[test]
fn if_break_broken() {
  assert_eq!(render(&trailing_comma(), 0), "a,");
}

#[test]
fn line_suffix_precedes_the_newline() {
  let doc =
    concat([text("x"), line_suffix(text(" // c")), hardline(), text("y")]);

  assert_eq!(render(&doc, 40), "x // c\ny");
}

#[test]
fn line_suffix_waits_for_the_line_to_end() {
  let doc = concat([
    text("a"),
    line_suffix(text(" // c")),
    text(","),
    hardline(),
    text("b"),
  ]);

  assert_eq!(render(&doc, 40), "a, // c\nb");
}

#[test]
fn two_line_suffixes_never_share_a_line() {
  let doc = concat([
    text("a"),
    line_suffix(text(" // one")),
    text(" + b"),
    line_suffix(text(" // two")),
    hardline(),
    anchor(),
    text("c"),
  ]);

  assert_eq!(render(&doc, 40), "a + b // one\n// two\nc");
}

#[test]
fn a_pushed_off_suffix_goes_below_blank_lines() {
  let doc = nest(
    2,
    concat([
      text("a"),
      line_suffix(text(" // one")),
      line_suffix(text(" // two")),
      hardline(),
      hardline(),
      anchor(),
      text("b"),
    ]),
  );

  assert_eq!(render(&doc, 40), "a // one\n\n  // two\n  b");
}

#[test]
fn a_pushed_off_suffix_waits_for_the_next_anchor() {
  let doc = concat([
    text("a"),
    line_suffix(text(" // one")),
    line_suffix(text(" // two")),
    nest(2, concat([hardline(), text("|| "), anchor(), text("b")])),
  ]);

  assert_eq!(render(&doc, 40), "a // one\n  || // two\n  b");
}

#[test]
fn a_pushed_off_suffix_goes_inside_the_brackets_above_a_closer() {
  let doc = concat([
    text("{"),
    line_suffix(text(" // one")),
    nest(2, line_suffix(text(" // two"))),
    hardline(),
    closer(2, text("}")),
  ]);

  assert_eq!(render(&doc, 40), "{ // one\n  // two\n}");
}

#[test]
fn a_pushed_off_suffix_without_an_anchor_ends_the_document() {
  let doc = concat([
    text("a"),
    line_suffix(text(" // one")),
    line_suffix(text(" // two")),
  ]);

  assert_eq!(render(&doc, 40), "a // one\n// two");
}

#[test]
fn no_trailing_whitespace() {
  let doc = group(concat([
    text("x "),
    nest(4, concat([hardline(), hardline(), text("y "), line(), text("z")])),
  ]));
  let out = render(&doc, 80);

  assert_eq!(out, "x\n\n    y\n    z");
  assert!(out.lines().all(|l| !l.ends_with(' ')), "{out:?}");
}

#[test]
fn width_counts_code_points() {
  let doc = group(concat([text("😀😀😀"), line(), text("x")]));

  assert_eq!(render(&doc, 4), "😀😀😀\nx");
  assert_eq!(render(&doc, 5), "😀😀😀 x");
}

#[test]
fn text_with_a_newline_ices() {
  let ice = expect_ice(|| {
    let _ = text("a\nb");
  });

  assert!(ice.message.contains("newline"), "{}", ice.message);
}

#[test]
fn deep_document_does_not_overflow() {
  let handle = std::thread::Builder::new()
    .stack_size(64 * 1024)
    .spawn(|| {
      let mut doc = text("x");

      for _ in 0..100_000 {
        doc = group(nest(1, concat([softline(), doc])));
      }

      render(&doc, 80).len()
    })
    .expect("spawn a small-stack thread");

  assert!(handle.join().expect("render on a small stack") > 0);
}
