use polar_compiler::{
  CompileOptions, Stage,
  check::exhaustive::{Pat, show},
  dump_stage, dump_stage_with,
  shared::diagnostic::{Diagnostic, Severity},
  shared::modules::ModuleSource,
  shared::source::SourceFile,
};
use std::{fs, path::Path, sync::Arc};

const PRELUDE: &str = "types
  List<a> = Nil | Cons(a, List<a>)
  Option<a> = None | Some(a)
  Result<e, a> = Err(e) | Ok(a)
  Status = Draft | Published(Int)
";

fn module(functions: &str) -> String {
  let indented: String =
    functions.lines().flat_map(|l| ["  ", l, "\n"]).collect();

  format!("{PRELUDE}\nfunctions\n{indented}")
}

fn diagnostics_of(src: &str) -> Vec<Diagnostic> {
  dump_stage(src, "test.px", Stage::Types).diagnostics
}

fn errors_of(src: &str) -> Vec<String> {
  let file = SourceFile::new("test.px", src);

  diagnostics_of(src)
    .iter()
    .map(|d| format!("{} at {:?}", d.code, file.slice(&d.primary.span)))
    .collect()
}

fn witness(src: &str) -> String {
  let diagnostics = diagnostics_of(src);

  assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
  assert_eq!(diagnostics[0].code.to_string(), "POLAR0601");

  diagnostics[0].primary.message.clone().unwrap_or_default()
}

fn ctor(name: &str, args: Vec<Pat>) -> Pat {
  Pat::Ctor { name: Arc::from(name), args, list: false }
}

#[test]
fn show_patterns() {
  assert_eq!(show(&Pat::Wild), "_");
  assert_eq!(show(&ctor("None", vec![])), "None");
  assert_eq!(show(&ctor("Some", vec![Pat::Bool(false)])), "Some(false)");
  assert_eq!(
    show(&Pat::Record(vec![
      (Arc::from("id"), Pat::Wild),
      (Arc::from("status"), ctor("Published", vec![Pat::Wild])),
    ])),
    "{ status: Published(_), .. }"
  );
  assert_eq!(
    show(&Pat::Ctor {
      name: Arc::from("Cons"),
      args: vec![
        Pat::Wild,
        Pat::Ctor { name: Arc::from("Nil"), args: vec![], list: true }
      ],
      list: true,
    }),
    "[_]"
  );
}

#[test]
fn all_ctors_covered() {
  let src = module("f(o: Option<Int>) { match o { Some(_) -> 1, None -> 0 } }");

  assert_eq!(errors_of(&src), Vec::<String>::new());
}

#[test]
fn missing_ctor() {
  let src = module("f(o: Option<Int>) { match o { Some(_) -> 1 } }");

  assert_eq!(witness(&src), "`None` is not covered");
  assert_eq!(errors_of(&src), vec!["POLAR0601 at \"match o\""]);
}

#[test]
fn missing_nested() {
  let src =
    module("f(o: Option<Bool>) { match o { Some(true) -> 1, None -> 0 } }");

  assert_eq!(witness(&src), "`Some(false)` is not covered");
}

#[test]
fn bool_covered() {
  let src = module("f(b: Bool) { match b { true -> 1, false -> 0 } }");

  assert_eq!(errors_of(&src), Vec::<String>::new());
}

#[test]
fn int_needs_wildcard() {
  let src = module("f(n: Int) { match n { 0 -> \"a\", 1 -> \"b\" } }");

  assert_eq!(witness(&src), "`_` is not covered");
}

#[test]
fn list_covered() {
  let src =
    module("f(xs: List<Int>) { match xs { [] -> 0, [x, ..rest] -> x } }");

  assert_eq!(errors_of(&src), Vec::<String>::new());
}

#[test]
fn list_missing_long() {
  let src = module("f(xs: List<Int>) { match xs { [] -> 0, [x] -> x } }");

  assert_eq!(witness(&src), "`[_, _, .._]` is not covered");
}

const POST: &str = "types
  Status = Draft | Published(Int)
  Post = { id: Int, title: String, status: Status }
";

fn describe(arms: &str) -> String {
  format!(
    "{POST}\nfunctions\n  describe(post: Post) -> String {{\n    match post {{\n{arms}    }}\n  }}\n"
  )
}

#[test]
fn record_fields() {
  let src = describe(
    "      { title: t, status: Draft, .. } -> \"#{t} — draft\",\n      { title: t, status: Published(year), .. } -> \"#{t} — #{year}\",\n",
  );

  assert_eq!(errors_of(&src), Vec::<String>::new());
}

#[test]
fn record_missing() {
  let src =
    describe("      { title: t, status: Draft, .. } -> \"#{t} — draft\",\n");

  assert!(witness(&src).contains("Published(_)"), "{}", witness(&src));
}

#[test]
fn unreachable_after_wildcard() {
  let src = module("f(s: Status) { match s { _ -> 1, Draft -> 2 } }");
  let result = dump_stage(&src, "test.px", Stage::Types);

  assert_eq!(errors_of(&src), vec!["POLAR0603 at \"Draft -> 2\""]);
  assert_eq!(result.diagnostics[0].severity, Severity::Warning);
  assert!(result.output.is_some());
}

