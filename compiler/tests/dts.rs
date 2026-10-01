mod common;

use std::{path::PathBuf, process::Command};

use common::node::TempDir;
use polar_compiler::{CompileOptions, HostOption, compile, stdlib};

fn output(src: &str, host: Option<&str>) -> (String, String) {
  let out = compile(
    src,
    "app.px",
    &CompileOptions {
      host: host
        .map_or(HostOption::Auto, |h| HostOption::Fixed(Some(h.to_string()))),
      ..CompileOptions::default()
    },
  );

  assert!(out.diagnostics.is_empty(), "{src}\n{:#?}", out.diagnostics);
  (out.js, out.dts)
}

fn dts(src: &str) -> String {
  output(src, None).1
}

fn exported(fns: &str) -> String {
  let names: Vec<&str> = fns
    .lines()
    .filter(|l| l.starts_with("  ") && !l.starts_with("   ") && l.contains('('))
    .filter_map(|l| l.trim().split('(').next())
    .collect();
  let exports: String = names.iter().flat_map(|n| ["  ", n, "\n"]).collect();

  format!("functions\n{fns}\nexports\n{exports}")
}

const CLOCK: &str = "hosts\n  Node\n\neffects\n  Clock in Node {\n    now() -> Int\n  }\n\nbinds\n  Clock in Node {\n    now() {\n      1\n    }\n  }\n\n";

#[test]
fn primitives_and_a_pure_function() {
  assert_eq!(
    dts(&exported(
      "  f(a: Int, b: String, c: Float) -> Bool {\n    true\n  }\n"
    )),
    "export declare function f(a: number, b: string, c: number): boolean;\n\n"
  );
}

#[test]
fn effectful_export_returns_a_promise() {
  let src = format!(
    "{CLOCK}{}",
    exported("  f(a: Int) -> Int / {Clock} {\n    a + Clock.now()\n  }\n")
  );
  let (js, dts) = output(&src, Some("Node"));

  assert!(js.contains("export async function f(a)"), "{js}");
  assert!(
    dts.contains("export declare function f(a: number): Promise<number>;"),
    "{dts}"
  );
}

#[test]
fn throws_alone_is_not_a_promise() {
  let src = format!(
    "types\n  Oops = Oops(String)\n\n{}",
    exported(
      "  f(a: Int) -> Int / {Throws<Oops>} {\n    if a == 0 { throw Oops(\"zero\") } else { a }\n  }\n"
    )
  );

  assert!(
    dts(&src).contains("export declare function f(a: number): number;"),
    "{}",
    dts(&src)
  );
}

#[test]
fn effect_polymorphic_export_has_three_signatures() {
  let text = dts(&exported(
    "  apply(f: function(Int) -> Int / {| e}, x: Int) -> Int / {| e} {\n    f(x)\n  }\n",
  ));

  assert_eq!(
    text,
    "export declare function apply(f: (p0: number) => number, x: number): number;\n\
     export declare function apply$sync(f: (p0: number) => number, x: number): number;\n\
     export declare function apply$async(f: (p0: number) => number | Promise<number>, x: number): Promise<number>;\n\n"
  );
}

#[test]
fn variants_are_tagged_unions() {
  let text = dts(
    "types\n  Status = Draft | Published(Int)\n  Pair<a, b> = Pair(a, b)\n",
  );

  assert!(
    text.contains(
      "export type Status =\n  | { $: \"Draft\" }\n  | { $: \"Published\"; _0: number };"
    ),
    "{text}"
  );
  assert!(
    text
      .contains("export type Pair<a, b> =\n  | { $: \"Pair\"; _0: a; _1: b };"),
    "{text}"
  );
}

#[test]
fn record_aliases_and_open_rows() {
  let src = format!(
    "types\n  Post = {{ title: String, tags: List<String> }}\n\nuses\n  Std.List\n\n{}",
    exported(
      "  title_of(r: { title: String | rest }) -> String {\n    r.title\n  }\n\n  first(p: Post) -> Post {\n    p\n  }\n"
    )
  )
  .replacen("types\n  Post", "uses\n  Std.List\n\ntypes\n  Post", 1)
  .replacen("\nuses\n  Std.List\n\nfunctions", "\nfunctions", 1);
  let text = dts(&src);

  assert!(
    text.contains("import type { List } from \"./_polar/std/List.js\";"),
    "{text}"
  );
  assert!(
    text.contains("export type Post = { title: string; tags: List<string> };"),
    "{text}"
  );
  assert!(
    text.contains(
      "export declare function title_of<a>(r: { title: string } & a): string;"
    ),
    "{text}"
  );
  assert!(
    text.contains("export declare function first(p: Post): Post;"),
    "{text}"
  );
}

#[test]
fn generic_export() {
  let text = dts(&exported("  pick(a, b) {\n    b\n  }\n"));

  assert_eq!(text, "export declare function pick<a, b>(a: a, b: b): b;\n\n");
}

