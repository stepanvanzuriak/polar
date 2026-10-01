use polar_cli::{
  Compiler,
  watch::{CycleResult, Watch, WatchMode, WatchOptions},
};
use std::{
  fs,
  path::{Path, PathBuf},
  process::{Command, Output},
  sync::mpsc,
  time::Duration,
};

const POLAR: &str = env!("CARGO_BIN_EXE_polar");

struct Ran {
  code: Option<i32>,
  out: String,
  err: String,
}

fn polar(cwd: &Path, args: &[&str]) -> Ran {
  let tmp = tempfile::tempdir().unwrap();
  let out: Output = Command::new(POLAR)
    .args(args)
    .current_dir(cwd)
    .env("TMPDIR", tmp.path())
    .env_remove("POLAR_DEBUG")
    .output()
    .expect("run polar");

  Ran {
    code: out.status.code(),
    out: String::from_utf8_lossy(&out.stdout).into_owned(),
    err: String::from_utf8_lossy(&out.stderr).into_owned(),
  }
}

fn fixtures() -> PathBuf {
  PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/packages"))
}

fn copy_tree(from: &Path, to: &Path) {
  fs::create_dir_all(to).unwrap();

  for entry in fs::read_dir(from).unwrap() {
    let path = entry.unwrap().path();
    let name = path.file_name().unwrap();

    if name == "dist" || name == "target" {
      continue;
    }

    if path.is_dir() {
      copy_tree(&path, &to.join(name));
    } else {
      fs::copy(&path, to.join(name)).unwrap();
    }
  }
}

fn copied() -> tempfile::TempDir {
  let dir = tempfile::tempdir().unwrap();

  copy_tree(&fixtures(), dir.path());
  dir
}

fn write(dir: &Path, files: &[(&str, &str)]) {
  for (path, text) in files {
    let path = dir.join(path);

    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
  }
}

mod manifest {
  use super::*;

  fn check(files: &[(&str, &str)]) -> Ran {
    let dir = tempfile::tempdir().unwrap();

    write(dir.path(), files);
    polar(&dir.path().join("app"), &["check"])
  }

  const MAIN: (&str, &str) = ("app/src/main.px", "module Main\n");

  #[test]
  fn a_project_without_dependencies_loads_as_before() {
    let ran = check(&[("app/polar.toml", "[project]\nname = \"app\"\n"), MAIN]);

    assert_eq!(ran.code, Some(0), "{}", ran.err);
  }

  #[test]
  fn a_dependency_is_a_path() {
    let ran = check(&[
      (
        "app/polar.toml",
        "[project]\nname = \"app\"\n\n[dependencies]\nweb = \"1.0\"\n",
      ),
      MAIN,
    ]);

    assert_eq!(ran.code, Some(1));
    assert!(ran.err.contains("must be `{ path = \"…\" }`"), "{}", ran.err);
  }

  #[test]
  fn a_path_dependency_must_be_a_package() {
    let ran = check(&[
      (
        "app/polar.toml",
        "[project]\nname = \"app\"\n\n[dependencies]\nother = { path = \"../other\" }\n",
      ),
      ("other/polar.toml", "[project]\nname = \"other\"\n"),
      MAIN,
    ]);

    assert_eq!(ran.code, Some(1));
    assert!(ran.err.contains("is a project, not a package"), "{}", ran.err);
  }

  #[test]
  fn project_and_package_together_are_an_error() {
    let ran = check(&[
      (
        "app/polar.toml",
        "[project]\nname = \"app\"\n\n[package]\nname = \"app\"\nmodule = \"App\"\n",
      ),
      MAIN,
    ]);

    assert_eq!(ran.code, Some(1));
    assert!(ran.err.contains("not both"), "{}", ran.err);
  }

  #[test]
  fn unknown_keys_are_still_rejected() {
    let ran = check(&[
      ("app/polar.toml", "[project]\nname = \"app\"\nplugins = []\n"),
      MAIN,
    ]);

    assert_eq!(ran.code, Some(1));
    assert!(ran.err.contains("plugins"), "{}", ran.err);
  }

