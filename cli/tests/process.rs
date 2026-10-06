use std::{
  fs,
  path::{Path, PathBuf},
  process::{Command, Output},
};
use tempfile::TempDir;

const POLAR: &str = env!("CARGO_BIN_EXE_polar");

fn project(files: &[(&str, &str)]) -> TempDir {
  let dir = tempfile::tempdir().expect("create a temp dir");

  for (path, text) in files {
    let path = dir.path().join(path);

    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
  }

  dir
}

fn polar(
  cwd: &Path,
  tmp: &Path,
  args: &[&str],
  env: &[(&str, &str)],
) -> Output {
  let mut command = Command::new(POLAR);

  command
    .args(args)
    .current_dir(cwd)
    .env("TMPDIR", tmp)
    .env_remove("POLAR_DEBUG");

  for (k, v) in env {
    command.env(k, v);
  }

  command.output().expect("run polar")
}

struct Ran {
  code: Option<i32>,
  out: String,
  err: String,
}

fn run(files: &[(&str, &str)], file: &str, env: &[(&str, &str)]) -> Ran {
  let dir = project(files);
  let tmp = tempfile::tempdir().unwrap();
  let out = polar(dir.path(), tmp.path(), &["run", file], env);
  let ran = Ran {
    code: out.status.code(),
    out: String::from_utf8_lossy(&out.stdout).into_owned(),
    err: String::from_utf8_lossy(&out.stderr).into_owned(),
  };

  if !env.contains(&("POLAR_DEBUG", "1")) {
    assert_eq!(
      leftovers(tmp.path()),
      Vec::<PathBuf>::new(),
      "temp dirs remain"
    );
  }

  ran
}

fn leftovers(tmp: &Path) -> Vec<PathBuf> {
  fs::read_dir(tmp)
    .unwrap()
    .filter_map(Result::ok)
    .map(|e| e.path())
    .filter(|p| {
      p.file_name()
        .is_some_and(|n| n.to_string_lossy().starts_with("polar-run-"))
    })
    .collect()
}

const HELLO: &str = "functions\n  main() {\n    Log.info(\"hello, world\")\n  }\n\nexports\n  main\n";

