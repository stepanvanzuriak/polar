use polar_compiler::{
  FormatResult,
  fmt::{format_with, print::print_module},
  format,
  shared::diagnostic::DiagnosticBag,
  shared::source::SourceFile,
  syntax::ast::eq::ast_eq,
  syntax::lexer::{lex, token::Comment},
  syntax::parser::{parse, precedence::PrecedenceTable},
};

mod common;

use common::expect_ice;

fn fmt(src: &str) -> String {
  let FormatResult { output, diagnostics } = format(src, "test.px");

  output
    .unwrap_or_else(|| panic!("refused to format:\n{src}\n{diagnostics:#?}"))
}

#[track_caller]
fn assert_formatted(src: &str) {
  assert_eq!(fmt(src), src);
}

#[track_caller]
fn assert_formats(src: &str, expected: &str) {
  assert_eq!(fmt(src), expected);
  assert_eq!(
    fmt(expected),
    expected,
    "the expected output is not a fixed point"
  );
}

fn in_fn(lines: &str) -> String {
  let body: String = lines
    .lines()
    .map(|l| if l.is_empty() { "\n".to_string() } else { format!("    {l}\n") })
    .collect();

  format!("functions\n  f() {{\n{body}  }}\n")
}

fn types(entries: &str) -> String {
  let body: String = entries
    .lines()
    .map(|l| if l.is_empty() { "\n".to_string() } else { format!("  {l}\n") })
    .collect();

  format!("types\n{body}")
}

fn comment_texts(src: &str) -> Vec<String> {
  let file = SourceFile::new("test.px", src);
  let mut bag = DiagnosticBag::default();

  lex(&file, &mut bag)
    .comments
    .iter()
    .map(|c: &Comment| src[c.span.start..c.span.end].trim_end().to_string())
    .collect()
}

mod idempotent_shapes {
  use super::*;

  #[test]
  fn short_record_type() {
    assert_formatted(&types("P = { a: Int, b: String }"));
  }

  #[test]
  fn empty_record_type() {
    assert_formatted(&types("P = {}"));
  }

  #[test]
  fn row_tail_flat() {
    assert_formatted(
      "functions\n  slug(r: { title: String | rest }) -> String {\n    r.title\n  }\n",
    );
  }

  #[test]
  fn short_variant() {
    assert_formatted(&types("Shape = Circle(Float) | Square(Float)"));
  }

  #[test]
  fn lone_constructor_keeps_its_bar() {
    assert_formatted(&types("Unit = | Unit"));
    assert_formatted(&types("Wrap = Wrap(Int)"));
  }

  #[test]
  fn derive_on_the_closing_line() {
    assert_formatted(&types(
      "Post = {\n  id: Int,\n  title: String,\n  body: String,\n  author_id: Int,\n  \
       published_at_year: Int,\n} derive(Json, Sql, Eq)",
    ));
  }

  #[test]
  fn fn_signature_flat() {
    assert_formatted(
      "functions\n  f(a: Int, b: String) -> Bool / {Db} {\n    a\n  }\n",
    );
  }

  #[test]
  fn short_if() {
    assert_formatted(&in_fn("if a { b } else { c }"));
  }

  #[test]
  fn short_pipeline() {
    assert_formatted(&in_fn(
      r#"r.title |> String.lowercase |> String.replace(" ", "-")"#,
    ));
  }

  #[test]
  fn short_lambda() {
    assert_formatted(&in_fn("function(x) { x + 1 }"));
  }

  #[test]
  fn goal_examples() {
    assert_formatted(
      r#"functions
  create_post(body: NewPost) -> Post / {Db, Auth, Throws<Invalid>} {
    let user = Auth.require()
    let post = Db.insert(Posts, { ..body, author_id: user.id })
    Log.info("created #{post.id}")
    post
  }

  slug(r: { title: String | rest }) -> String {
    r.title |> String.lowercase |> String.replace(" ", "-")
  }
"#,
    );
  }

  #[test]
  fn examples_are_formatted() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/programs");

    for entry in std::fs::read_dir(dir).expect("read tests/fixtures/programs/")
    {
      let path = entry.expect("compiler/tests/fixtures/programs/ entry").path();

      if path.extension().is_some_and(|e| e == "px") {
        let src = std::fs::read_to_string(&path).expect("read an example");

        assert_eq!(fmt(&src), src, "{} is not formatted", path.display());
      }
    }
  }
}

mod breaking {
  use super::*;

