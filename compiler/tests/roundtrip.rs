use polar_compiler::{
  fmt::{
    parens::{Side, binary_token, entry, exposes_record, operand_needs_parens},
    print::print_module,
  },
  format,
  shared::diagnostic::{Diagnostic, DiagnosticBag},
  shared::source::SourceFile,
  syntax::ast::{
    Block, Expr, Module,
    eq::{ast_diff, ast_eq},
    fields::{AsNode, NodeRef, children},
  },
  syntax::lexer::{
    lex,
    token::{Comment, TokenKind},
  },
  syntax::parser::{
    ParseOptions, parse_with,
    precedence::{Assoc, InfixEntry, PrecedenceTable},
  },
};
use proptest::{
  prelude::*,
  strategy::ValueTree,
  test_runner::{
    Config, FileFailurePersistence, TestCaseError, TestError, TestRunner,
  },
};
use std::{collections::BTreeSet, fmt::Write as _};

mod common;

use common::{arbitrary, invariants::check_invariants};

fn print(ast: &Module) -> String {
  print_module(ast, "", &[], &PrecedenceTable::default())
}

fn parse_src(src: &str, table: &PrecedenceTable) -> (Module, Vec<Diagnostic>) {
  let file = SourceFile::new("gen.px", src);
  let mut bag = DiagnosticBag::default();
  let lexed = lex(&file, &mut bag);
  let options = ParseOptions { precedence: table.clone() };
  let module = parse_with(&file, &lexed, &mut bag, &options);

  (module, bag.into_sorted())
}

fn round_trips(
  ast: &Module,
  table: &PrecedenceTable,
) -> Result<(), TestCaseError> {
  let src = print(ast);
  let (reparsed, diagnostics) = parse_src(&src, table);

  prop_assert!(
    diagnostics.is_empty(),
    "the printed program does not parse:\n{src}\n{diagnostics:#?}"
  );

  if let Some(diff) = ast_diff(ast, &reparsed) {
    return Err(TestCaseError::fail(format!(
      "the printed program means something else: {diff}\n{src}"
    )));
  }

  Ok(())
}

fn cases() -> u32 {
  std::env::var("PROPTEST_CASES")
    .ok()
    .and_then(|n| n.parse().ok())
    .unwrap_or(1000)
}

fn config() -> ProptestConfig {
  ProptestConfig {
    cases: cases(),
    failure_persistence: Some(Box::new(FileFailurePersistence::WithSource(
      "proptest-regressions",
    ))),
    ..ProptestConfig::default()
  }
}

fn samples() -> Vec<Module> {
  let mut runner = TestRunner::deterministic();
  let strategy = arbitrary::module();

  (0..200)
    .map(|_| {
      strategy.new_tree(&mut runner).expect("generate a module").current()
    })
    .collect()
}

fn nodes(root: &Module) -> Vec<NodeRef<'_>> {
  let mut out = Vec::new();
  let mut work = vec![root.as_node()];

  while let Some(node) = work.pop() {
    out.push(node);
    work.extend(children(node));
  }

  out
}

#[test]
fn sample_is_parseable() {
  for ast in samples() {
    let src = print(&ast);
    let (_, diagnostics) = parse_src(&src, &PrecedenceTable::default());

    assert!(diagnostics.is_empty(), "{src}\n{diagnostics:#?}");
  }
}

#[test]
fn kind_histogram_is_complete() {
  let samples = samples();
  let seen: BTreeSet<&str> =
    samples.iter().flat_map(nodes).map(|node| node.kind()).collect();

  let expected = [
    "IntLit",
    "FloatLit",
    "BoolLit",
    "StringLit",
    "StringInterp",
    "Var",
    "FieldAccess",
    "Call",
    "Pipe",
    "Binary",
    "Unary",
    "RecordLit",
    "Lambda",
    "Block",
    "If",
    "Match",
    "LetStmt",
    "ExprStmt",
    "PWildcard",
    "PVar",
    "PLit",
    "PCtor",
    "PRecord",
    "PField",
    "TypeRef",
    "TypeVar",
    "FnType",
    "RecordType",
    "FieldType",
    "EffectRow",
    "Import",
    "TypeDecl",
    "VariantBody",
    "CtorDecl",
    "Derive",
    "FnDecl",
    "Param",
    "ExportDecl",
    "TraitDecl",
    "MethodSig",
    "Recipe",
    "ImplDecl",
    "Bound",
  ];
  let missing: Vec<_> =
    expected.iter().filter(|k| !seen.contains(*k)).collect();

  assert!(missing.is_empty(), "never generated: {missing:?}");
}