  #[test]
  fn a_dependency_is_named_after_its_package() {
    let ran = check(&[
      (
        "app/polar.toml",
        "[project]\nname = \"app\"\n\n[dependencies]\nshape = { path = \"../shapes\" }\n",
      ),
      (
        "shapes/polar.toml",
        "[package]\nname = \"shapes\"\nmodule = \"Shapes\"\n",
      ),
      MAIN,
    ]);

    assert_eq!(ran.code, Some(1));
    assert!(ran.err.contains("name it `shapes`"), "{}", ran.err);
  }

  #[test]
  fn two_packages_claiming_one_module_collide() {
    let ran = check(&[
      (
        "app/polar.toml",
        "[project]\nname = \"app\"\n\n[dependencies]\na = { path = \"../a\" }\nb = { path = \"../b\" }\n",
      ),
      ("a/polar.toml", "[package]\nname = \"a\"\nmodule = \"Same\"\n"),
      ("b/polar.toml", "[package]\nname = \"b\"\nmodule = \"Same\"\n"),
      MAIN,
    ]);

    assert_eq!(ran.code, Some(1));
    assert!(
      ran.err.contains("`a` and `b` both claim the module `Same`"),
      "{}",
      ran.err
    );
  }

  #[test]
  fn std_is_not_a_package_module() {
    let ran = check(&[
      (
        "app/polar.toml",
        "[project]\nname = \"app\"\n\n[dependencies]\nfake = { path = \"../fake\" }\n",
      ),
      ("fake/polar.toml", "[package]\nname = \"fake\"\nmodule = \"Std\"\n"),
      MAIN,
    ]);

    assert_eq!(ran.code, Some(1));
    assert!(ran.err.contains("claims the module `Std`"), "{}", ran.err);
  }

  #[test]
  fn package_cycles_are_an_error() {
    let ran = check(&[
      (
        "app/polar.toml",
        "[project]\nname = \"app\"\n\n[dependencies]\na = { path = \"../a\" }\n",
      ),
      (
        "a/polar.toml",
        "[package]\nname = \"a\"\nmodule = \"A\"\n\n[dependencies]\nb = { path = \"../b\" }\n",
      ),
      (
        "b/polar.toml",
        "[package]\nname = \"b\"\nmodule = \"B\"\n\n[dependencies]\na = { path = \"../a\" }\n",
      ),
      MAIN,
    ]);

    assert_eq!(ran.code, Some(1));
    assert!(ran.err.contains("package cycle"), "{}", ran.err);
  }

  #[test]
  fn a_package_is_not_built_on_its_own() {
    let ran = polar(&fixtures().join("pkgs/shapes"), &["build"]);

    assert_eq!(ran.code, Some(1));
    assert!(ran.err.contains("is a package"), "{}", ran.err);
  }
}

mod resolver {
  use super::*;

  #[test]
  fn basic_checks() {
    let ran = polar(&fixtures().join("basic"), &["check"]);

    assert_eq!(ran.code, Some(0), "{}", ran.err);
  }

  #[test]
  fn transitive_checks() {
    let ran = polar(&fixtures().join("transitive"), &["check"]);

    assert_eq!(ran.code, Some(0), "{}", ran.err);
  }

  #[test]
  fn a_transitive_package_is_not_visible() {
    let dir = copied();

    fs::write(
      dir.path().join("transitive/src/main.px"),
      "module Main\n\nuses\n  Shapes.Circle\n\nfunctions\n  main() {\n    Circle.area(1.0)\n  }\n",
    )
    .unwrap();

    let ran = polar(&dir.path().join("transitive"), &["check"]);

    assert_eq!(ran.code, Some(1));
    assert!(
      ran.err.contains("there is no module `Shapes.Circle`"),
      "{}",
      ran.err
    );
  }

  #[test]
  fn a_source_file_shadowing_a_package_collides() {
    let ran = polar(&fixtures().join("collision"), &["check"]);

    assert_eq!(ran.code, Some(1));
    assert!(
      ran.err.contains("`src/shapes.px` and the package `shapes` both claim the module `Shapes`"),
      "{}",
      ran.err
    );
  }

  #[test]
  fn a_header_must_match_its_path() {
    let ran = polar(&fixtures().join("header_mismatch"), &["check"]);

    assert_eq!(ran.code, Some(1));
    assert!(
      ran.err.contains("declares `module Round`, not `module Circle`"),
      "{}",
      ran.err
    );
  }