  #[test]
  fn long_record_breaks() {
    assert_formats(
      &in_fn(
        r#"{ id: 1, title: "Hello Polar World", body: "Polar compiles to JavaScript.", author_id: 7 }"#,
      ),
      &in_fn(
        "{\n  id: 1,\n  title: \"Hello Polar World\",\n  body: \"Polar compiles to JavaScript.\",\n  \
         author_id: 7,\n}",
      ),
    );
  }

  #[test]
  fn long_row_tail() {
    assert_formats(
      &types(
        "Named = { title: String, subtitle: String, author: String, description: String, slug: String | rest }",
      ),
      &types(
        "Named = {\n  title: String,\n  subtitle: String,\n  author: String,\n  \
         description: String,\n  slug: String,\n  | rest\n}",
      ),
    );
  }

  #[test]
  fn long_variant_breaks() {
    assert_formats(
      &types(
        "Shape = Circle(Float) | Square(Float) | Rectangle(Float, Float) | Triangle(Float, Float, Float)",
      ),
      &types(
        "Shape =\n  | Circle(Float)\n  | Square(Float)\n  | Rectangle(Float, Float)\n  \
         | Triangle(Float, Float, Float)",
      ),
    );
  }

  #[test]
  fn long_signature_breaks_params() {
    assert_formats(
      "functions\n  create(title: String, body: String, author: Int, tags: List<String>, draft: Bool) -> Post / {Db} {\n    title\n  }\n",
      "functions\n  create(\n    title: String,\n    body: String,\n    author: Int,\n    tags: List<String>,\n    \
       draft: Bool,\n  ) -> Post / {Db} {\n    title\n  }\n",
    );
  }

  #[test]
  fn long_call_breaks() {
    assert_formats(
      &in_fn(
        r"Log.info(first_argument_value, second_argument_value, third_argument_value, fourth_one)",
      ),
      &in_fn(
        "Log.info(\n  first_argument_value,\n  second_argument_value,\n  third_argument_value,\n  fourth_one,\n)",
      ),
    );
  }

  #[test]
  fn long_pipeline_breaks() {
    assert_formats(
      &in_fn(
        r#"post.title |> String.lowercase |> String.replace(" ", "-") |> String.trim |> String.truncate(80)"#,
      ),
      &in_fn(
        "post.title\n  |> String.lowercase\n  |> String.replace(\" \", \"-\")\n  |> String.trim\n  \
         |> String.truncate(80)",
      ),
    );
  }

  #[test]
  fn long_boolean_chain_breaks() {
    assert_formats(
      &in_fn(
        "is_published && has_title && has_body && author_is_verified && not_deleted && visible_now",
      ),
      &in_fn(
        "is_published\n  && has_title\n  && has_body\n  && author_is_verified\n  && not_deleted\n  && visible_now",
      ),
    );
  }

  #[test]
  fn function_body_always_breaks() {
    assert_formats(
      "functions\n  f() { 1 }\n",
      "functions\n  f() {\n    1\n  }\n",
    );
  }

  #[test]
  fn zone_keyword_is_never_indented() {
    let out = fmt(
      "  module M\n     uses\n A.B\n    types\n        T = Int\n functions\n f() { \
       match x { 1 -> { let y = 2\n y }, _ -> 3 } }\n  exports\n       f\n",
    );

    for line in out.lines().filter(|l| !l.is_empty()) {
      let zone = ["module", "uses", "types", "functions", "exports"]
        .iter()
        .any(|kw| line.split_whitespace().next() == Some(kw));

      if zone {
        assert!(!line.starts_with(' '), "zone keyword indented: {line:?}");
      } else {
        assert!(line.starts_with("  "), "entry not indented: {line:?}");
      }
    }

    assert!(out.contains("\n  f() {\n"), "{out}");
  }

  #[test]
  fn empty_zone_prints_its_keyword() {
    assert_formats(
      "types\n\n\nfunctions\n  f() { 1 }\n",
      "types\n\nfunctions\n  f() {\n    1\n  }\n",
    );
    assert_formats("types\n", "types\n");
  }