#[test]
fn constrained_export_takes_its_dictionary_first() {
  let text = dts(&exported(
    "  label(x: a) -> String where Show<a> {\n    \"#{x}\"\n  }\n",
  ));

  assert!(
    text.contains(
      "export declare function label<a>($d0: { show: (p0: a) => string }, x: a): string;"
    ),
    "{text}"
  );
}

#[test]
fn constants() {
  let text = dts(
    "constants\n  pi = 3.14\n  name = \"polar\"\n\nexports\n  pi\n  name\n",
  );

  assert!(text.contains("export declare const pi: number;"), "{text}");
  assert!(text.contains("export declare const name: string;"), "{text}");
}

#[test]
fn private_functions_and_dictionaries_are_left_out() {
  let text = dts(
    "types\n  R = { x: Int } derive(Eq)\n\nfunctions\n  hidden() {\n    1\n  }\n",
  );

  assert!(!text.contains("hidden"), "{text}");
  assert!(!text.contains("$Eq"), "{text}");
}

#[test]
fn std_declarations_match_their_javascript() {
  for m in stdlib::MODULES {
    let out = compile(
      m.source,
      &stdlib::filename(m.name),
      &CompileOptions {
        runtime: "../runtime.js".to_string(),
        ..CompileOptions::default()
      },
    );

    if !out.diagnostics.is_empty() {
      continue;
    }

    for line in out.dts.lines() {
      let Some(rest) = line.strip_prefix("export declare function ") else {
        continue;
      };
      let name = rest.split(['<', '(']).next().unwrap();
      let promise =
        line.ends_with("): Promise<") || line.contains("): Promise<");
      let asynchronous =
        out.js.contains(&format!("export async function {name}("));

      assert_eq!(promise, asynchronous, "{}: {line}\n{}", m.name, out.js);
      assert!(
        out.js.contains(&format!("function {name}(")),
        "{}: {name} is declared but not emitted",
        m.name
      );
    }
  }
}

fn tsc() -> Option<PathBuf> {
  if let Some(path) = std::env::var_os("POLAR_TSC") {
    return Some(PathBuf::from(path));
  }

  Command::new("tsc")
    .arg("--version")
    .output()
    .is_ok_and(|o| o.status.success())
    .then(|| PathBuf::from("tsc"))
}

#[test]
fn blog_declarations_pass_tsc_strict() {
  let Some(tsc) = tsc() else {
    eprintln!("skipped: no `tsc` on PATH and POLAR_TSC unset");
    return;
  };
  let blog = std::fs::read_to_string(
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
      .join("tests/fixtures/programs/blog.px"),
  )
  .unwrap();
  let dir = TempDir::new();
  let std_dir = dir.path.join("_polar/std");
  let out = compile(&blog, "blog.px", &CompileOptions::default());

  assert!(out.diagnostics.is_empty(), "{:#?}", out.diagnostics);
  std::fs::create_dir_all(&std_dir).unwrap();
  std::fs::write(dir.path.join("blog.d.ts"), &out.dts).unwrap();

  for m in stdlib::MODULES {
    let std = compile(
      m.source,
      &stdlib::filename(m.name),
      &CompileOptions {
        runtime: "../runtime.js".to_string(),
        ..CompileOptions::default()
      },
    );

    if std.diagnostics.is_empty() {
      std::fs::write(std_dir.join(format!("{}.d.ts", m.name)), &std.dts)
        .unwrap();
    }
  }

  std::fs::write(
    dir.path.join("use.ts"),
    "import { title_of, slug, main, type Post, type Status } from \"./blog.js\";\n\
     import { map, map$async, fold, range } from \"./_polar/std/List.js\";\n\
     const draft: Status = { $: \"Draft\" };\n\
     const post: Post = { id: 1, title: \"Hi\", body: \"b\", author_id: 2, status: draft };\n\
     const t: string = title_of(post);\n\
     const s: string = slug({ title: \"A B\", extra: 1 });\n\
     const unit: Record<string, never> = main();\n\
     const doubled = map(range(0, 3), (x) => x * 2);\n\
     const total: number = fold(doubled, 0, (a, b) => a + b);\n\
     const later: Promise<unknown> = map$async(range(0, 2), async (x) => x + 1);\n\
     void [t, s, unit, total, later];\n",
  )
  .unwrap();

  let ran = Command::new(tsc)
    .current_dir(&dir.path)
    .args([
      "--strict",
      "--noEmit",
      "--module",
      "nodenext",
      "--moduleResolution",
      "nodenext",
      "--target",
      "es2022",
      "use.ts",
    ])
    .output()
    .expect("spawn tsc");

  assert!(
    ran.status.success(),
    "{}\n{}\n{}",
    String::from_utf8_lossy(&ran.stdout),
    String::from_utf8_lossy(&ran.stderr),
    out.dts
  );
}