  #[test]
  fn a_package_checks_from_disk() {
    let root = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/.."));
    let ran = polar(root, &["check", "projects/simple_framework"]);

    assert_eq!(ran.code, Some(0), "{}", ran.err);
  }
}

mod output {
  use super::*;

  #[test]
  fn dependencies_build_into_deps() {
    let dir = copied();
    let basic = dir.path().join("basic");
    let ran = polar(&basic, &["build"]);

    assert_eq!(ran.code, Some(0), "{}", ran.err);
    assert!(basic.join("dist/_deps/shapes/circle.js").is_file());
    assert!(
      basic.join("dist/_deps/shapes/pi.js").is_file(),
      "the extern's file is copied"
    );

    let main = fs::read_to_string(basic.join("dist/main.js")).unwrap();

    assert!(main.contains("\"./_deps/shapes/circle.js\""), "{main}");

    let circle =
      fs::read_to_string(basic.join("dist/_deps/shapes/circle.js")).unwrap();

    assert!(circle.contains("\"../../_polar/runtime.js\""), "{circle}");

    let node =
      std::env::var("POLAR_NODE").unwrap_or_else(|_| "node".to_string());
    let ran = Command::new(node)
      .args([
        "--input-type=module",
        "-e",
        "(await import('./dist/main.js')).main()",
      ])
      .current_dir(&basic)
      .output()
      .unwrap();

    assert_eq!(String::from_utf8_lossy(&ran.stdout), "12\n");
  }

  #[test]
  fn a_source_file_may_not_land_in_deps() {
    let dir = tempfile::tempdir().unwrap();

    write(dir.path(), &[("src/_deps/x.px", "module X\n")]);

    let ran = polar(dir.path(), &["build", "src"]);

    assert_eq!(ran.code, Some(1));
    assert!(ran.err.contains("where dependencies go"), "{}", ran.err);
  }

  #[test]
  fn run_prints_through_a_dependency() {
    let ran = polar(&fixtures().join("basic"), &["run"]);

    assert_eq!((ran.code, ran.out.as_str()), (Some(0), "12\n"), "{}", ran.err);

    let ran = polar(&fixtures().join("transitive"), &["run"]);

    assert_eq!(ran.out, "a circle of area 3\n", "{}", ran.err);
  }

  #[test]
  fn fmt_formats_only_the_project() {
    let dir = copied();

    fs::write(
      dir.path().join("pkgs/shapes/src/circle.px"),
      "module Circle\nfunctions\n  area(r: Float) -> Float { r }\nexports\n  area\n",
    )
    .unwrap();

    let ran = polar(&dir.path().join("basic"), &["fmt", "--check"]);

    assert_eq!(ran.code, Some(0), "{}{}", ran.out, ran.err);
  }
}

mod watch {
  use super::*;

  #[test]
  fn editing_a_dependency_rechecks() {
    let dir = copied();
    let basic = dir.path().join("basic");
    let (tx, rx) = mpsc::channel::<CycleResult>();
    let watch = Watch::start(WatchOptions {
      paths: vec![PathBuf::from("src")],
      cwd: basic.clone(),
      mode: WatchMode::Check { out: None },
      debounce: Duration::from_millis(50),
      compiler: Compiler::default(),
      color: false,
      clear_screen: false,
      debug: false,
      err: Box::new(std::io::sink()),
      on_cycle: Box::new(move |r| {
        let _ = tx.send(r.clone());
      }),
    })
    .expect("start watching");

    let first =
      rx.recv_timeout(Duration::from_secs(10)).expect("a first cycle");

    assert_eq!(first.errors, 0);

    fs::write(
      dir.path().join("pkgs/shapes/src/circle.px"),
      "module Circle\n\nfunctions\n  size(r: Float) -> Float {\n    r\n  }\n\nexports\n  size\n",
    )
    .unwrap();

    let cycle =
      rx.recv_timeout(Duration::from_secs(10)).expect("a cycle after the edit");

    assert!(cycle.errors > 0, "{cycle:?}");
    watch.stop();
  }
}

mod plugins {
  use super::*;