  #[test]
  fn match_always_breaks() {
    assert_formats(
      &in_fn(r#"match x { 1 -> "a", _ -> "b" }"#),
      &in_fn("match x {\n  1 -> \"a\",\n  _ -> \"b\",\n}"),
    );
    assert_formats(
      &in_fn("match x { 1 -> { let y = 2\n y }, _ -> 3 }"),
      &in_fn("match x {\n  1 -> {\n    let y = 2\n    y\n  }\n  _ -> 3,\n}"),
    );
  }

  #[test]
  fn comma_before_a_negative_pattern() {
    assert_formatted(&in_fn(
      "match x {\n  1 -> {\n    let y = 2\n    y\n  },\n  -1 -> 3,\n}",
    ));
  }

  #[test]
  fn else_if_chain_layout() {
    assert_formats(
      &in_fn(
        "if first_condition { first_value } else if second_condition { second_value } else { third }",
      ),
      &in_fn(
        "if first_condition {\n  first_value\n} else if second_condition {\n  second_value\n} else {\n  third\n}",
      ),
    );
  }

  #[test]
  fn leading_minus_statement_is_wrapped() {
    assert_formats(&in_fn("let a = 1\n(-a)"), &in_fn("let a = 1\n(-a)"));
    assert_formats(&in_fn("-a"), &in_fn("-a"));
  }
}

mod normalisation {
  use super::*;

  #[test]
  fn mangled_file() {
    let mangled = "module   Blog\ntypes\n      Status=Draft|Published(Int)\n\n\n\n   \
                   User={id:Int,name:String}derive(Json,Eq)\nfunctions\n title(p:{title:String|r})->String{\n\
                   p.title}\n\n\n\n  main(){let x={a:1,b:2}\n  Log.info(title({title:\"t\"}))\nmatch x{\
                   {a:a,..}->a,_->0}}\nexports\n  main";

    assert_formats(
      mangled,
      r#"module Blog

types
  Status = Draft | Published(Int)

  User = { id: Int, name: String } derive(Json, Eq)

functions
  title(p: { title: String | r }) -> String {
    p.title
  }

  main() {
    let x = { a: 1, b: 2 }
    Log.info(title({ title: "t" }))
    match x {
      { a: a, .. } -> a,
      _ -> 0,
    }
  }

exports
  main
"#,
    );
  }

  #[test]
  fn blank_lines_collapse() {
    assert_formats("exports\n  a\n\n\n\n\n  b\n", "exports\n  a\n\n  b\n");
    assert_formats(
      &in_fn("let a = 1\n\n\n\nlet b = 2\nb"),
      &in_fn("let a = 1\n\nlet b = 2\nb"),
    );
  }

  #[test]
  fn crlf_becomes_lf() {
    let out = fmt("functions\r\n  f() {\r\n    1\r\n  }\r\n\r\n\r\n");

    assert_eq!(out, "functions\n  f() {\n    1\n  }\n");
    assert!(!out.contains('\r'));
  }

  #[test]
  fn raw_text_survives() {
    assert_formatted(&in_fn("let x = 1_000\nlet s = \"\\u{e9}\"\ns"));
  }

  #[test]
  fn unparseable_file_is_refused() {
    let result = format("functions\n  f() {\n    let = 1\n  }\n", "test.px");

    assert_eq!(result.output, None);
    assert!(!result.is_ok());
    assert!(!result.diagnostics.is_empty());
  }
}

mod parens {
  use super::*;

  fn expr(src: &str, expected: &str) {
    assert_formats(&in_fn(src), &in_fn(expected));
  }

  #[test]
  fn lower_bp_child_keeps_parens() {
    expr("(a + b) * c", "(a + b) * c");
  }

  #[test]
  fn higher_bp_child_loses_them() {
    expr("a + (b * c)", "a + b * c");
  }

  #[test]
  fn left_assoc_right_side() {
    expr("a - (b - c)", "a - (b - c)");
  }

  #[test]
  fn left_assoc_left_side_is_bare() {
    expr("(a - b) - c", "a - b - c");
  }

  #[test]
  fn right_assoc_left_side() {
    expr("(a || b) || c", "(a || b) || c");
  }

  #[test]
  fn right_assoc_right_side_is_bare() {
    expr("a || (b || c)", "a || b || c");
  }

  #[test]
  fn non_assoc_both_sides() {
    expr("(a < b) == c", "(a < b) == c");
    expr("a == (b < c)", "a == (b < c)");
  }

  #[test]
  fn pipe_as_callee() {
    expr("(f |> g)(x)", "(f |> g)(x)");
  }

  #[test]
  fn binary_as_field_target() {
    expr("(a + b).c", "(a + b).c");
  }

  #[test]
  fn unary_as_callee() {
    expr("(-f)(x)", "(-f)(x)");
  }

  #[test]
  fn record_as_scrutinee() {
    expr(
      "match ({ a: 1 }) {\n  _ -> 1,\n}",
      "match ({ a: 1 }) {\n  _ -> 1,\n}",
    );
  }