#[test]
fn numbers_are_well_formed() {
  for ast in samples() {
    for node in nodes(&ast) {
      let (raw, kind) = match node {
        NodeRef::IntLit(l) => (&l.raw, TokenKind::Int),
        NodeRef::FloatLit(l) => (&l.raw, TokenKind::Float),
        _ => continue,
      };

      let file = SourceFile::new("n.px", raw.as_str());
      let mut bag = DiagnosticBag::default();
      let tokens = lex(&file, &mut bag).tokens;

      assert!(!bag.has_errors(), "{raw}: {:#?}", bag.into_sorted());
      assert_eq!(tokens.len(), 2, "{raw} is not one token");
      assert_eq!(tokens[0].kind, kind, "{raw}");
    }
  }
}

#[test]
fn string_values_match_their_raw() {
  for ast in samples() {
    for node in nodes(&ast) {
      let NodeRef::StringText(text) = node else { continue };
      let src = format!("\"{}\"", text.raw);
      let file = SourceFile::new("s.px", src.as_str());
      let mut bag = DiagnosticBag::default();
      let tokens = lex(&file, &mut bag).tokens;

      assert!(!bag.has_errors(), "{src}");
      assert_eq!(tokens[1].kind, TokenKind::StringPart, "{src}");
      assert_eq!(
        tokens[1].value.as_deref(),
        Some(text.value.as_str()),
        "{src}"
      );
    }
  }
}

#[test]
fn blocks_are_never_empty() {
  for ast in samples() {
    for node in nodes(&ast) {
      if let NodeRef::Block(Block { result, .. }) = node {
        assert!(!matches!(**result, Expr::Invalid(_)));
      }
    }
  }
}

#[test]
fn ambiguities_are_generated() {
  let table = PrecedenceTable::default();
  let (mut clashes, mut conditions) = (0, 0);

  for ast in samples() {
    for node in nodes(&ast) {
      match node {
        NodeRef::Binary(b) => {
          let parent = entry(&table, binary_token(b.op));
          let clash = |child: &Expr, side| {
            matches!(child, Expr::Binary(_) | Expr::Pipe(_))
              && operand_needs_parens(parent, child, side, &table)
          };

          clashes += usize::from(
            clash(&b.left, Side::Left) || clash(&b.right, Side::Right),
          );
        }
        NodeRef::If(i) => conditions += usize::from(exposes_record(&i.cond)),
        NodeRef::Match(m) => {
          conditions += m.subjects.iter().filter(|s| exposes_record(s)).count();
        }
        _ => {}
      }
    }
  }

  assert!(clashes > 0, "no precedence clash needing parentheses");
  assert!(conditions > 0, "no record literal in a condition");
}

proptest! {
  #![proptest_config(config())]

  #[test]
  fn p1_round_trip(ast in arbitrary::module()) {
    round_trips(&ast, &PrecedenceTable::default())?;
  }

  #[test]
  fn p2_idempotence(ast in arbitrary::module()) {
    let once = print(&ast);
    let (reparsed, _) = parse_src(&once, &PrecedenceTable::default());
    let twice = print(&reparsed);

    prop_assert_eq!(twice, once);
  }

  #[test]
  fn p3_spans(ast in arbitrary::module()) {
    let src = print(&ast);
    let (reparsed, _) = parse_src(&src, &PrecedenceTable::default());

    check_invariants(&src, &reparsed);
  }

  #[test]
  fn p4_comment_injection(
    file in 0..corpus::files().len(),
    marks in prop::collection::vec(any::<bool>(), 0..120),
  ) {
    let (path, src) = &corpus::files()[file];
    let injected = corpus::inject_comments(src, &marks);
    let result = format(&injected, "injected.px");
    let out = result.output.as_ref().ok_or_else(|| {
      TestCaseError::fail(format!("{} no longer formats:\n{injected}", path.display()))
    })?;

    prop_assert_eq!(corpus::comment_texts(out), corpus::comment_texts(&injected));
  }

  #[test]
  fn p4b_comments_anywhere(
    ast in arbitrary::module(),
    picks in prop::collection::vec(any::<prop::sample::Index>(), 1..12),
  ) {
    let src = inject_after_tokens(&print(&ast), &picks);

    if let Some(out) = format_or_fail(&src)?.output {
      prop_assert_eq!(
        corpus::comment_texts(&out),
        corpus::comment_texts(&src),
        "input:\n{}\noutput:\n{}",
        src,
        out
      );

      let again = format_or_fail(&out)?.output;

      prop_assert_eq!(again.as_deref(), Some(out.as_str()), "input:\n{}", src);
    }
  }
}