  #[test]
  fn a_declared_plugin_is_scaffolded_built_and_checked() {
    let dir = tempfile::tempdir().unwrap();

    write(
      dir.path(),
      &[
        (
          "fw/polar.toml",
          "[package]\nname = \"fw\"\nmodule = \"Fw\"\n\n[plugin]\nzones = [\"things\"]\n",
        ),
        (
          "app/polar.toml",
          "[project]\nname = \"app\"\n\n[dependencies]\nfw = { path = \"../fw\" }\n",
        ),
        ("app/src/main.px", "module Main\n\nthings\n  one\n"),
      ],
    );

    let app = dir.path().join("app");
    let ran = polar(&app, &["check"]);

    assert_eq!(ran.code, Some(1), "{}", ran.err);
    assert!(
      ran.err.contains("note: created ../fw/plugin/lib.rs"),
      "{}",
      ran.err
    );
    assert!(
      ran.err.contains("the `things` zone isn't implemented yet"),
      "{}",
      ran.err
    );
    assert!(dir.path().join("fw/plugin/lib.rs").is_file());
    assert!(dir.path().join("fw/.polar/plugin/Cargo.toml").is_file());
    assert!(dir.path().join("fw/.polar/api/src/lib.rs").is_file());

    let ran = polar(&app, &["check"]);

    assert!(!ran.err.contains("note: created"), "{}", ran.err);

    fs::write(
      dir.path().join("fw/polar.toml"),
      "[package]\nname = \"fw\"\nmodule = \"Fw\"\n\n[plugin]\nzones = [\"things\", \"more\"]\n",
    )
    .unwrap();

    let ran = polar(&app, &["check"]);

    assert!(
      ran
        .err
        .contains("lists the zone `more`, but the plugin doesn't define it"),
      "{}",
      ran.err
    );

    fs::write(dir.path().join("fw/plugin/lib.rs"), "this is not rust\n")
      .unwrap();

    let ran = polar(&app, &["check"]);

    assert!(ran.err.contains("doesn't build"), "{}", ran.err);
  }
}

mod launcher {
  use super::*;

  const KIT: &str = "[package]\nname = \"kit\"\nmodule = \"Kit\"\n\n[launcher]\nscript = \"launch.mjs\"\n";

  const LAUNCH: &str = r#"import { existsSync } from "node:fs";
import { readFile } from "node:fs/promises";
import { join } from "node:path";

const manifest = JSON.parse(await readFile(process.argv[2], "utf8"));
const hosts = Object.entries(manifest.hosts).sort();

console.log(manifest.version, manifest.project, manifest.main, JSON.stringify(manifest.options));

for (const [host, dir] of hosts) {
  console.log(host, existsSync(join(dir, manifest.main)), existsSync(join(dir, "_polar/runtime.js")));
}

process.exitCode = manifest.options.code ?? 0;
"#;

  const MAIN: &str = "module Main\n\nhosts\n  Browser\n  Node\n\nfunctions\n  main() {\n    Log.info(\"main ran\")\n  }\n\nexports\n  Browser\n  Node\n  main\n";

  fn app(run: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let manifest = format!(
      "[project]\nname = \"app\"\nhosts = [\"Browser\", \"Node\"]\n{run}\n[dependencies]\nkit = {{ path = \"../kit\" }}\n"
    );

    write(
      dir.path(),
      &[
        ("kit/polar.toml", KIT),
        ("kit/launch.mjs", LAUNCH),
        ("kit/src/.keep", ""),
        ("app/polar.toml", &manifest),
        ("app/src/main.px", MAIN),
      ],
    );
    dir
  }

  #[test]
  fn run_hands_every_host_build_to_the_launcher() {
    let dir =
      app("\n[run]\nlauncher = \"kit\"\noptions = { port = 1, code = 3 }\n");
    let ran = polar(&dir.path().join("app"), &["run"]);

    assert_eq!(ran.code, Some(3), "{}", ran.err);
    assert_eq!(
      ran.out,
      "1 app main.js {\"code\":3,\"port\":1}\nBrowser true true\nNode true true\n",
      "{}",
      ran.err
    );
    assert!(!ran.err.contains("built"), "{}", ran.err);
  }