  #[test]
  fn record_as_condition() {
    expr("if ({ a: 1 }) { 1 } else { 2 }", "if ({ a: 1 }) { 1 } else { 2 }");
    expr(
      "if (x == { a: 1 }) { 1 } else { 2 }",
      "if (x == { a: 1 }) { 1 } else { 2 }",
    );
  }

  #[test]
  fn fn_type_effect_capture() {
    assert_formatted(
      "functions\n  f() -> (function(B) -> C) / {E} {\n    g\n  }\n",
    );
    assert_formatted(
      "functions\n  f(g: function(A) -> (function(B) -> C) / {E}) -> Int {\n    1\n  }\n",
    );
  }

  #[test]
  fn readability_parens() {
    expr("if a { 1 } else { 2 } + 1", "(if a { 1 } else { 2 }) + 1");
  }

  #[test]
  fn nested_unary() {
    expr("- -x", "-(-x)");
  }
}

mod comments {
  use super::*;

  #[test]
  fn leading_comment_on_a_declaration() {
    assert_formatted("functions\n  // why\n  f() {\n    1\n  }\n");
    assert_formats(
      "functions\n// why\n        f() { 1 }\n",
      "functions\n  // why\n  f() {\n    1\n  }\n",
    );
  }

  #[test]
  fn comment_hoisted_out_of_a_zones_last_entry() {
    assert_formats(
      "constants\n  a = (-b // k0\n) // k1\n.a\n\nfunctions\n  f() {\n    1\n  }\n",
      "constants\n  a = (-b).a // k0\n  // k1\n\nfunctions\n  f() {\n    1\n  }\n",
    );
  }

  #[test]
  fn leading_blank_line_preserved() {
    assert_formatted("functions\n  // why\n\n  f() {\n    1\n  }\n");
  }

  #[test]
  fn leading_blank_lines_collapse() {
    assert_formats(
      "functions\n  // why\n\n\n\n  f() {\n    1\n  }\n",
      "functions\n  // why\n\n  f() {\n    1\n  }\n",
    );
  }

  #[test]
  fn trailing_same_line_comment() {
    assert_formatted(&in_fn("let x = 1 // why\nx"));
  }

  #[test]
  fn trailing_comment_forces_a_break() {
    assert_formats(&in_fn("{ a: 1 // why\n}"), &in_fn("{\n  a: 1, // why\n}"));
  }

  #[test]
  fn trailing_on_the_last_child() {
    assert_formatted(&in_fn("let x = 1\nx\n// done"));
  }

  #[test]
  fn dangling_in_an_empty_record() {
    assert_formatted(&in_fn("{ // nothing\n}"));
    assert_formats(&in_fn("{\n// nothing\n}"), &in_fn("{\n  // nothing\n}"));
    assert_formatted(&types("Empty = {\n  // nothing\n}"));
  }

  #[test]
  fn comment_between_arguments() {
    assert_formats(
      &in_fn("f(a, // why\n  b)"),
      &in_fn("f(\n  a, // why\n  b,\n)"),
    );
  }

  #[test]
  fn comment_between_match_arms() {
    assert_formatted(&in_fn("match x {\n  1 -> 2,\n  // why\n  _ -> 3,\n}"));
  }

  #[test]
  fn doc_comment_is_a_leading_comment() {
    assert_formatted("functions\n  /// doc\n  f() {\n    1\n  }\n");
  }

  #[test]
  fn header_comment_stays_above_the_header() {
    assert_formatted("// about\n\nmodule M\n\nexports\n  f\n");
  }

  #[test]
  fn comment_only_file() {
    assert_formatted("// one\n// two\n");
  }

  #[test]
  fn reproduction_a_comments_meeting_in_empty_braces() {
    assert_formats(
      "types\n  Foo // k0\n = { // k1\n}\n",
      "types\n  Foo = // k0\n  { // k1\n  }\n",
    );
  }

  #[test]
  fn reproduction_b_comment_pushed_off_between_entries() {
    assert_formats(
      "types\n  Foo<c, // k0\n f2> = None<Foo> // k1\n\n\n  B<x, y> = {}\n",
      "types\n  Foo<c, f2> = // k0\n  None<Foo> // k1\n\n  B<x, y> = {}\n",
    );
  }

  #[test]
  fn colliding_comment_goes_below_the_blank_line() {
    assert_formats(
      "types\n  A = X<Y // k0\n> // k1\n\n\n  B = {}\n",
      "types\n  A = X<Y> // k0\n\n  // k1\n  B = {}\n",
    );
  }

