mod common;

use std::fmt::Write;

use polar_compiler::{
  CompileOptions, HostOption, compile, shared::diagnostic::Severity,
};

const CASES: &[(&str, &str)] = &[
  ("normalize(\"a/./b/../c//d/\")", "a/c/d"),
  ("normalize(\"/../x\")", "/x"),
  ("normalize(\"\")", "."),
  ("normalize(\"../a/../../b\")", "../../b"),
  ("normalize(\"/\")", "/"),
  ("join(\"a\", \"/b\")", "/b"),
  ("join(\"a/\", \"b\")", "a/b"),
  ("join(\"\", \"b\")", "b"),
  ("relative(\"/w/ticket\", \"/w/apps/demo\")", "../apps/demo"),
  ("relative(\"/a\", \"/a\")", "."),
  ("relative(\"a/b\", \"a/b/c\")", "c"),
  ("dirname(\"/a/b.px\")", "/a"),
  ("dirname(\"b.px\")", "."),
  ("dirname(\"/b.px\")", "/"),
  ("dirname(\"a/b/\")", "a"),
  ("basename(\"/a/b.px\")", "b.px"),
  ("basename(\"a/b/\")", "b"),
  ("extension(\"/a/b.px\")", ".px"),
  ("extension(\".gitignore\")", ""),
  ("extension(\"a.tar.gz\")", ".gz"),
  ("extension(\"a\")", ""),
  ("is_absolute(\"/a\")", "true"),
  ("is_absolute(\"a\")", "false"),
];

fn program() -> String {
  let lines = CASES.iter().fold(String::new(), |mut acc, (call, _)| {
    writeln!(acc, "    Log.info(\"[#{{Path.{call}}}]\")").unwrap();
    acc
  });

  format!(
    "module App\n\nuses\n  Std.Path\n\nfunctions\n  main() {{\n{lines}  }}\n\nexports\n  main\n"
  )
}

#[test]
fn path_table() {
  let out = common::node::run_program(&program(), "app.px", &[])
    .unwrap_or_else(|e| panic!("{e}"));
  let expected: Vec<String> =
    CASES.iter().map(|(_, want)| format!("[{want}]")).collect();
  let got: Vec<&str> = out.lines().collect();

  for ((call, _), (got, want)) in CASES.iter().zip(got.iter().zip(&expected)) {
    assert_eq!(got, want, "Path.{call}");
  }
  assert_eq!(got.len(), expected.len(), "{out}");
}

#[test]
fn path_in_browser() {
  let src = "module App\n\nuses\n  Std.Path\n\nhosts\n  Browser\n\nfunctions\n  main() {\n    Log.info(Path.join(\"a\", \"b\"))\n  }\n\nexports\n  main\n";
  let options = CompileOptions {
    host: HostOption::Fixed(Some("Browser".to_string())),
    ..CompileOptions::default()
  };
  let out = compile(src, "app.px", &options);

  assert!(
    out.diagnostics.iter().all(|d| d.severity != Severity::Error),
    "{:#?}",
    out.diagnostics
  );
  assert!(out.std_imports.contains(&"Path".to_string()));
  assert_eq!(
    common::node::run_program_with(src, "app.px", &[], Some("Browser"), &[]),
    Ok("a/b\n".to_string())
  );
}