  #[test]
  fn run_with_a_host_runs_main_instead() {
    let dir = app("\n[run]\nlauncher = \"kit\"\n");
    let ran = polar(&dir.path().join("app"), &["run", "--host", "Node"]);

    assert_eq!(
      (ran.code, ran.out.as_str()),
      (Some(0), "main ran\n"),
      "{}",
      ran.err
    );
  }

  #[test]
  fn a_compile_error_stops_before_the_launcher() {
    let dir = app("\n[run]\nlauncher = \"kit\"\n");

    fs::write(
      dir.path().join("app/src/main.px"),
      MAIN.replace("Log.info(\"main ran\")", "missing()"),
    )
    .unwrap();

    let ran = polar(&dir.path().join("app"), &["run"]);

    assert_eq!(ran.code, Some(1));
    assert!(ran.out.is_empty(), "{}", ran.out);
    assert!(ran.err.contains("missing"), "{}", ran.err);
  }

  #[test]
  fn the_launcher_must_be_a_dependency() {
    let dir = app("\n[run]\nlauncher = \"web\"\n");
    let ran = polar(&dir.path().join("app"), &["run"]);

    assert_eq!(ran.code, Some(1));
    assert!(ran.err.contains("`web`, which isn't a dependency"), "{}", ran.err);
  }

  #[test]
  fn the_dependency_must_provide_a_launcher() {
    let dir = app("\n[run]\nlauncher = \"kit\"\n");

    fs::write(
      dir.path().join("kit/polar.toml"),
      "[package]\nname = \"kit\"\nmodule = \"Kit\"\n",
    )
    .unwrap();

    let ran = polar(&dir.path().join("app"), &["run"]);

    assert_eq!(ran.code, Some(1));
    assert!(
      ran.err.contains("that package has no `[launcher]`"),
      "{}",
      ran.err
    );
  }

  #[test]
  fn the_launcher_script_must_exist() {
    let dir = app("\n[run]\nlauncher = \"kit\"\n");

    fs::remove_file(dir.path().join("kit/launch.mjs")).unwrap();

    let ran = polar(&dir.path().join("app"), &["check"]);

    assert_eq!(ran.code, Some(1));
    assert!(ran.err.contains("the launcher script"), "{}", ran.err);
    assert!(ran.err.contains("doesn't exist"), "{}", ran.err);
  }

  #[test]
  fn only_a_package_provides_a_launcher() {
    let dir = app("\n[launcher]\nscript = \"x.mjs\"\n");
    let ran = polar(&dir.path().join("app"), &["check"]);

    assert_eq!(ran.code, Some(1));
    assert!(
      ran.err.contains("only a `[package]` provides a `[launcher]`"),
      "{}",
      ran.err
    );
  }

  #[test]
  fn build_bundles_the_launcher_for_start() {
    let dir =
      app("\n[run]\nlauncher = \"kit\"\noptions = { port = 1, code = 3 }\n");
    let app = dir.path().join("app");
    let built = polar(&app, &["build"]);

    assert_eq!(built.code, Some(0), "{}", built.err);
    assert!(app.join("dist/start.mjs").is_file());
    assert!(app.join("dist/_polar/launcher/launch.mjs").is_file());

    let ran = polar(&app, &["start"]);

    assert_eq!(ran.code, Some(3), "{}", ran.err);
    assert_eq!(
      ran.out,
      "1 app main.js {\"code\":3,\"port\":1}\nBrowser true true\nNode true true\n",
      "{}",
      ran.err
    );
  }

  #[test]
  fn a_moved_build_still_starts() {
    let dir = app("\n[run]\nlauncher = \"kit\"\n");
    let app = dir.path().join("app");

    assert_eq!(polar(&app, &["build"]).code, Some(0));

    let moved = dir.path().join("moved");

    copy_tree(&app.join("dist"), &moved);
    fs::remove_dir_all(app.join("dist")).unwrap();

    let out = Command::new(polar_compiler_node())
      .arg(moved.join("start.mjs"))
      .output()
      .expect("run node");

    assert_eq!(
      String::from_utf8_lossy(&out.stdout),
      "1 app main.js {}\nBrowser true true\nNode true true\n",
      "{}",
      String::from_utf8_lossy(&out.stderr)
    );
  }