  #[test]
  fn colliding_comment_leads_the_next_statement() {
    assert_formats(
      &in_fn("let x = a // k0\n+ b // k1\nx"),
      &in_fn("let x = a + b // k0\n// k1\nx"),
    );
  }

  #[test]
  fn colliding_comment_stays_inside_the_brackets() {
    assert_formats(
      &in_fn("g(a // k0\n+ b // k1\n)"),
      &in_fn("g(\n  a + b, // k0\n  // k1\n)"),
    );
  }

  #[test]
  fn comment_after_an_opening_bracket_stays_there() {
    assert_formats(
      "types\n  A = function(B) -> C // k0\n  / // k1\n  {D}\n",
      "types\n  A = function(B) -> C / { // k0\n    // k1\n    D,\n  }\n",
    );
  }

  #[test]
  fn comment_before_a_function_body_moves_into_it() {
    assert_formats(
      "functions\n  f(a: Int) -> Int // why\n  {\n    a\n  }\n",
      "functions\n  f(a: Int) -> Int { // why\n    a\n  }\n",
    );
  }

  #[test]
  fn all_comments_survive_in_order() {
    let src = r"// header

module M

types
  /// doc on a type
  P = { // after brace
    a: Int, // after a
    // before b
    b: Int,
  }

  Empty = {
    // dangling
  }

functions
  /// doc on a function
  f(x: Int) -> Int {
    // leading a statement
    let y = g(x, // between arguments
      2)
    match y {
      1 -> 2, // after an arm
      // between arms
      _ -> 3,
    }
    // trailing, own line
  }

exports
  f // trailing an export
// end of file
";
    let out = fmt(src);

    assert_eq!(comment_texts(&out), comment_texts(src), "{out}");
    assert_eq!(fmt(&out), out, "not idempotent");
  }

  #[test]
  fn count_guard_ices() {
    let src = "functions\n  // why\n  f() {\n    1\n  }\n";
    let ice = expect_ice(|| {
      let _ = format_with(src, "test.px", |out| {
        *out = out.replace("  // why\n", "");
      });
    });

    assert!(ice.message.contains("comment count"), "{}", ice.message);
  }

  #[test]
  fn ast_is_not_mutated() {
    let src = "functions\n  // a\n  f() {\n    x + 1 // b\n  }\n";
    let file = SourceFile::new("test.px", src);
    let mut bag = DiagnosticBag::default();
    let lexed = lex(&file, &mut bag);
    let module = parse(&file, &lexed, &mut bag);

    let _ =
      print_module(&module, src, &lexed.comments, &PrecedenceTable::default());

    let mut fresh_bag = DiagnosticBag::default();
    let fresh = parse(&file, &lex(&file, &mut fresh_bag), &mut fresh_bag);

    assert_eq!(module, fresh);
  }
}

mod corpus {
  use super::*;
  use std::path::{Path, PathBuf};

  fn px_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<_> =
      entries.filter_map(Result::ok).map(|e| e.path()).collect();

    entries.sort();

    for path in entries {
      if path.is_dir() {
        px_files(&path, out);
      } else if path.extension().is_some_and(|e| e == "px") {
        out.push(path);
      }
    }
  }

  pub fn corpus() -> Vec<(PathBuf, String)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut paths = Vec::new();

    px_files(&root.join("tests/fixtures"), &mut paths);

    paths
      .into_iter()
      .map(|p| {
        let src = std::fs::read_to_string(&p).expect("read a corpus file");
        (p, src)
      })
      .filter(|(_, src)| format(src, "corpus.px").is_ok())
      .collect()
  }

  #[test]
  fn corpus_idempotence() {
    for (path, src) in corpus() {
      let once = fmt(&src);

      assert_eq!(fmt(&once), once, "{} is not a fixed point", path.display());
    }
  }

  #[test]
  fn corpus_preserves_meaning() {
    for (path, src) in corpus() {
      let parse_src = |text: &str| {
        let file = SourceFile::new("corpus.px", text);
        let mut bag = DiagnosticBag::default();
        let lexed = lex(&file, &mut bag);

        parse(&file, &lexed, &mut bag)
      };

      assert!(
        ast_eq(&parse_src(&src), &parse_src(&fmt(&src))),
        "{} changed meaning",
        path.display()
      );
    }
  }

  #[test]
  fn examples_are_included_when_present() {
    let files = corpus();
    let examples =
      Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/programs");

    assert_eq!(
      examples.is_dir(),
      files.iter().any(|(p, _)| p.starts_with(&examples)),
    );
    assert!(
      files.iter().any(|(p, _)| p.to_string_lossy().contains("fixtures"))
    );
  }
}