fn format_or_fail(
  src: &str,
) -> Result<polar_compiler::FormatResult, TestCaseError> {
  std::panic::catch_unwind(|| format(src, "fuzz.px")).map_err(|payload| {
    let message = payload
      .downcast_ref::<polar_compiler::shared::ice::InternalCompilerError>()
      .map_or_else(|| "a panic".to_string(), |ice| ice.message.clone());

    TestCaseError::fail(format!("{message}\n{src}"))
  })
}

fn inject_after_tokens(src: &str, picks: &[prop::sample::Index]) -> String {
  let file = SourceFile::new("fuzz.px", src);
  let mut bag = DiagnosticBag::default();
  let tokens = lex(&file, &mut bag).tokens;
  let mut cuts: Vec<usize> =
    picks.iter().map(|i| tokens[i.index(tokens.len())].span.end).collect();

  cuts.sort_unstable();
  cuts.dedup();

  let mut out = String::new();
  let mut last = 0;

  for (n, cut) in cuts.iter().enumerate() {
    out.push_str(&src[last..*cut]);
    let _ = writeln!(out, " // k{n}");
    last = *cut;
  }

  out.push_str(&src[last..]);
  out
}

#[test]
fn p5_mutation_is_caught() {
  let mutated = PrecedenceTable::default()
    .with(TokenKind::Plus, InfixEntry { bp: 60, assoc: Assoc::Left })
    .with(TokenKind::Star, InfixEntry { bp: 50, assoc: Assoc::Left });
  let mut runner = TestRunner::new(Config {
    cases: cases(),
    failure_persistence: None,
    ..Config::default()
  });

  let result =
    runner.run(&arbitrary::module(), |ast| round_trips(&ast, &mutated));

  assert!(
    matches!(result, Err(TestError::Fail(..))),
    "the mutated parser passed P1: {result:?}"
  );
}

#[test]
fn mutation_changes_meaning() {
  let mutated = PrecedenceTable::default()
    .with(TokenKind::Plus, InfixEntry { bp: 60, assoc: Assoc::Left })
    .with(TokenKind::Star, InfixEntry { bp: 50, assoc: Assoc::Left });
  let src = "functions\n  f() {\n    a + b * c\n  }\n";
  let (normal, _) = parse_src(src, &PrecedenceTable::default());
  let (swapped, _) = parse_src(src, &mutated);

  assert!(!ast_eq(&normal, &swapped));
}

mod corpus {
  use super::*;
  use std::{
    path::{Path, PathBuf},
    sync::OnceLock,
  };

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

  pub fn files() -> &'static [(PathBuf, String)] {
    static FILES: OnceLock<Vec<(PathBuf, String)>> = OnceLock::new();

    FILES.get_or_init(|| {
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
    })
  }

  pub fn inject_comments(src: &str, marks: &[bool]) -> String {
    let mut out = String::new();
    let mut n = 0;

    for (i, line) in src.split_inclusive('\n').enumerate() {
      let (body, end) = match line.strip_suffix('\n') {
        Some(body) => (body, "\n"),
        None => (line, ""),
      };

      out.push_str(body);

      if !end.is_empty() && marks.get(i).copied().unwrap_or(false) {
        let _ = write!(out, " // c{n}");
        n += 1;
      }

      out.push_str(end);
    }

    out
  }

  pub fn comment_texts(src: &str) -> Vec<String> {
    let file = SourceFile::new("c.px", src);
    let mut bag = DiagnosticBag::default();

    lex(&file, &mut bag)
      .comments
      .iter()
      .map(|c: &Comment| src[c.span.start..c.span.end].trim_end().to_string())
      .collect()
  }
}