  fn polar_compiler_node() -> PathBuf {
    std::env::var_os("POLAR_NODE")
      .map_or_else(|| PathBuf::from("node"), PathBuf::from)
  }
}

mod start {
  use super::*;

  const MAIN: &str = "module Main\n\nfunctions\n  main() {\n    Log.info(\"started\")\n  }\n\nexports\n  main\n";

  fn project(hosts: &str, main: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();

    write(
      dir.path(),
      &[
        ("polar.toml", &format!("[project]\nname = \"app\"\n{hosts}")),
        ("src/main.px", main),
      ],
    );
    dir
  }

  #[test]
  fn start_runs_the_built_main() {
    let dir = project("", MAIN);

    assert_eq!(polar(dir.path(), &["build"]).code, Some(0));

    let ran = polar(dir.path(), &["start"]);

    assert_eq!(
      (ran.code, ran.out.as_str()),
      (Some(0), "started\n"),
      "{}",
      ran.err
    );
    assert!(dir.path().join("dist/_polar/launcher.mjs").is_file());
  }

  #[test]
  fn start_needs_a_build() {
    let dir = project("", MAIN);
    let ran = polar(dir.path(), &["start"]);

    assert_eq!(ran.code, Some(1));
    assert!(ran.err.contains("run `polar build` first"), "{}", ran.err);
  }

  #[test]
  fn start_reports_an_uncaught_error() {
    let main = "module Main\n\nuses\n  Std.Assert\n\nfunctions\n  main() {\n    Assert.assert(1 == 2)\n  }\n\nexports\n  main\n";
    let dir = project("", main);

    assert_eq!(polar(dir.path(), &["build"]).code, Some(0));

    let ran = polar(dir.path(), &["start"]);

    assert_eq!(ran.code, Some(1), "{}", ran.out);
    assert!(ran.err.contains("uncaught error"), "{}", ran.err);
  }

  #[test]
  fn several_hosts_need_a_choice() {
    let main = "module Main\n\nhosts\n  Browser\n  Node\n\nfunctions\n  main() {\n    Log.info(\"started\")\n  }\n\nexports\n  Browser\n  Node\n  main\n";
    let dir = project("hosts = [\"Browser\", \"Node\"]\n", main);

    assert_eq!(polar(dir.path(), &["build"]).code, Some(0));

    let ran = polar(dir.path(), &["start"]);

    assert_eq!(ran.code, Some(1));
    assert!(ran.err.contains("choose one with `--host`"), "{}", ran.err);

    let ran = polar(dir.path(), &["start", "--host", "Node"]);

    assert_eq!(
      (ran.code, ran.out.as_str()),
      (Some(0), "started\n"),
      "{}",
      ran.err
    );
  }

  #[test]
  fn arguments_after_a_double_dash_reach_the_launcher() {
    let dir = project("", MAIN);

    assert_eq!(polar(dir.path(), &["build"]).code, Some(0));

    let ran = polar(dir.path(), &["start", "--", "extra"]);

    assert_eq!(
      (ran.code, ran.out.as_str()),
      (Some(0), "started\n"),
      "{}",
      ran.err
    );
  }
}

mod siblings {
  use super::*;

  /// A `names` zone: `greeting = "hi"` entries become functions of a sibling
  /// module, `Names` unless an entry `module Other` renames it.
  const PLUGIN: &str = r#"use polar_plugin::{Entry, Expansion, Module, Span, Zone, ZonePlugin, export};

struct Names;

impl ZonePlugin for Names {
  fn zone(&self) -> Zone {
    Zone { keyword: "names".to_string(), after: "types".to_string(), blank_between_entries: false }
  }

  fn expand(&self, zone: Span, entries: &[Entry], _module: &Module) -> Expansion {
    let mut out = Expansion::default();
    let mut name = "Names".to_string();
    let mut functions = String::new();
    let mut exports = String::new();

    for entry in entries {
      let t = &entry.tokens;

      if t[0].text == "module" {
        name = entry.text(Span { start: t[1].span.start, end: entry.span.end }).to_string();
        continue;
      }

      functions.push_str(&format!("  {}() -> String {{\n    {}\n  }}\n\n", t[0].text, entry.text(Span { start: t[2].span.start, end: entry.span.end })));
      exports.push_str(&format!("  {}\n", t[0].text));
    }

    out.emit_sibling(
      name.clone(),
      format!("module {name}\n\nfunctions\n{functions}exports\n{exports}"),
      zone,
    );
    out
  }
}

export!(Names);
"#;