#[test]
fn unreachable_duplicate() {
  let src = module(
    "f(o: Option<Int>) { match o { None -> 0, None -> 1, Some(_) -> 2 } }",
  );
  let diagnostics = diagnostics_of(&src);

  assert_eq!(errors_of(&src), vec!["POLAR0603 at \"None -> 1\""]);
  assert_eq!(diagnostics[0].secondary.len(), 1);
}

#[test]
fn refutable_let() {
  let src = module("f(o: Option<Int>) {\n  let Some(x) = o\n  x\n}");

  assert_eq!(errors_of(&src), vec!["POLAR0602 at \"Some(x)\""]);
}

#[test]
fn irrefutable_let_ok() {
  let src =
    module("f(r: { a: Int, b: Int }) {\n  let { a: x, .. } = r\n  x\n}");

  assert_eq!(errors_of(&src), Vec::<String>::new());
}

#[test]
fn nested_match_in_lambda() {
  let src = module(
    "f() {\n  let g = function(o: Option<Int>) { match o { Some(x) -> x } }\n  g(None)\n}",
  );

  assert_eq!(witness(&src), "`None` is not covered");
}

fn files_in(dir: &Path) -> Vec<std::path::PathBuf> {
  let mut out = Vec::new();

  for entry in fs::read_dir(dir).unwrap() {
    let path = entry.unwrap().path();

    if path.is_dir() {
      if path.file_name().is_some_and(|n| n != "dist") {
        out.extend(files_in(&path));
      }
    } else if path.extension().is_some_and(|e| e == "px") {
      out.push(path);
    }
  }

  out.sort();
  out
}

fn header(src: &str) -> Option<String> {
  src
    .lines()
    .find_map(|l| l.strip_prefix("module "))
    .map(|n| n.trim().to_string())
}

fn src_root(path: &Path) -> Option<&Path> {
  path
    .ancestors()
    .skip(1)
    .find(|dir| dir.parent().is_some_and(|p| p.join("polar.toml").exists()))
}

fn pascal_case(segment: &str) -> String {
  segment
    .split('_')
    .map(|part| {
      let mut chars = part.chars();

      chars
        .next()
        .map_or_else(String::new, |c| c.to_uppercase().chain(chars).collect())
    })
    .collect()
}

fn package_module(path: &Path) -> Option<String> {
  let manifest =
    fs::read_to_string(path.parent()?.parent()?.join("polar.toml")).ok()?;

  manifest
    .lines()
    .find_map(|l| l.trim().strip_prefix("module = "))
    .map(|m| m.trim().trim_matches('"').to_string())
}

#[test]
fn all_code_still_compiles() {
  let root = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/.."));
  let mut checked = 0;

  for dir in ["compiler/tests/fixtures/programs", "std", "projects"] {
    let files = files_in(&root.join(dir));
    let sources: Vec<(std::path::PathBuf, String)> = files
      .iter()
      .map(|p| (p.clone(), fs::read_to_string(p).unwrap()))
      .collect();

    let plugin_dirs: Vec<&Path> = sources
      .iter()
      .filter(|(_, src)| {
        polar_compiler::format(src, "test.px").diagnostics.iter().any(|d| {
          d.help.as_deref().is_some_and(|h| h.contains("is a plugin zone"))
        })
      })
      .filter_map(|(path, _)| path.parent())
      .collect();

    for (path, src) in &sources {
      if path.parent().is_some_and(|dir| plugin_dirs.contains(&dir)) {
        continue;
      }

      let package = package_module(path);
      let base = src_root(path).or(path.parent());
      let modules = sources
        .iter()
        .filter(|(other, _)| other != path)
        .filter_map(|(other, text)| {
          let dirs = other.parent()?.strip_prefix(base?).ok()?;

          if src_root(path).is_none() && !dirs.as_os_str().is_empty() {
            return None;
          }

          let name = header(text)?;
          let mut segments: Vec<String> =
            dirs.iter().map(|d| pascal_case(&d.to_string_lossy())).collect();

          segments.push(name);

          Some((segments.join("."), text))
        })
        .flat_map(|(name, text)| {
          let qualified = package.as_ref().map(|p| format!("{p}.{name}"));

          std::iter::once(name).chain(qualified).map(|path| ModuleSource {
            path,
            source: text.clone(),
            specifier: String::new(),
            plugins: Vec::new(),
          })
        })
        .collect();
      let options = CompileOptions { modules, ..CompileOptions::default() };
      let name = path.display().to_string();
      let result = dump_stage_with(src, &name, Stage::Types, &options);

      assert!(result.output.is_some(), "{name}: {:#?}", result.diagnostics);
      assert!(
        result.diagnostics.iter().all(|d| {
          !["POLAR0601", "POLAR0602"].contains(&d.code.to_string().as_str())
        }),
        "{name}: {:#?}",
        result.diagnostics
      );
      checked += 1;
    }
  }

  assert!(checked >= 10, "only {checked} files");
}