const NESTED_FAILURE: &str = "functions
  pick(n) {
    String.repeat(\"one\", 9007199254740991 * (n - 1) + 1)
  }

  middle(n) {
    pick(n)
  }

  outer(n) {
    middle(n)
  }

  main() {
    Log.info(outer(1))
    Log.info(outer(2))
  }

exports
  main
";

#[test]
fn hello_runs() {
  let ran = run(&[("hello.px", HELLO)], "hello.px", &[]);

  assert_eq!(ran.code, Some(0), "{}", ran.err);
  assert_eq!(ran.out, "hello, world\n");
}

#[test]
fn output_streams_through() {
  let src = "functions\n  main() {\n    Log.info(\"one\")\n    Log.info(\"two\")\n    Log.info(\"three\")\n  }\n\nexports\n  main\n";
  let ran = run(&[("a.px", src)], "a.px", &[]);

  assert_eq!(ran.out, "one\ntwo\nthree\n");
}

#[test]
fn missing_main() {
  let src = "functions\n  main() {\n    Log.info(\"x\")\n  }\n";
  let ran = run(&[("a.px", src)], "a.px", &[]);

  assert_eq!(ran.code, Some(1));
  assert!(
    ran.err.contains("error: `a.px` does not export `main`"),
    "{}",
    ran.err
  );
  assert!(ran.err.contains("`exports` zone"), "{}", ran.err);
}

#[test]
fn compile_error_exits_1() {
  let ran =
    run(&[("a.px", "functions\n  main() {\n    let = 1\n  }\n")], "a.px", &[]);

  assert_eq!(ran.code, Some(1));
  assert!(ran.err.contains("error["), "{}", ran.err);
  assert!(!ran.err.contains("uncaught"), "Node ran: {}", ran.err);
}

#[test]
fn runtime_error_names_the_polar_line() {
  let ran = run(&[("src/nested.px", NESTED_FAILURE)], "src/nested.px", &[]);

  assert_eq!(ran.code, Some(1));
  assert_eq!(ran.out, "one\n");
  assert!(ran.err.starts_with("error: uncaught "), "{}", ran.err);
  assert!(ran.err.contains("nested.px:3:5)"), "{}", ran.err);
}

#[test]
fn stack_is_filtered() {
  let ran = run(&[("a.px", NESTED_FAILURE)], "a.px", &[]);

  for noise in ["runtime.js", "launcher.mjs", "node:internal", "main.js"] {
    assert!(!ran.err.contains(noise), "{noise} in:\n{}", ran.err);
  }
}

#[test]
fn three_frames_in_order() {
  let ran = run(&[("a.px", NESTED_FAILURE)], "a.px", &[]);
  let at = |needle: &str| {
    ran.err.find(needle).unwrap_or_else(|| panic!("{needle}:\n{}", ran.err))
  };

  assert!(
    at("at pick") < at("at middle") && at("at middle") < at("at outer"),
    "{}",
    ran.err
  );
  assert!(
    ran.err.contains("a.px:7:5)") && ran.err.contains("a.px:11:5)"),
    "{}",
    ran.err
  );
}

#[test]
fn unfiltered_fallback() {
  let dir = project(&[
    (
      "main.js",
      "export async function main() { throw new TypeError(\"synthetic\"); }\n",
    ),
    ("launcher.mjs", include_str!("../src/js/launcher.mjs")),
  ]);
  let node = std::env::var("POLAR_NODE").unwrap_or_else(|_| "node".to_string());
  let out = Command::new(node)
    .arg(dir.path().join("launcher.mjs"))
    .arg(dir.path().join("main.js"))
    .arg("a.px")
    .output()
    .unwrap();
  let err = String::from_utf8_lossy(&out.stderr);

  assert_eq!(out.status.code(), Some(1));
  assert!(err.contains("error: uncaught TypeError: synthetic"), "{err}");
  assert!(err.contains("main.js"), "no fallback stack: {err}");
}

#[test]
fn missing_node() {
  let ran =
    run(&[("a.px", HELLO)], "a.px", &[("POLAR_NODE", "/nonexistent/node")]);

  assert_eq!(ran.code, Some(1));
  assert!(
    ran.err.contains("Polar needs Node.js to run programs"),
    "{}",
    ran.err
  );
}

#[test]
fn exit_code_passthrough() {
  assert_eq!(run(&[("a.px", NESTED_FAILURE)], "a.px", &[]).code, Some(1));
}

#[test]
fn debug_keeps_the_temp_dir() {
  let dir = project(&[("a.px", HELLO)]);
  let tmp = tempfile::tempdir().unwrap();
  let out =
    polar(dir.path(), tmp.path(), &["run", "a.px"], &[("POLAR_DEBUG", "1")]);
  let err = String::from_utf8_lossy(&out.stderr);
  let kept = leftovers(tmp.path());

  assert_eq!(kept.len(), 1, "{err}");
  assert!(err.contains(&kept[0].display().to_string()), "{err}");
  assert!(kept[0].join("main.js").exists());
}

#[test]
fn panic_hook_is_quiet() {
  let dir = project(&[("a.px", HELLO)]);
  let tmp = tempfile::tempdir().unwrap();
  let out = polar(
    dir.path(),
    tmp.path(),
    &["build", "a.px"],
    &[("POLAR_INTERNAL_TEST_PANIC", "1")],
  );
  let err = String::from_utf8_lossy(&out.stderr);

  assert_eq!(out.status.code(), Some(2), "{err}");
  assert!(
    err.starts_with("error[POLAR0001]: internal compiler error: test panic"),
    "{err}"
  );
  assert!(!err.contains("panicked at"), "{err}");
}

#[test]
fn std_list_and_string_builtins_run() {
  let src = r##"uses
  Std.List

functions
  show(xs) -> String {
    List.join(List.map(xs, Int.to_string), ",")
  }

  main() {
    let xs = List.range(1, 6)
    let evens = List.filter(xs, function(n) { n % 2 == 0 })
    let sum = List.fold(xs, 0, function(acc, n) { acc + n })

    Log.info(show(xs))
    Log.info(show(evens))
    Log.info(show(List.reverse(xs)))
    Log.info(show(List.append(evens, Cons(9, Nil))))
    Log.info("#{List.length(xs)} #{sum} #{List.is_empty(Nil)} #{List.is_empty(xs)}")
    Log.info("  Hello, Polar  " |> String.trim |> String.uppercase)
    Log.info(String.slice("héllo", 1, 3))
    Log.info("#{String.length("😀ab")} #{String.contains("abc", "b")} #{String.starts_with("abc", "ab")}")
    Log.info(String.concat(String.repeat("ab", 2), String.lowercase("!X")))
  }

exports
  main
"##;
  let ran = run(&[("a.px", src)], "a.px", &[]);

  assert_eq!(ran.code, Some(0), "{}", ran.err);
  assert_eq!(
    ran.out,
    "1,2,3,4,5\n2,4\n5,4,3,2,1\n2,4,9\n5 15 true false\nHELLO, POLAR\nél\n3 true true\nabab!x\n"
  );
}

#[test]
fn std_option_and_result_run() {
  let src = r##"uses
  Std.List
  Std.Option
  Std.Result

functions
  show(option) -> String {
    match option {
      None -> "none",
      Some(x) -> "some #{x}",
    }
  }

  describe(result) -> String {
    match result {
      Err(e) -> "err #{e}",
      Ok(x) -> "ok #{x}",
    }
  }

  half(n: Int) -> Option<Int> {
    if n % 2 == 0 { Some(n / 2) } else { None }
  }

  checked(n: Int) -> Result<String, Int> {
    if n > 0 { Ok(n) } else { Err("not positive") }
  }

  main() {
    let xs = List.range(1, 6)
    let empty: List<Int> = Nil

    Log.info(show(List.head(xs)))
    Log.info(show(List.head(empty)))
    Log.info(show(List.nth(xs, 2)))
    Log.info(show(List.nth(xs, 9)))
    Log.info(show(List.find(xs, function(n) { n > 3 })))
    Log.info("#{List.any(xs, function(n) { n > 4 })} #{List.all(xs, function(n) { n > 4 })}")
    Log.info(List.join(List.concat_map([1, 2], function(n) { [Int.to_string(n), "."] }), ""))
    Log.info(show(Option.map(Some(2), function(n) { n * 10 })))
    Log.info(show(Option.and_then(Some(3), half)))
    Log.info(show(Option.and_then(Some(4), half)))
    Log.info("#{Option.with_default(None, 7)} #{Option.with_default(Some(1), 7)}")
    Log.info(describe(Result.map(checked(2), function(n) { n + 1 })))
    Log.info(describe(Result.map_err(checked(0), String.uppercase)))
    Log.info(describe(Result.and_then(checked(5), function(n) { checked(n - 5) })))
  }

exports
  main
"##;
  let ran = run(&[("a.px", src)], "a.px", &[]);

  assert_eq!(ran.code, Some(0), "{}", ran.err);
  assert_eq!(
    ran.out,
    "some 1\nnone\nsome 3\nnone\nsome 4\ntrue false\n1.2.\nsome 20\nnone\nsome 2\n7 1\nok 3\nerr NOT POSITIVE\nerr not positive\n"
  );
}

#[test]
fn std_math_constants_run() {
  let src = r##"uses
  Std.Math

constants
  tau = Math.pi * 2.0

functions
  main() {
    Log.info("#{Math.pi} #{tau}")
  }

exports
  main
"##;
  let ran = run(&[("a.px", src)], "a.px", &[]);

  assert_eq!(ran.code, Some(0), "{}", ran.err);
  assert_eq!(ran.out, "3.141592653589793 6.283185307179586\n");
}

#[test]
fn std_modules_bring_the_std_modules_they_import() {
  let src = r##"uses
  Std.List

functions
  main() {
    let first = List.head([1, 2])

    Log.info("#{List.length([first])}")
  }

exports
  main
"##;
  let ran = run(&[("a.px", src)], "a.px", &[]);

  assert_eq!(ran.code, Some(0), "{}", ran.err);
  assert_eq!(ran.out, "1\n");
}

#[test]
fn std_is_formatted() {
  let root = concat!(env!("CARGO_MANIFEST_DIR"), "/..");
  let tmp = tempfile::tempdir().unwrap();
  let out = polar(Path::new(root), tmp.path(), &["fmt", "std", "--check"], &[]);

  assert_eq!(
    out.status.code(),
    Some(0),
    "{}",
    String::from_utf8_lossy(&out.stdout)
  );
}

#[test]
fn projects_run() {
  let root = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/.."));
  let mut checked = 0;

  for entry in fs::read_dir(root.join("projects")).unwrap() {
    let dir = entry.unwrap().path();

    let manifest =
      fs::read_to_string(dir.join("polar.toml")).unwrap_or_default();

    if !manifest.contains("[project]") || dir.join("e2e.mjs").is_file() {
      continue;
    }

    let mut runs: Vec<(Option<String>, PathBuf)> = fs::read_dir(&dir)
      .unwrap()
      .filter_map(|entry| {
        let path = entry.ok()?.path();
        let file = path.file_name()?.to_str()?;
        let host = file.strip_prefix("main.")?.strip_suffix(".expected.txt")?;

        Some((Some(host.to_string()), path.clone()))
      })
      .collect();

    if runs.is_empty() {
      let shared = dir.join("main.expected.txt");
      let hosts = project_hosts(&dir);

      if hosts.is_empty() {
        runs.push((None, shared));
      } else {
        runs.extend(hosts.into_iter().map(|h| (Some(h), shared.clone())));
      }
    }

    for (host, expected) in runs {
      let mut args = vec!["run"];

      if let Some(host) = &host {
        args.extend(["--host", host.as_str()]);
      }

      let tmp = tempfile::tempdir().unwrap();
      let storage = tmp.path().join("storage.json");
      let out = polar(
        &dir,
        tmp.path(),
        &args,
        &[("POLAR_STORAGE_FILE", storage.to_str().unwrap())],
      );
      let name = format!("{} {host:?}", dir.display());

      assert_eq!(
        out.status.code(),
        Some(0),
        "{name}: {}",
        String::from_utf8_lossy(&out.stderr)
      );

      if let Ok(expected) = fs::read_to_string(expected) {
        assert_eq!(String::from_utf8_lossy(&out.stdout), expected, "{name}");
      }
    }

    checked += 1;
  }

  assert!(checked >= 1);
}

fn copy_tree(from: &Path, to: &Path) {
  fs::create_dir_all(to).unwrap();

  for entry in fs::read_dir(from).unwrap() {
    let path = entry.unwrap().path();
    let name = path.file_name().unwrap();

    if name == "dist" {
      continue;
    }

    if path.is_dir() {
      copy_tree(&path, &to.join(name));
    } else {
      fs::copy(&path, to.join(name)).unwrap();
    }
  }
}

fn files_under(dir: &Path) -> Vec<PathBuf> {
  let mut out = Vec::new();

  for entry in fs::read_dir(dir).unwrap() {
    let path = entry.unwrap().path();

    if path.is_dir() {
      out.extend(files_under(&path));
    } else {
      out.push(path);
    }
  }

  out
}

fn build_out_of_tree(project: &Path) -> (tempfile::TempDir, PathBuf) {
  let work = tempfile::tempdir().unwrap();
  let tmp = tempfile::tempdir().unwrap();
  let dist = work.path().join("dist");
  let built = polar(
    project,
    tmp.path(),
    &["build", "--out", dist.to_str().unwrap()],
    &[],
  );

  assert_eq!(
    built.status.code(),
    Some(0),
    "{}: {}",
    project.display(),
    String::from_utf8_lossy(&built.stderr)
  );

  (work, dist)
}

#[test]
fn runtime_tests_pass() {
  let node = std::env::var("POLAR_NODE").unwrap_or_else(|_| "node".to_string());
  let ran = Command::new(node)
    .args(["--test", "runtime.test.mjs"])
    .current_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/../runtime"))
    .output()
    .unwrap();

  assert!(
    ran.status.success(),
    "{}{}",
    String::from_utf8_lossy(&ran.stdout),
    String::from_utf8_lossy(&ran.stderr)
  );
}

#[test]
fn web_posts_bundles_hold_only_their_host() {
  let source =
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../projects/web_posts"));
  let (_work, dist) = build_out_of_tree(source);
  let browser = dist.join("Browser");
  let node = dist.join("Node");
  let names = |dir: &Path| -> Vec<String> {
    files_under(dir)
      .iter()
      .map(|p| p.strip_prefix(dir).unwrap().display().to_string())
      .collect()
  };

  for leak in ["posts.js", "users.js", "_polar/bridges.js"] {
    assert!(!names(&browser).contains(&leak.to_string()), "{leak} in Browser");
  }

  assert!(!names(&node).contains(&"dom.js".to_string()), "dom.js in Node");

  for file in files_under(&browser) {
    let text = fs::read_to_string(&file).unwrap_or_default();

    for leak in ["posts.js", "users.js", "$bridge$"] {
      assert!(!text.contains(leak), "{} contains `{leak}`", file.display());
    }
  }

  for file in files_under(&node) {
    let text = fs::read_to_string(&file).unwrap_or_default();

    assert!(!text.contains("dom.js"), "{} contains `dom.js`", file.display());
  }

  let server = fs::read_to_string(node.join("main.js")).unwrap();

  assert!(server.contains("function router("), "{server}");
  assert!(node.join("_deps/simple_framework/response.js").is_file());
}

#[test]
fn projects_run_end_to_end() {
  let root = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/.."));
  let node = std::env::var("POLAR_NODE").unwrap_or_else(|_| "node".to_string());
  let mut checked = 0;

  for entry in fs::read_dir(root.join("projects")).unwrap() {
    let source = entry.unwrap().path();

    if !source.join("e2e.mjs").is_file() {
      continue;
    }

    let (_work, dist) = build_out_of_tree(&source);
    let name = source.display().to_string();
    let ran = Command::new(&node)
      .arg("e2e.mjs")
      .arg(&dist)
      .current_dir(&source)
      .output()
      .unwrap();

    assert_eq!(
      ran.status.code(),
      Some(0),
      "{name}: {}",
      String::from_utf8_lossy(&ran.stderr)
    );
    assert_eq!(
      String::from_utf8_lossy(&ran.stdout),
      fs::read_to_string(source.join("e2e.expected.txt")).unwrap(),
      "{name}"
    );
    checked += 1;
  }

  assert!(checked >= 1);
}

#[test]
fn bridge_posts_browser_bundle_holds_no_server_code() {
  let source =
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../projects/bridge_posts"));
  let work = tempfile::tempdir().unwrap();
  let tmp = tempfile::tempdir().unwrap();
  let dir = work.path().join("project");

  copy_tree(source, &dir);

  let built = polar(&dir, tmp.path(), &["build"], &[]);

  assert_eq!(
    built.status.code(),
    Some(0),
    "{}",
    String::from_utf8_lossy(&built.stderr)
  );

  let browser = dir.join("dist/Browser");
  let names: Vec<String> = files_under(&browser)
    .iter()
    .map(|p| p.strip_prefix(&browser).unwrap().display().to_string())
    .collect();

  assert!(!names.iter().any(|n| n.contains("db.js")), "{names:?}");
  assert!(!names.iter().any(|n| n.contains("bridges.js")), "{names:?}");

  for file in files_under(&browser) {
    let text = fs::read_to_string(&file).unwrap_or_default();

    for leak in [
      "db.js",
      "$bridge$",
      "$bind$Db",
      "Db.find",
      "throw Missing",
      "Hello from the server",
    ] {
      assert!(!text.contains(leak), "{} contains `{leak}`", file.display());
    }
  }

  let server = fs::read_to_string(dir.join("dist/Node/main.js")).unwrap();

  assert!(server.contains("$bridge$Posts"), "{server}");
  assert!(dir.join("dist/Node/_polar/bridges.js").is_file());
  assert!(dir.join("dist/Node/_polar/bridges.d.ts").is_file());
  assert!(browser.join("_polar/std/bindings/Dom.js").is_file());
  assert!(!dir.join("dist/Node/_polar/std/Dom.js").exists());
  assert!(!dir.join("dist/Node/_polar/std/bindings").exists());
}

fn project_hosts(dir: &Path) -> Vec<String> {
  let text = fs::read_to_string(dir.join("polar.toml")).unwrap();
  let config: toml::Table = text.parse().unwrap();

  config
    .get("project")
    .and_then(|p| p.get("hosts"))
    .and_then(toml::Value::as_array)
    .into_iter()
    .flatten()
    .filter_map(|h| h.as_str().map(str::to_string))
    .collect()
}

#[test]
fn projects_are_formatted() {
  let root = concat!(env!("CARGO_MANIFEST_DIR"), "/..");
  let tmp = tempfile::tempdir().unwrap();
  let out =
    polar(Path::new(root), tmp.path(), &["fmt", "projects", "--check"], &[]);

  assert_eq!(
    out.status.code(),
    Some(0),
    "{}",
    String::from_utf8_lossy(&out.stdout)
  );
}

fn run_with_args(files: &[(&str, &str)], args: &[&str]) -> Ran {
  let dir = project(files);
  let tmp = tempfile::tempdir().unwrap();
  let out = polar(dir.path(), tmp.path(), args, &[]);

  Ran {
    code: out.status.code(),
    out: String::from_utf8_lossy(&out.stdout).into_owned(),
    err: String::from_utf8_lossy(&out.stderr).into_owned(),
  }
}

const UNCAUGHT: &str = include_str!("fixtures/run/uncaught.px");

#[test]
fn runs_throws_section_1() {
  let ran = run(&[("app.px", UNCAUGHT)], "app.px", &[]);

  assert_eq!(ran.out, "(empty)\n");
  assert_eq!(ran.err, "uncaught error: Empty\n");
  assert_eq!(ran.code, Some(1));
  assert!(!ran.err.contains("    at "), "a stack was printed: {}", ran.err);
}

#[test]
fn uncaught_fixture() {
  let ran = run(&[("uncaught.px", UNCAUGHT)], "uncaught.px", &[]);

  assert_eq!(ran.err, include_str!("fixtures/run/uncaught.stderr"));
}

const NOTES: &str = "module Notes

hosts
  DOM
  Node

effects
  Storage in DOM {
    get(key: String) -> String
  }

externs
  local_get(key: String) -> String = \"./dom.js\" get_item

binds
  Storage in DOM {
    get(key) {
      local_get(key)
    }
  }

  Storage in Node {
    get(key) {
      \"node:#{key}\"
    }
  }

functions
  main() {
    Log.info(Storage.get(\"k\"))
  }

exports
  main
";

const DOM_JS: &str = "export function get_item(key) {\n  console.log(\"dom.js saw \" + key);\n  return \"dom:\" + key;\n}\n";

#[test]
fn run_host_flag() {
  let ran = run_with_args(
    &[("src/notes.px", NOTES), ("src/dom.js", DOM_JS)],
    &["run", "--host", "Node", "src/notes.px"],
  );

  assert_eq!(ran.code, Some(0), "{}", ran.err);
  assert_eq!(ran.out, "node:k\n");
}

#[test]
fn relative_specifier_run() {
  let ran = run_with_args(
    &[("src/notes.px", NOTES), ("src/dom.js", DOM_JS)],
    &["run", "--host", "DOM", "src/notes.px"],
  );

  assert_eq!(ran.code, Some(0), "{}", ran.err);
  assert_eq!(ran.out, "dom.js saw k\ndom:k\n");
}

#[test]
fn run_needs_a_host() {
  let ran = run_with_args(&[("notes.px", NOTES)], &["run", "notes.px"]);

  assert_eq!(ran.code, Some(1));
  assert_eq!(
    ran.err,
    "error: this program declares hosts `DOM` and `Node`; choose one with `--host`\n"
  );
}

#[test]
fn run_unknown_host() {
  let ran = run_with_args(
    &[("notes.px", NOTES)],
    &["run", "--host", "Mars", "notes.px"],
  );

  assert_eq!(ran.code, Some(1));
  assert_eq!(
    ran.err,
    "error: unknown host `Mars`; this program declares `DOM`, `Node`\n"
  );
}

#[test]
fn relative_specifier_build() {
  let dir = project(&[("src/notes.px", NOTES), ("src/dom.js", DOM_JS)]);
  let tmp = tempfile::tempdir().unwrap();
  let out = polar(
    dir.path(),
    tmp.path(),
    &["build", "--host", "DOM", "--out", "dist", "src/notes.px"],
    &[],
  );

  assert_eq!(
    out.status.code(),
    Some(0),
    "{}",
    String::from_utf8_lossy(&out.stderr)
  );

  let js = fs::read_to_string(dir.path().join("dist/notes.js")).unwrap();

  assert!(js.contains("from \"./dom.js\""), "{js}");
  assert_eq!(
    fs::read_to_string(dir.path().join("dist/dom.js")).unwrap(),
    DOM_JS
  );
}

#[test]
fn extern_outside_the_source_root_is_referenced() {
  let notes = NOTES.replace("\"./dom.js\"", "\"../shared/dom.js\"");
  let dir = project(&[("src/notes.px", &notes), ("shared/dom.js", DOM_JS)]);
  let tmp = tempfile::tempdir().unwrap();
  let out = polar(
    dir.path(),
    tmp.path(),
    &["build", "--host", "DOM", "--out", "dist", "src"],
    &[],
  );

  assert_eq!(
    out.status.code(),
    Some(0),
    "{}",
    String::from_utf8_lossy(&out.stderr)
  );

  let js = fs::read_to_string(dir.path().join("dist/notes.js")).unwrap();

  assert!(js.contains("from \"../shared/dom.js\""), "{js}");
  assert!(!dir.path().join("dist/dom.js").exists());
}

#[test]
fn built_host_runs_on_its_own() {
  let dir = project(&[("src/notes.px", NOTES), ("src/dom.js", DOM_JS)]);
  let tmp = tempfile::tempdir().unwrap();
  let out = polar(
    dir.path(),
    tmp.path(),
    &["build", "--host", "DOM", "--out", "dist", "src"],
    &[],
  );

  assert_eq!(
    out.status.code(),
    Some(0),
    "{}",
    String::from_utf8_lossy(&out.stderr)
  );
  fs::remove_dir_all(dir.path().join("src")).unwrap();

  let node = std::env::var("POLAR_NODE").unwrap_or_else(|_| "node".to_string());
  let ran = Command::new(node)
    .args([
      "--input-type=module",
      "-e",
      "const m = await import(process.argv[1]); await m.main();",
    ])
    .arg(dir.path().join("dist/notes.js"))
    .output()
    .unwrap();

  assert_eq!(
    String::from_utf8_lossy(&ran.stdout),
    "dom.js saw k\ndom:k\n",
    "{}",
    String::from_utf8_lossy(&ran.stderr)
  );
}

#[test]
fn run_uses_the_only_configured_host() {
  let ran = run_with_args(
    &[
      ("polar.toml", "[project]\nname = \"n\"\nhosts = [\"Node\"]\n"),
      ("src/main.px", NOTES),
      ("src/dom.js", DOM_JS),
    ],
    &["run"],
  );

  assert_eq!(ran.code, Some(0), "{}", ran.err);
  assert_eq!(ran.out, "node:k\n");
}

#[test]
fn run_with_several_configured_hosts_asks_for_one() {
  let ran = run_with_args(
    &[
      ("polar.toml", "[project]\nname = \"n\"\nhosts = [\"DOM\", \"Node\"]\n"),
      ("src/main.px", NOTES),
    ],
    &["run"],
  );

  assert_eq!(ran.code, Some(1));
  assert_eq!(
    ran.err,
    "error: this project builds for hosts `DOM`, `Node`; choose one with `--host`\n"
  );
}

fn process_args() -> TempDir {
  let dir = tempfile::tempdir().unwrap();

  copy_tree(
    Path::new(concat!(
      env!("CARGO_MANIFEST_DIR"),
      "/tests/fixtures/process_args"
    )),
    dir.path(),
  );
  dir
}

fn in_project(dir: &Path, args: &[&str], env: &[(&str, &str)]) -> Ran {
  let tmp = tempfile::tempdir().unwrap();
  let out = polar(dir, tmp.path(), args, env);

  Ran {
    code: out.status.code(),
    out: String::from_utf8_lossy(&out.stdout).into_owned(),
    err: String::from_utf8_lossy(&out.stderr).into_owned(),
  }
}

fn built_process_args() -> TempDir {
  let dir = process_args();
  let built = in_project(dir.path(), &["build"], &[]);

  assert_eq!(built.code, Some(0), "{}", built.err);
  dir
}

fn node_start(dir: &Path, args: &[&str]) -> Ran {
  let node = std::env::var("POLAR_NODE").unwrap_or_else(|_| "node".to_string());
  let out = Command::new(node)
    .arg(dir.join("dist/start.mjs"))
    .args(args)
    .current_dir(dir)
    .output()
    .unwrap();

  Ran {
    code: out.status.code(),
    out: String::from_utf8_lossy(&out.stdout).into_owned(),
    err: String::from_utf8_lossy(&out.stderr).into_owned(),
  }
}

const PROCESS_PROGRAM: &str = "uses
  Std.List
  Std.Option
  Std.Process

hosts
  Node

functions
  main() -> {} / {Process} {
BODY
  }

exports
  Node
  main
";

fn run_process(body: &str) -> Ran {
  run(&[("a.px", &PROCESS_PROGRAM.replace("BODY", body))], "a.px", &[])
}

#[test]
fn args_after_dashdash_run() {
  let dir = process_args();
  let ran = in_project(dir.path(), &["run", "--", "a", "b c"], &[]);

  assert_eq!(ran.code, Some(0), "{}", ran.err);
  assert_eq!(ran.out, "[\"a\", \"b c\"]\n");
}

#[test]
fn args_none() {
  let dir = process_args();
  let ran = in_project(dir.path(), &["run"], &[]);

  assert_eq!(ran.code, Some(0), "{}", ran.err);
  assert_eq!(ran.out, "[]\n");
}

#[test]
fn args_start() {
  let dir = built_process_args();
  let ran = in_project(dir.path(), &["start", "--", "x"], &[]);

  assert_eq!(ran.code, Some(0), "{}", ran.err);
  assert_eq!(ran.out, "[\"x\"]\n");

  for args in [&["--", "x"][..], &["x"]] {
    let ran = node_start(dir.path(), args);

    assert_eq!(ran.code, Some(0), "{}", ran.err);
    assert_eq!(ran.out, "[\"x\"]\n", "{args:?}");
  }

  assert_eq!(node_start(dir.path(), &[]).out, "[]\n");
}

#[test]
fn args_custom_launcher() {
  let dir = project(&[
    (
      "kit/polar.toml",
      "[package]\nname = \"kit\"\nmodule = \"Kit\"\n\n[launcher]\nscript = \"launch.mjs\"\n",
    ),
    ("kit/launch.mjs", "console.log(process.argv.slice(3).join(\" \"));\n"),
    ("kit/src/.keep", ""),
    (
      "app/polar.toml",
      "[project]\nname = \"app\"\n\n[run]\nlauncher = \"kit\"\n\n[dependencies]\nkit = { path = \"../kit\" }\n",
    ),
    (
      "app/src/main.px",
      "module Main\n\nfunctions\n  main() {\n    Log.info(\"main ran\")\n  }\n\nexports\n  main\n",
    ),
  ]);
  let app = dir.path().join("app");
  let ran = in_project(&app, &["run", "--", "x"], &[]);

  assert_eq!((ran.code, ran.out.as_str()), (Some(0), "-- x\n"), "{}", ran.err);

  let built = in_project(&app, &["build"], &[]);

  assert_eq!(built.code, Some(0), "{}", built.err);

  let ran = in_project(&app, &["start", "--", "x"], &[]);

  assert_eq!((ran.code, ran.out.as_str()), (Some(0), "-- x\n"), "{}", ran.err);
}

#[test]
fn env_unset_vs_empty() {
  let dir = process_args();
  let tmp = tempfile::tempdir().unwrap();
  let out = Command::new(POLAR)
    .args(["run", "--", "env", "X", "Y"])
    .current_dir(dir.path())
    .env("TMPDIR", tmp.path())
    .env("X", "")
    .env_remove("Y")
    .output()
    .unwrap();

  assert_eq!(
    String::from_utf8_lossy(&out.stdout),
    "Some(\"\")\nNone\n",
    "{}",
    String::from_utf8_lossy(&out.stderr)
  );
}

#[test]
fn exit_code_set() {
  let dir = built_process_args();

  assert_eq!(
    in_project(dir.path(), &["run", "--", "exit", "3"], &[]).code,
    Some(3)
  );
  assert_eq!(
    in_project(dir.path(), &["start", "--", "exit", "3"], &[]).code,
    Some(3)
  );
  assert_eq!(node_start(dir.path(), &["exit", "3"]).code, Some(3));
}

#[test]
fn exit_code_default() {
  let dir = built_process_args();

  assert_eq!(in_project(dir.path(), &["run", "--", "a"], &[]).code, Some(0));
  assert_eq!(in_project(dir.path(), &["start", "--", "a"], &[]).code, Some(0));
}

#[test]
fn exit_code_uncaught() {
  let dir = built_process_args();

  for args in [&["run", "--", "throw"][..], &["start", "--", "throw"]] {
    let ran = in_project(dir.path(), args, &[]);

    assert_eq!(ran.code, Some(1), "{args:?}");
    assert_eq!(ran.err, "uncaught error: Failed(\"on purpose\")\n", "{args:?}");
  }
}

#[test]
fn run_inherits() {
  let ran = run_process(
    "    let code = Process.run(\"sh\", [\"-c\", \"echo hi; exit 4\"], Process.inherit())\n\n    Log.info(\"code #{code}\")",
  );

  assert_eq!(ran.code, Some(0), "{}", ran.err);
  assert_eq!(ran.out, "hi\ncode 4\n");
}

#[test]
fn run_in_dir_env() {
  let dir = project(&[
    (
      "a.px",
      &PROCESS_PROGRAM.replace(
        "BODY",
        "    let options = Process.inherit()\n      |> Process.in_dir(\"sub\")\n      |> Process.with_env(\"Z\", \"1\")\n\n    Process.set_exit_code(Process.run(\"sh\", [\"-c\", \"pwd; echo $Z\"], options))",
      ),
    ),
    ("sub/.keep", ""),
  ]);
  let ran = in_project(dir.path(), &["run", "a.px"], &[]);
  let sub = dir.path().join("sub").canonicalize().unwrap();

  assert_eq!(ran.code, Some(0), "{}", ran.err);
  assert_eq!(ran.out, format!("{}\n1\n", sub.display()));
}

#[test]
fn run_missing() {
  let ran = run_process(
    "    Log.info(\"#{Process.run(\"no-such-cmd\", [], Process.inherit())}\")\n    Log.info(\"#{Process.output(\"no-such-cmd\", [], Process.inherit()).code}\")",
  );

  assert_eq!(ran.code, Some(0), "{}", ran.err);
  assert_eq!(ran.out, "127\n127\n");
  assert_eq!(
    ran.err,
    "error: cannot run `no-such-cmd`: command not found\n".repeat(2)
  );
}

#[test]
fn output_captures() {
  let ran = run_process(
    "    let out = Process.output(\"sh\", [\"-c\", \"echo o; echo e >&2; exit 2\"], Process.inherit())\n\n    Log.info(\"#{out.code} #{String.length(out.stdout)} #{String.length(out.stderr)}\")\n    Log.info(String.concat(out.stdout, out.stderr))",
  );

  assert_eq!(ran.code, Some(0), "{}", ran.err);
  assert_eq!(ran.out, "2 2 2\no\ne\n\n");
  assert_eq!(ran.err, "");
}

#[test]
fn browser_rejected() {
  let src = "uses\n  Std.Dom\n  Std.Process\n\nhosts\n  Browser\n\nfunctions\n  main() {\n    Log.info(Process.cwd())\n  }\n\nexports\n  main\n";
  let ran =
    run_with_args(&[("a.px", src)], &["build", "a.px", "--out", "dist"]);

  assert_eq!(ran.code, Some(1));
  assert!(
    ran.err.contains("`main` can't run on `Browser`: it uses `Process`"),
    "{}",
    ran.err
  );
}