  fn app(table: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();

    write(
      dir.path(),
      &[
        (
          "fw/polar.toml",
          "[package]\nname = \"fw\"\nmodule = \"Fw\"\n\n[plugin]\nzones = [\"names\"]\n",
        ),
        ("fw/plugin/lib.rs", PLUGIN),
        ("fw/src/.keep", ""),
        (
          "app/polar.toml",
          "[project]\nname = \"app\"\n\n[dependencies]\nfw = { path = \"../fw\" }\n",
        ),
        (
          "app/src/main.px",
          "module Main\n\nuses\n  Table\n\nfunctions\n  main() {\n    Log.info(Table.show())\n  }\n\nexports\n  main\n",
        ),
        ("app/src/table.px", table),
        (
          "app/src/greeter.px",
          "module Greeter\n\nuses\n  Names\n\nfunctions\n  greet() -> String {\n    \"#{Names.greeting()} there\"\n  }\n\nexports\n  greet\n",
        ),
      ],
    );
    dir
  }

  const TABLE: &str = "module Table\n\nuses\n  Greeter\n\nnames\n  greeting = \"hi\"\n\nfunctions\n  show() -> String {\n    Greeter.greet()\n  }\n\nexports\n  show\n";

  #[test]
  fn a_zone_generates_a_module_that_breaks_an_import_cycle() {
    // `Table` imports `Greeter`, which imports `Names`, generated from
    // `Table`'s own zone: no cycle, because `Names` doesn't import `Table`.
    let dir = app(TABLE);
    let app = dir.path().join("app");
    let ran = polar(&app, &["run"]);

    assert_eq!(
      (ran.code, ran.out.as_str()),
      (Some(0), "hi there\n"),
      "{}",
      ran.err
    );

    let ran = polar(&app, &["build"]);

    assert_eq!(ran.code, Some(0), "{}", ran.err);
    assert!(app.join("dist/names.js").is_file());
    assert!(!app.join("src/names.px").exists());
  }

  #[test]
  fn an_unimported_sibling_is_still_built_and_checked() {
    let dir = app(&TABLE.replace("greeting", "other"));
    let app = dir.path().join("app");

    fs::write(
      app.join("src/greeter.px"),
      "module Greeter\n\nfunctions\n  greet() -> String {\n    \"there\"\n  }\n\nexports\n  greet\n",
    )
    .unwrap();

    let ran = polar(&app, &["build"]);

    assert_eq!(ran.code, Some(0), "{}", ran.err);
    assert!(app.join("dist/names.js").is_file());

    fs::write(app.join("src/table.px"), TABLE.replace("\"hi\"", "1 + \"x\""))
      .unwrap();

    let ran = polar(&app, &["check"]);

    assert_eq!(ran.code, Some(1), "{}", ran.err);
    assert!(ran.err.contains("src/names.px"), "{}", ran.err);
    assert!(ran.err.contains("Generated by the `names` zone"), "{}", ran.err);
  }

  #[test]
  fn a_file_with_the_siblings_name_is_an_error() {
    let dir = app(TABLE);
    let app = dir.path().join("app");

    fs::write(app.join("src/names.px"), "module Names\n").unwrap();

    let ran = polar(&app, &["check"]);

    assert_eq!(ran.code, Some(1), "{}", ran.err);
    assert!(
      ran.err.contains(
        "generates the module `Names` next to it, but `src/names.px` already exists"
      ),
      "{}",
      ran.err
    );
  }

  #[test]
  fn a_sibling_needs_a_pascal_case_name_other_than_its_own() {
    for (renamed, problem) in [
      ("module bad_name", "one PascalCase word"),
      ("module Table", "the name of the module holding the zone"),
    ] {
      let dir =
        app(&TABLE.replace("names\n", &format!("names\n  {renamed}\n")));
      let ran = polar(&dir.path().join("app"), &["check"]);

      assert_eq!(ran.code, Some(1), "{}", ran.err);
      assert!(ran.err.contains(problem), "{renamed}: {}", ran.err);
    }
  }
}
