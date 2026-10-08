use polar_cli::{
  Compiler, Io,
  files::{self, Input},
  run_with,
  watch::{CycleResult, Watch, WatchMode, WatchOptions},
};
use polar_compiler::{
  CompileOptions, CompileOutput,
  shared::codes::DiagnosticCode,
  shared::diagnostic::{Diagnostic, Label},
};
use std::{
  collections::HashMap,
  ffi::OsString,
  fs,
  path::{Path, PathBuf},
  process::Command,
  sync::mpsc,
  time::Duration,
};
use tempfile::TempDir;

struct Ran {
  code: u8,
  out: String,
  err: String,
}

fn polar_in(
  cwd: &Path,
  args: &[&str],
  env: &[(&str, &str)],
  compiler: Compiler,
) -> Ran {
  let (mut out, mut err) = (Vec::new(), Vec::new());
  let env: HashMap<String, String> =
    env.iter().map(|(k, v)| ((*k).to_string(), (*v).to_string())).collect();
  let lookup = |name: &str| env.get(name).cloned();
  let args: Vec<OsString> = args.iter().map(OsString::from).collect();
  let code = {
    let mut io = Io {
      out: &mut out,
      err: &mut err,
      stdout_is_tty: false,
      stderr_is_tty: false,
      cwd: cwd.to_path_buf(),
      env: &lookup,
    };

    run_with(&args, &mut io, compiler)
  };

  Ran {
    code,
    out: String::from_utf8(out).expect("stdout is UTF-8"),
    err: String::from_utf8(err).expect("stderr is UTF-8"),
  }
}

fn polar(cwd: &Path, args: &[&str]) -> Ran {
  polar_in(cwd, args, &[], Compiler::default())
}

const HELLO: &str = "functions\n  main() {\n    Log.info(\"hello, world\")\n  }\n\nexports\n  main\n";
const BROKEN: &str = "functions\n  main() {\n    let = 1\n  }\n";

fn project(files: &[(&str, &[u8])]) -> TempDir {
  let dir = tempfile::tempdir().expect("create a temp dir");

  for (path, bytes) in files {
    let path = dir.path().join(path);

    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
  }

  dir
}

fn tree(dir: &Path) -> Vec<String> {
  let mut out: Vec<String> = walkdir::WalkDir::new(dir)
    .into_iter()
    .filter_map(Result::ok)
    .filter(|e| e.file_type().is_file())
    .map(|e| e.path().strip_prefix(dir).unwrap().display().to_string())
    .collect();

  out.sort();
  out
}

mod skeleton {
  use super::*;

  #[test]
  fn help_lists_commands() {
    let ran = polar(Path::new("."), &["--help"]);

    assert_eq!(ran.code, 0);

    for command in ["build", "run", "check", "fmt"] {
      assert!(ran.out.contains(command), "{}", ran.out);
    }
  }

  #[test]
  fn per_command_help() {
    let ran = polar(Path::new("."), &["build", "--help"]);

    assert_eq!(ran.code, 0);

    for flag in ["--out", "--watch", "--emit"] {
      assert!(ran.out.contains(flag), "{}", ran.out);
    }
  }

  #[test]
  fn version() {
    let ran = polar(Path::new("."), &["--version"]);

    assert_eq!(
      (ran.code, ran.out.trim()),
      (0, concat!("polar ", env!("CARGO_PKG_VERSION")))
    );
  }

  #[test]
  fn unknown_command() {
    let ran = polar(Path::new("."), &["bulid", "projects"]);

    assert_eq!(ran.code, 1);
    assert!(ran.err.contains("build"), "{}", ran.err);
    assert!(ran.err.contains("polar --help"), "{}", ran.err);
  }

  #[test]
  fn unknown_option() {
    let ran = polar(Path::new("."), &["build", "x.px", "--nope"]);

    assert_eq!(ran.code, 1);
    assert!(ran.err.contains("--nope"), "{}", ran.err);
    assert!(ran.out.is_empty());
  }

  #[test]
  fn paths_are_required() {
    let ran = polar(Path::new("."), &["fmt"]);

    assert_eq!(ran.code, 1);
    assert!(ran.err.contains("required"), "{}", ran.err);
  }

  #[test]
  fn emit_requires_one_file() {
    let dir =
      project(&[("a.px", HELLO.as_bytes()), ("b.px", HELLO.as_bytes())]);
    let ran = polar(dir.path(), &["build", "a.px", "b.px", "--emit=core"]);

    assert_eq!(ran.code, 1);
    assert!(ran.err.contains("exactly one input file"), "{}", ran.err);
  }

  #[test]
  fn emit_writes_nothing() {
    let dir = project(&[("a.px", HELLO.as_bytes())]);
    let ran =
      polar(dir.path(), &["build", "a.px", "--emit=core", "--out", "dist"]);

    assert_eq!(ran.code, 0, "{}", ran.err);
    assert!(ran.out.contains("main"), "{}", ran.out);
    assert!(!dir.path().join("dist").exists());
  }

  fn ice_compile(_: &str, _: &str, _: &CompileOptions) -> CompileOutput {
    polar_compiler::shared::ice::ice("boom", None)
  }

  #[allow(
    clippy::unnecessary_literal_unwrap,
    reason = "the panic is the point"
  )]
  fn unwrap_compile(_: &str, _: &str, _: &CompileOptions) -> CompileOutput {
    let nothing: Option<CompileOutput> = None;

    nothing.unwrap()
  }

  fn stubbed(
    compile: fn(&str, &str, &CompileOptions) -> CompileOutput,
  ) -> Compiler {
    Compiler { compile, ..Compiler::default() }
  }

  #[test]
  fn ice_exits_2() {
    let dir = project(&[("a.px", HELLO.as_bytes())]);
    let ran =
      polar_in(dir.path(), &["build", "a.px"], &[], stubbed(ice_compile));

    assert_eq!(ran.code, 2);
    assert!(
      ran.err.contains("error[POLAR0001]: internal compiler error: boom"),
      "{}",
      ran.err
    );
    assert!(
      ran.err.contains("this is a bug in the Polar compiler"),
      "{}",
      ran.err
    );
    assert!(ran.err.contains("a.px"), "{}", ran.err);
  }

  #[test]
  fn plain_panic_also_exits_2() {
    let dir = project(&[("a.px", HELLO.as_bytes())]);
    let ran =
      polar_in(dir.path(), &["check", "a.px"], &[], stubbed(unwrap_compile));

    assert_eq!(ran.code, 2);
    assert!(ran.err.contains("error[POLAR0001]"), "{}", ran.err);
    assert!(ran.err.contains("`None`"), "{}", ran.err);
  }

  #[test]
  fn backtrace_only_under_debug() {
    let dir = project(&[("a.px", HELLO.as_bytes())]);
    let args = ["build", "a.px"];
    let quiet = polar_in(dir.path(), &args, &[], stubbed(ice_compile));
    let debug = polar_in(
      dir.path(),
      &args,
      &[("POLAR_DEBUG", "1")],
      stubbed(ice_compile),
    );

    assert!(!quiet.err.contains("backtrace"), "{}", quiet.err);
    assert!(debug.err.contains("backtrace"), "{}", debug.err);

    for ran in [quiet, debug] {
      assert!(!ran.err.contains("panicked at"), "{}", ran.err);
    }
  }

  #[test]
  fn io_error_exits_1() {
    let dir = project(&[]);
    let ran = polar(dir.path(), &["build", "missing.px"]);

    assert_eq!(ran.code, 1);
    assert!(
      ran.err.starts_with("error: cannot read missing.px: "),
      "{}",
      ran.err
    );
    assert!(!ran.err.contains("POLAR0001"));
  }
}

mod discovery {
  use super::*;

  fn names(inputs: &[Input]) -> Vec<&str> {
    inputs.iter().map(|i| i.display.as_str()).collect()
  }

  #[test]
  fn file_argument_must_be_polar() {
    let dir = project(&[("notes.txt", b"hi")]);
    let ran = polar(dir.path(), &["build", "notes.txt"]);

    assert_eq!(ran.code, 1);
    assert!(ran.err.contains(".px"), "{}", ran.err);
  }

  #[test]
  fn missing_path() {
    let ran = polar(project(&[]).path(), &["build", "nope.px"]);

    assert_eq!(ran.code, 1);
  }

  #[test]
  fn directory_is_walked() {
    let dir =
      project(&[("src/a.px", b""), ("src/b.px", b""), ("src/c.txt", b"")]);
    let inputs = files::discover(&["src".into()], dir.path(), None).unwrap();

    assert_eq!(names(&inputs), ["src/a.px", "src/b.px"]);
  }

  #[test]
  fn pruned_directories() {
    let dir = project(&[
      ("src/.git/a.px", b""),
      ("src/node_modules/b.px", b""),
      ("src/target/c.px", b""),
      ("src/snapshots/d.px", b""),
    ]);
    let inputs = files::discover(&["src".into()], dir.path(), None).unwrap();

    assert!(inputs.is_empty(), "{inputs:?}");
  }

  #[test]
  fn out_dir_is_pruned() {
    let dir = project(&[("src/a.px", b""), ("src/dist/old.px", b"")]);
    let inputs =
      files::discover(&["src".into()], dir.path(), Some(Path::new("src/dist")))
        .unwrap();

    assert_eq!(names(&inputs), ["src/a.px"]);
  }

  #[test]
  fn discovery_is_sorted() {
    let dir = project(&[("s/b.px", b""), ("s/a.px", b""), ("s/c.px", b"")]);
    let inputs = files::discover(&["s".into()], dir.path(), None).unwrap();

    assert_eq!(names(&inputs), ["s/a.px", "s/b.px", "s/c.px"]);
  }

  #[test]
  fn bom_is_stripped() {
    let source = files::decode("a.px", b"\xEF\xBB\xBFfunctions\n");

    assert_eq!(source.text, "functions\n");
    assert!(source.invalid.is_none());
  }

  #[test]
  fn bom_does_not_shift_offsets() {
    let dir = project(&[("a.px", b"\xEF\xBB\xBFoops\n")]);
    let ran = polar(dir.path(), &["check", "a.px"]);

    assert_eq!(ran.code, 1);
    assert!(ran.err.contains("--> a.px:1:1"), "{}", ran.err);
  }

  const INVALID: &[u8] = b"functions\n  f() {\n    \"\xff\"\n  }\n";

  #[test]
  fn invalid_utf8_is_reported() {
    let source = files::decode("a.px", INVALID);
    let d = source.invalid.expect("a diagnostic");

    assert_eq!(d.code, DiagnosticCode::InvalidUtf8);
    assert_eq!((d.primary.span.start, d.primary.span.end), (23, 23));
  }

  #[test]
  fn invalid_utf8_position_is_right() {
    let dir = project(&[("a.px", INVALID)]);
    let ran = polar(dir.path(), &["check", "a.px"]);

    assert!(ran.err.contains("error[POLAR0002]"), "{}", ran.err);
    assert!(ran.err.contains("--> a.px:3:6"), "{}", ran.err);
  }

  #[test]
  fn invalid_utf8_is_not_compiled() {
    let dir = project(&[("a.px", INVALID), ("b.px", HELLO.as_bytes())]);
    let ran = polar(dir.path(), &["build", "a.px", "b.px", "--out", "dist"]);

    assert_eq!(ran.code, 1);
    assert!(!dir.path().join("dist").exists());
  }

  #[test]
  fn output_mapping_for_a_file() {
    let dir = project(&[("src/a.px", HELLO.as_bytes())]);
    let ran = polar(dir.path(), &["build", "src/a.px", "--out", "dist"]);

    assert_eq!(ran.code, 0, "{}", ran.err);
    assert_eq!(
      tree(&dir.path().join("dist")),
      ["_polar/runtime.js", "a.d.ts", "a.js", "a.js.map"]
    );
  }

  #[test]
  fn output_mapping_for_a_directory() {
    let dir = project(&[("src/posts/show.px", HELLO.as_bytes())]);
    let ran = polar(dir.path(), &["build", "src", "--out", "dist"]);

    assert_eq!(ran.code, 0, "{}", ran.err);
    assert_eq!(
      tree(&dir.path().join("dist")),
      [
        "_polar/runtime.js",
        "posts/show.d.ts",
        "posts/show.js",
        "posts/show.js.map"
      ]
    );
  }

  #[test]
  fn output_collision() {
    let dir = project(&[
      ("src/a.px", HELLO.as_bytes()),
      ("lib/a.px", HELLO.as_bytes()),
    ]);
    let ran = polar(dir.path(), &["build", "src/a.px", "lib/a.px"]);

    assert_eq!(ran.code, 1);
    assert!(
      ran.err.contains("src/a.px") && ran.err.contains("lib/a.px"),
      "{}",
      ran.err
    );
  }

  #[test]
  fn writes_are_atomic() {
    let dir = project(&[("keep.px", b"original")]);
    let target = dir.path().join("keep.px");
    let failed = files::write_atomic_with(&target, b"new and longer", |_| {
      Err(std::io::Error::other("interrupted"))
    });

    assert!(failed.is_err());
    assert_eq!(fs::read(&target).unwrap(), b"original");
    assert_eq!(tree(dir.path()), ["keep.px"], "a partial file remains");

    files::write_atomic(&target, b"replaced").unwrap();
    assert_eq!(fs::read(&target).unwrap(), b"replaced");
  }

  #[test]
  fn code_is_registered() {
    assert!(DiagnosticCode::ALL.contains(&DiagnosticCode::InvalidUtf8));
    assert_eq!(DiagnosticCode::InvalidUtf8.to_string(), "POLAR0002");
  }

  #[test]
  fn relative_paths() {
    let rel =
      |a: &str, b: &str| files::relative_path(Path::new(a), Path::new(b));

    assert_eq!(
      rel("/p/dist/posts", "/p/src/posts/show.px"),
      "../../src/posts/show.px"
    );
    assert_eq!(rel("/p/dist", "/p/dist/a.px"), "a.px");
  }
}

mod build {
  use super::*;

  const WARN: &str = "functions\n  main() {\n    1\n  }\n\nexports\n  main\n";

  fn built(files: &[(&str, &[u8])], args: &[&str]) -> (TempDir, Ran) {
    let dir = project(files);
    let ran = polar(dir.path(), args);

    (dir, ran)
  }

  fn map_json(path: &Path) -> serde_json::Value {
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
  }

  #[test]
  fn builds_a_project() {
    let (dir, ran) = built(
      &[("src/a.px", HELLO.as_bytes()), ("src/b.px", WARN.as_bytes())],
      &["build", "src", "--out", "dist"],
    );

    assert_eq!(ran.code, 0, "{}", ran.err);
    assert_eq!(
      tree(&dir.path().join("dist")),
      [
        "_polar/runtime.js",
        "a.d.ts",
        "a.js",
        "a.js.map",
        "b.d.ts",
        "b.js",
        "b.js.map"
      ]
    );
  }

  #[test]
  fn built_hello_runs() {
    let (dir, ran) =
      built(&[("hello.px", HELLO.as_bytes())], &["build", "hello.px"]);

    assert_eq!(ran.code, 0, "{}", ran.err);

    let out = Command::new(
      std::env::var("POLAR_NODE").unwrap_or_else(|_| "node".into()),
    )
    .arg("--input-type=module")
    .arg("-e")
    .arg("const m = await import(process.argv[1]); await m.main();")
    .arg(dir.path().join("dist/hello.js"))
    .output()
    .expect("run node");

    assert_eq!(String::from_utf8_lossy(&out.stdout), "hello, world\n");
  }

  #[test]
  fn source_map_url_is_appended() {
    let (dir, _) =
      built(&[("src/posts/show.px", HELLO.as_bytes())], &["build", "src"]);
    let js = fs::read_to_string(dir.path().join("dist/posts/show.js")).unwrap();

    assert_eq!(js.lines().last(), Some("//# sourceMappingURL=show.js.map"));
    assert!(js.ends_with('\n'));
  }

  #[test]
  fn map_sources_resolve() {
    let (dir, _) =
      built(&[("src/posts/show.px", HELLO.as_bytes())], &["build", "src"]);
    let map_path = dir.path().join("dist/posts/show.js.map");
    let map = map_json(&map_path);
    let source = map["sources"][0].as_str().unwrap();

    assert_eq!(source, "../../src/posts/show.px");
    assert_eq!(
      map_path.parent().unwrap().join(source).canonicalize().unwrap(),
      dir.path().join("src/posts/show.px").canonicalize().unwrap()
    );
  }

  #[test]
  fn map_file_field() {
    let (dir, _) =
      built(&[("src/posts/show.px", HELLO.as_bytes())], &["build", "src"]);

    assert_eq!(
      map_json(&dir.path().join("dist/posts/show.js.map"))["file"],
      "show.js"
    );
  }

  #[test]
  fn nested_runtime_specifier() {
    let (dir, _) = built(
      &[
        ("src/posts/show.px", HELLO.as_bytes()),
        ("src/top.px", HELLO.as_bytes()),
      ],
      &["build", "src"],
    );
    let nested =
      fs::read_to_string(dir.path().join("dist/posts/show.js")).unwrap();
    let top = fs::read_to_string(dir.path().join("dist/top.js")).unwrap();

    assert!(nested.contains("\"../_polar/runtime.js\""), "{nested}");
    assert!(top.contains("\"./_polar/runtime.js\""), "{top}");
  }

  #[test]
  fn runtime_written_once() {
    let (dir, _) = built(
      &[
        ("s/a.px", HELLO.as_bytes()),
        ("s/b.px", HELLO.as_bytes()),
        ("s/c/d.px", HELLO.as_bytes()),
      ],
      &["build", "s"],
    );
    let runtimes = tree(&dir.path().join("dist"))
      .into_iter()
      .filter(|p| p.ends_with("runtime.js"))
      .count();

    assert_eq!(runtimes, 1);
  }

  #[test]
  fn polar_directory_collision() {
    let (_, ran) =
      built(&[("src/_polar/x.px", HELLO.as_bytes())], &["build", "src"]);

    assert_eq!(ran.code, 1);
    assert!(ran.err.contains("collision"), "{}", ran.err);
  }

  #[test]
  fn failed_build_writes_nothing() {
    let (dir, ran) = built(
      &[("src/good.px", HELLO.as_bytes()), ("src/bad.px", BROKEN.as_bytes())],
      &["build", "src", "--out", "dist"],
    );

    assert_eq!(ran.code, 1);
    assert!(ran.err.contains("src/bad.px"), "{}", ran.err);
    assert!(ran.err.contains("aborting due to"), "{}", ran.err);
    assert!(!dir.path().join("dist").exists());
  }

  #[test]
  fn failed_build_leaves_existing_output() {
    let dir = project(&[
      ("src/good.px", HELLO.as_bytes()),
      ("src/bad.px", BROKEN.as_bytes()),
      ("dist/good.js", b"old"),
    ]);
    let ran = polar(dir.path(), &["build", "src", "--out", "dist"]);

    assert_eq!(ran.code, 1);
    assert_eq!(tree(&dir.path().join("dist")), ["good.js"]);
    assert_eq!(fs::read(dir.path().join("dist/good.js")).unwrap(), b"old");
  }

  fn warning_compile(
    src: &str,
    name: &str,
    options: &CompileOptions,
  ) -> CompileOutput {
    let mut out = polar_compiler::compile(src, name, options);
    let span = polar_compiler::shared::source::Span::empty(name.into(), 0);

    out.diagnostics.push(Diagnostic::warning(
      DiagnosticCode::UnknownName,
      "a warning",
      Label::new(span),
    ));
    out
  }

  #[test]
  fn warnings_do_not_fail() {
    let dir = project(&[("a.px", HELLO.as_bytes())]);
    let compiler = Compiler { compile: warning_compile, ..Compiler::default() };
    let ran = polar_in(dir.path(), &["build", "a.px"], &[], compiler);

    assert_eq!(ran.code, 0, "{}", ran.err);
    assert!(ran.err.contains("warning"), "{}", ran.err);
    assert!(dir.path().join("dist/a.js").exists());
  }

  #[test]
  fn timing_line() {
    let (_, ran) = built(
      &[("a.px", HELLO.as_bytes()), ("b.px", HELLO.as_bytes())],
      &["build", "a.px", "b.px"],
    );
    let last = ran.err.lines().last().unwrap_or_default();

    assert!(
      last.starts_with("built 2 files in ") && last.ends_with("ms"),
      "{last}"
    );
  }

  const USES_LIST: &str = "uses\n  Std.List\n\nfunctions\n  main() {\n    Log.info(List.join(List.map(List.range(1, 4), Int.to_string), \", \"))\n  }\n\nexports\n  main\n";

  #[test]
  fn std_modules_are_written_beside_the_runtime() {
    let (dir, ran) = built(
      &[
        ("src/a.px", USES_LIST.as_bytes()),
        ("src/posts/b.px", USES_LIST.as_bytes()),
      ],
      &["build", "src"],
    );

    assert_eq!(ran.code, 0, "{}", ran.err);
    assert_eq!(
      tree(&dir.path().join("dist")),
      [
        "_polar/runtime.js",
        "_polar/std/List.d.ts",
        "_polar/std/List.js",
        "_polar/std/Option.d.ts",
        "_polar/std/Option.js",
        "a.d.ts",
        "a.js",
        "a.js.map",
        "posts/b.d.ts",
        "posts/b.js",
        "posts/b.js.map",
      ]
    );

    let nested =
      fs::read_to_string(dir.path().join("dist/posts/b.js")).unwrap();
    let std =
      fs::read_to_string(dir.path().join("dist/_polar/std/List.js")).unwrap();

    assert!(nested.contains("from \"../_polar/std/List.js\""), "{nested}");
    assert!(
      std.starts_with("import * as $rt from \"../runtime.js\";"),
      "{std}"
    );
  }

  #[test]
  fn no_std_imports_no_std_directory() {
    let (dir, _) = built(&[("a.px", HELLO.as_bytes())], &["build", "a.px"]);

    assert!(!dir.path().join("dist/_polar/std").exists());
  }

  #[test]
  fn check_never_writes() {
    let (dir, ran) = built(
      &[("src/a.px", HELLO.as_bytes())],
      &["check", "src", "--out", "dist"],
    );

    assert_eq!(ran.code, 0, "{}", ran.err);
    assert!(!dir.path().join("dist").exists());
  }

  #[test]
  fn check_exit_code() {
    let dirty =
      project(&[("a.px", HELLO.as_bytes()), ("b.px", BROKEN.as_bytes())]);
    let clean = project(&[("a.px", HELLO.as_bytes())]);

    assert_eq!(polar(dirty.path(), &["check", "."]).code, 1);
    assert_eq!(polar(clean.path(), &["check", "."]).code, 0);
  }

  #[test]
  fn no_color_flag_and_env() {
    let dir = project(&[("b.px", BROKEN.as_bytes())]);
    let forced = polar_in(
      dir.path(),
      &["check", "b.px", "--color"],
      &[],
      Compiler::default(),
    );
    let env = polar_in(
      dir.path(),
      &["check", "b.px"],
      &[("NO_COLOR", "1")],
      Compiler::default(),
    );
    let flag = polar(dir.path(), &["check", "b.px", "--no-color"]);

    assert!(forced.err.contains('\x1b'), "--color did not colour");
    assert!(!env.err.contains('\x1b'));
    assert!(!flag.err.contains('\x1b'));
  }
}

mod fmt {
  use super::*;

  const MANGLED: &str =
    "functions\n main(){Log.info(\"x\")}\nexports\n  main\n";
  const FORMATTED: &str =
    "functions\n  main() {\n    Log.info(\"x\")\n  }\n\nexports\n  main\n";

  #[test]
  fn reformats_a_mangled_file() {
    let dir = project(&[("a.px", MANGLED.as_bytes())]);
    let ran = polar(dir.path(), &["fmt", "a.px"]);

    assert_eq!(ran.code, 0, "{}", ran.err);
    assert_eq!(fs::read_to_string(dir.path().join("a.px")).unwrap(), FORMATTED);
    assert!(ran.err.contains("1 file reformatted"), "{}", ran.err);
  }

  #[test]
  fn is_idempotent() {
    let dir = project(&[
      ("s/a.px", MANGLED.as_bytes()),
      ("s/b.px", FORMATTED.as_bytes()),
    ]);

    assert!(
      polar(dir.path(), &["fmt", "s"]).err.contains("1 file reformatted")
    );
    assert!(
      polar(dir.path(), &["fmt", "s"])
        .err
        .contains("0 files reformatted, 2 unchanged")
    );
  }

  #[test]
  fn unchanged_files_keep_their_mtime() {
    let dir = project(&[("a.px", FORMATTED.as_bytes())]);
    let path = dir.path().join("a.px");
    let old = fs::File::options().write(true).open(&path).unwrap();

    old
      .set_modified(
        std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000),
      )
      .unwrap();
    drop(old);

    let before = fs::metadata(&path).unwrap().modified().unwrap();

    assert_eq!(polar(dir.path(), &["fmt", "a.px"]).code, 0);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), before);
  }

  #[test]
  fn check_on_a_clean_tree() {
    let dir = project(&[("a.px", FORMATTED.as_bytes())]);
    let ran = polar(dir.path(), &["fmt", ".", "--check"]);

    assert_eq!(ran.code, 0, "{}", ran.err);
    assert!(ran.out.is_empty());
  }

  #[test]
  fn check_on_a_dirty_tree() {
    let mut files: Vec<(String, &[u8])> =
      (0..8).map(|i| (format!("s/ok{i}.px"), FORMATTED.as_bytes())).collect();

    files.push(("s/bad1.px".into(), MANGLED.as_bytes()));
    files.push(("s/bad2.px".into(), MANGLED.as_bytes()));

    let refs: Vec<(&str, &[u8])> =
      files.iter().map(|(p, b)| (p.as_str(), *b)).collect();
    let dir = project(&refs);
    let ran = polar(dir.path(), &["fmt", "s", "--check"]);

    assert_eq!(ran.code, 1);
    assert_eq!(
      ran.out,
      "would reformat: s/bad1.px\nwould reformat: s/bad2.px\n"
    );
    assert_eq!(
      fs::read_to_string(dir.path().join("s/bad1.px")).unwrap(),
      MANGLED
    );
    assert!(
      ran.err.contains("2 files would be reformatted, 8 unchanged"),
      "{}",
      ran.err
    );
  }

  #[test]
  fn parse_error_is_reported_not_written() {
    let dir = project(&[("b.px", BROKEN.as_bytes())]);
    let ran = polar(dir.path(), &["fmt", "b.px"]);

    assert_eq!(ran.code, 1);
    assert!(ran.err.contains("error["), "{}", ran.err);
    assert_eq!(fs::read_to_string(dir.path().join("b.px")).unwrap(), BROKEN);
  }

  #[test]
  fn parse_error_does_not_block_others() {
    let dir = project(&[
      ("s/a.px", MANGLED.as_bytes()),
      ("s/b.px", BROKEN.as_bytes()),
      ("s/c.px", MANGLED.as_bytes()),
      ("s/d.px", FORMATTED.as_bytes()),
    ]);
    let ran = polar(dir.path(), &["fmt", "s"]);

    assert_eq!(ran.code, 1);
    assert_eq!(
      fs::read_to_string(dir.path().join("s/a.px")).unwrap(),
      FORMATTED
    );
    assert_eq!(
      fs::read_to_string(dir.path().join("s/c.px")).unwrap(),
      FORMATTED
    );
  }

  #[test]
  fn crlf_counts_as_a_change() {
    let dir = project(&[("a.px", FORMATTED.replace('\n', "\r\n").as_bytes())]);
    let ran = polar(dir.path(), &["fmt", "a.px"]);

    assert!(ran.err.contains("1 file reformatted"), "{}", ran.err);
    assert_eq!(fs::read_to_string(dir.path().join("a.px")).unwrap(), FORMATTED);
  }

  #[test]
  fn summary_counts() {
    let dir = project(&[
      ("s/a.px", MANGLED.as_bytes()),
      ("s/b.px", BROKEN.as_bytes()),
      ("s/c.px", FORMATTED.as_bytes()),
    ]);
    let ran = polar(dir.path(), &["fmt", "s"]);

    assert!(
      ran.err.ends_with("1 file reformatted, 1 unchanged, 1 failed to parse\n"),
      "{}",
      ran.err
    );
  }

  #[test]
  fn cargo_alias_is_configured() {
    let config = fs::read_to_string(concat!(
      env!("CARGO_MANIFEST_DIR"),
      "/../.cargo/config.toml"
    ))
    .expect("read .cargo/config.toml");

    assert!(
      config.contains(r#"polar-fmt = "run -p polar-cli -- fmt projects""#),
      "{config}"
    );
  }

  #[cfg(unix)]
  #[test]
  fn permissions_survive() {
    use std::os::unix::fs::PermissionsExt;

    let dir = project(&[("a.px", MANGLED.as_bytes())]);
    let path = dir.path().join("a.px");

    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    polar(dir.path(), &["fmt", "a.px"]);

    assert_eq!(
      fs::metadata(&path).unwrap().permissions().mode() & 0o777,
      0o644
    );
  }
}

mod watch {
  use super::*;

  const WAIT: Duration = Duration::from_secs(10);
  const QUIET: Duration = Duration::from_millis(600);

  fn start(
    dir: &Path,
    mode: WatchMode,
  ) -> (Watch, mpsc::Receiver<CycleResult>) {
    let (tx, rx) = mpsc::channel();
    let watch = Watch::start(WatchOptions {
      paths: vec![PathBuf::from("src")],
      cwd: dir.to_path_buf(),
      mode,
      debounce: Duration::from_millis(300),
      compiler: Compiler::default(),
      color: false,
      clear_screen: false,
      debug: false,
      err: Box::new(std::io::sink()),
      on_cycle: Box::new(move |result| {
        let _ = tx.send(result.clone());
      }),
    })
    .expect("start watching");

    (watch, rx)
  }

  fn build_mode() -> WatchMode {
    WatchMode::Build { out: PathBuf::from("dist") }
  }

  #[test]
  fn first_cycle_runs_immediately() {
    let dir = project(&[("src/a.px", HELLO.as_bytes())]);
    let (watch, rx) = start(dir.path(), build_mode());
    let first = rx.recv_timeout(WAIT).expect("a first cycle");

    assert_eq!((first.compiled, first.errors), (1, 0));
    assert!(dir.path().join("dist/a.js").exists());
    watch.stop();
  }

  #[test]
  fn edit_triggers_one_cycle() {
    let dir = project(&[("src/a.px", HELLO.as_bytes())]);
    let (watch, rx) = start(dir.path(), build_mode());

    rx.recv_timeout(WAIT).unwrap();
    fs::write(dir.path().join("src/a.px"), HELLO.replace("hello", "hi"))
      .unwrap();

    let cycle = rx.recv_timeout(WAIT).expect("a cycle after the edit");

    assert_eq!(cycle.compiled, 1);
    assert!(rx.recv_timeout(QUIET).is_err(), "a second cycle for one edit");
    watch.stop();
  }

  #[test]
  fn burst_coalesces() {
    let dir = project(&[("src/a.px", HELLO.as_bytes())]);
    let (watch, rx) = start(dir.path(), build_mode());

    rx.recv_timeout(WAIT).unwrap();

    for i in 0..5 {
      fs::write(
        dir.path().join("src/a.px"),
        HELLO.replace("hello", &format!("hi {i}")),
      )
      .unwrap();
    }

    rx.recv_timeout(WAIT).expect("a cycle after the burst");
    assert!(
      rx.recv_timeout(QUIET).is_err(),
      "the burst took more than one cycle"
    );
    watch.stop();
  }

  #[test]
  fn error_does_not_terminate_and_fixing_clears_it() {
    let dir = project(&[("src/a.px", HELLO.as_bytes())]);
    let (watch, rx) = start(dir.path(), WatchMode::Check { out: None });
    let path = dir.path().join("src/a.px");

    rx.recv_timeout(WAIT).unwrap();
    fs::write(&path, BROKEN).unwrap();
    assert!(rx.recv_timeout(WAIT).expect("a cycle for the error").errors > 0);

    fs::write(&path, HELLO).unwrap();
    assert_eq!(rx.recv_timeout(WAIT).expect("still watching").errors, 0);
    watch.stop();
  }

  #[test]
  fn fixing_writes_the_output() {
    let dir = project(&[("src/a.px", BROKEN.as_bytes())]);
    let (watch, rx) = start(dir.path(), build_mode());

    assert!(rx.recv_timeout(WAIT).unwrap().errors > 0);
    assert!(!dir.path().join("dist/a.js").exists());

    fs::write(dir.path().join("src/a.px"), HELLO).unwrap();

    let cycle = rx.recv_timeout(WAIT).unwrap();

    assert_eq!(cycle.errors, 0);
    assert!(dir.path().join("dist/a.js").exists());
    watch.stop();
  }

  #[test]
  fn partial_success_writes() {
    let dir = project(&[
      ("src/good.px", HELLO.as_bytes()),
      ("src/bad.px", BROKEN.as_bytes()),
    ]);
    let (watch, rx) = start(dir.path(), build_mode());
    let first = rx.recv_timeout(WAIT).unwrap();

    assert!(first.errors > 0);
    assert!(dir.path().join("dist/good.js").exists());
    assert!(!dir.path().join("dist/bad.js").exists());
    watch.stop();
  }

  #[test]
  fn delete_removes_outputs() {
    let dir = project(&[
      ("src/a.px", HELLO.as_bytes()),
      ("src/b.px", HELLO.as_bytes()),
    ]);
    let (watch, rx) = start(dir.path(), build_mode());

    rx.recv_timeout(WAIT).unwrap();
    assert!(dir.path().join("dist/b.js").exists());

    fs::remove_file(dir.path().join("src/b.px")).unwrap();

    let cycle = rx.recv_timeout(WAIT).expect("a cycle for the deletion");

    assert_eq!(cycle.removed.len(), 2, "{cycle:?}");
    assert!(!dir.path().join("dist/b.js").exists());
    assert!(!dir.path().join("dist/b.js.map").exists());
    assert!(dir.path().join("dist/a.js").exists());
    watch.stop();
  }

  #[test]
  fn new_file_is_picked_up() {
    let dir = project(&[("src/a.px", HELLO.as_bytes())]);
    let (watch, rx) = start(dir.path(), build_mode());

    rx.recv_timeout(WAIT).unwrap();
    fs::write(dir.path().join("src/new.px"), HELLO).unwrap();
    rx.recv_timeout(WAIT).expect("a cycle for the new file");
    assert!(dir.path().join("dist/new.js").exists());

    fs::create_dir(dir.path().join("src/deep")).unwrap();
    fs::write(dir.path().join("src/deep/newer.px"), HELLO).unwrap();

    let deadline = std::time::Instant::now() + WAIT;

    while !dir.path().join("dist/deep/newer.js").exists() {
      let left = deadline.saturating_duration_since(std::time::Instant::now());

      rx.recv_timeout(left).expect("a cycle for the file in the new directory");
    }

    watch.stop();
  }
}

mod projects {
  use super::*;
  use polar_cli::project::{self, Target};

  fn config(name: &str, extra: &str) -> String {
    format!("[project]\nname = \"{name}\"\n{extra}")
  }

  #[test]
  fn each_project_builds_into_its_own_out() {
    let a = config("a", "");
    let b = config("b", "");
    let dir = project(&[
      ("projects/a/polar.toml", a.as_bytes()),
      ("projects/a/src/main.px", HELLO.as_bytes()),
      ("projects/b/polar.toml", b.as_bytes()),
      ("projects/b/src/main.px", HELLO.as_bytes()),
    ]);
    let ran = polar(dir.path(), &["build", "projects/a", "projects/b"]);

    assert_eq!(ran.code, 0, "{}", ran.err);
    assert!(dir.path().join("projects/a/dist/main.js").exists());
    assert!(dir.path().join("projects/b/dist/_polar/runtime.js").exists());
    assert!(!dir.path().join("dist").exists());
    assert!(
      ran.err.contains("built a (1 file) into projects/a/dist"),
      "{}",
      ran.err
    );
  }

  #[test]
  fn no_arguments_inside_a_project() {
    let toml = config("p", "");
    let dir = project(&[
      ("polar.toml", toml.as_bytes()),
      ("src/main.px", HELLO.as_bytes()),
    ]);

    assert_eq!(polar(dir.path(), &["build"]).code, 0);
    assert!(dir.path().join("dist/main.js").exists());
    assert_eq!(polar(dir.path(), &["check"]).code, 0);
    assert_eq!(polar(dir.path(), &["fmt", "--check"]).code, 0);
  }

  #[test]
  fn no_arguments_outside_a_project_is_refused() {
    let dir = project(&[("src/main.px", HELLO.as_bytes())]);
    let ran = polar(dir.path(), &["fmt"]);

    assert_eq!(ran.code, 1);
    assert!(ran.err.contains("polar.toml"), "{}", ran.err);
  }

  #[test]
  fn configured_paths() {
    let toml =
      config("p", "src = \"lib\"\nout = \"build/js\"\nmain = \"lib/app.px\"\n");
    let dir = project(&[
      ("p/polar.toml", toml.as_bytes()),
      ("p/lib/app.px", HELLO.as_bytes()),
    ]);

    assert_eq!(polar(dir.path(), &["build", "p"]).code, 0);
    assert!(dir.path().join("p/build/js/app.js").exists());
    assert_eq!(
      project::run_file(Some(Path::new("p")), dir.path()).unwrap(),
      PathBuf::from("p/lib/app.px")
    );
  }

  #[test]
  fn out_dir_inside_src_is_skipped() {
    let toml = config("p", "src = \".\"\n");
    let dir = project(&[
      ("p/polar.toml", toml.as_bytes()),
      ("p/main.px", HELLO.as_bytes()),
      ("p/dist/stale.px", BROKEN.as_bytes()),
    ]);

    assert_eq!(polar(dir.path(), &["check", "p"]).code, 0);
  }

  #[test]
  fn out_flag_overrides_one_project() {
    let toml = config("p", "");
    let dir = project(&[
      ("p/polar.toml", toml.as_bytes()),
      ("p/src/main.px", HELLO.as_bytes()),
    ]);

    assert_eq!(
      polar(dir.path(), &["build", "p", "--out", "elsewhere"]).code,
      0
    );
    assert!(dir.path().join("elsewhere/main.js").exists());
  }

  #[test]
  fn out_flag_with_two_projects_is_refused() {
    let targets = project::targets(
      &["a".into(), "b".into()],
      project(&[
        ("a/polar.toml", config("a", "").as_bytes()),
        ("b/polar.toml", config("b", "").as_bytes()),
      ])
      .path(),
      Some(Path::new("x")),
    );

    assert!(
      matches!(targets, Err(polar_cli::CliError::Message(m)) if m.contains("one project at a time"))
    );
  }

  #[test]
  fn plain_paths_and_projects_mix() {
    let dir = project(&[("p/polar.toml", config("p", "").as_bytes())]);
    let targets =
      project::targets(&["p".into(), "other".into()], dir.path(), None)
        .unwrap();

    assert_eq!(
      targets,
      [
        Target {
          paths: vec!["p/src".into()],
          out: "p/dist".into(),
          project: Some("p".into()),
          hosts: vec![],
          library: false,
          start: Some(project::Start {
            dir: "p".into(),
            main: "main.js".into(),
            source: "src/main.px".into(),
            launch: None,
          }),
        },
        Target {
          paths: vec!["other".into()],
          out: "dist".into(),
          project: None,
          hosts: vec![],
          library: false,
          start: None,
        },
      ]
    );
  }

  const TWO_HOSTS: &str = "module Main\n\nhosts\n  Browser\n  Node\n\neffects\n  Clock in Browser {\n    now() -> Int\n  }\n\nbinds\n  Clock in Browser {\n    now() {\n      1\n    }\n  }\n\n  Clock in Node {\n    now() {\n      2\n    }\n  }\n\nfunctions\n  main() {\n    Log.info(\"#{Clock.now()}\")\n  }\n\nexports\n  main\n";

  #[test]
  fn configured_hosts_build_each_into_its_own_dir() {
    let toml = config("p", "hosts = [\"Browser\", \"Node\"]\n");
    let dir = project(&[
      ("p/polar.toml", toml.as_bytes()),
      ("p/src/main.px", TWO_HOSTS.as_bytes()),
    ]);
    let ran = polar(dir.path(), &["build", "p"]);

    assert_eq!(ran.code, 0, "{}", ran.err);

    for host in ["Browser", "Node"] {
      assert!(dir.path().join(format!("p/dist/{host}/main.js")).exists());
      assert!(
        dir.path().join(format!("p/dist/{host}/_polar/runtime.js")).exists()
      );
      assert!(
        ran
          .err
          .contains(&format!("built p (1 file) for {host} into p/dist/{host}")),
        "{}",
        ran.err
      );
    }
  }

  #[test]
  fn host_flag_builds_one_host() {
    let toml = config("p", "hosts = [\"Browser\", \"Node\"]\n");
    let dir = project(&[
      ("p/polar.toml", toml.as_bytes()),
      ("p/src/main.px", TWO_HOSTS.as_bytes()),
    ]);

    assert_eq!(polar(dir.path(), &["build", "p", "--host", "Node"]).code, 0);
    assert!(dir.path().join("p/dist/main.js").exists());
    assert!(!dir.path().join("p/dist/Node").exists());
  }

  #[test]
  fn unconfigured_hosts_are_refused_with_a_hint() {
    let dir = project(&[
      ("p/polar.toml", config("p", "").as_bytes()),
      ("p/src/main.px", TWO_HOSTS.as_bytes()),
    ]);
    let ran = polar(dir.path(), &["build", "p"]);

    assert_eq!(ran.code, 1);
    assert!(ran.err.contains("hosts = [\"Browser\", \"Node\"]"), "{}", ran.err);
  }

  #[test]
  fn configured_host_must_be_declared() {
    let dir = project(&[
      ("p/polar.toml", config("p", "hosts = [\"Mars\"]\n").as_bytes()),
      ("p/src/main.px", TWO_HOSTS.as_bytes()),
    ]);
    let ran = polar(dir.path(), &["build", "p"]);

    assert_eq!(ran.code, 1);
    assert!(
      ran.err.contains(
        "`polar.toml` lists host `Mars`, but this program declares `Browser`, `Node`"
      ),
      "{}",
      ran.err
    );
  }

  #[test]
  fn single_configured_host_builds_into_out() {
    let dir = project(&[
      ("p/polar.toml", config("p", "hosts = [\"Node\"]\n").as_bytes()),
      ("p/src/main.px", TWO_HOSTS.as_bytes()),
    ]);

    assert_eq!(polar(dir.path(), &["build", "p"]).code, 0);
    assert!(dir.path().join("p/dist/main.js").exists());
  }

  #[test]
  fn invalid_config() {
    let dir = project(&[
      ("typo/polar.toml", b"[project]\nname = \"t\"\nsrcs = \"lib\"\n"),
      ("nameless/polar.toml", b"[project]\n"),
      ("absolute/polar.toml", b"[project]\nname = \"a\"\nout = \"/tmp/x\"\n"),
    ]);

    for (path, needle) in
      [("typo", "srcs"), ("nameless", "name"), ("absolute", "relative")]
    {
      let ran = polar(dir.path(), &["build", path]);

      assert_eq!(ran.code, 1, "{path}");
      assert!(
        ran.err.contains(&format!("invalid {path}/polar.toml")),
        "{}",
        ran.err
      );
      assert!(ran.err.contains(needle), "{path}: {}", ran.err);
    }
  }

  #[test]
  fn a_failing_project_writes_nothing_anywhere() {
    let dir = project(&[
      ("a/polar.toml", config("a", "").as_bytes()),
      ("a/src/main.px", HELLO.as_bytes()),
      ("b/polar.toml", config("b", "").as_bytes()),
      ("b/src/main.px", BROKEN.as_bytes()),
    ]);

    assert_eq!(polar(dir.path(), &["build", "a", "b"]).code, 1);
    assert!(!dir.path().join("a/dist").exists());
    assert!(!dir.path().join("b/dist").exists());
  }

  #[test]
  fn watch_takes_one_project() {
    let dir = project(&[
      ("a/polar.toml", config("a", "").as_bytes()),
      ("b/polar.toml", config("b", "").as_bytes()),
    ]);
    let ran = polar(dir.path(), &["build", "a", "b", "--watch"]);

    assert_eq!(ran.code, 1);
    assert!(ran.err.contains("one project"), "{}", ran.err);
  }
}

mod types {
  use super::*;

  fn root() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/.."))
  }

  fn fixture(name: &str) -> PathBuf {
    PathBuf::from(concat!(
      env!("CARGO_MANIFEST_DIR"),
      "/tests/fixtures/imports"
    ))
    .join(name)
  }

  #[test]
  fn emit_types_cli() {
    let ran = polar(
      &root(),
      &["build", "--emit=types", "cli/tests/fixtures/run/hello.px"],
    );

    assert_eq!(ran.code, 0, "{}", ran.err);
    assert_eq!(ran.out, "main : function() -> {}\n");
  }

  #[test]
  fn user_type_in_scope() {
    let ran = polar(&root(), &["check", "projects/complex"]);

    assert_eq!(ran.code, 0, "{}", ran.err);
  }

  #[test]
  fn user_scheme() {
    let ran = polar(
      &root(),
      &["build", "--emit=types", "projects/complex/src/complex.px"],
    );
    let complex = "{ i: Float, r: Float } as Complex";

    assert_eq!(ran.code, 0, "{}", ran.err);
    assert!(
      ran.out.contains(&format!(
        "add : function({complex}, {complex}) -> {complex}\n"
      )),
      "{}",
      ran.out
    );
  }

  #[test]
  fn user_scheme_seen_by_importer() {
    let dir = project(&[
      ("src/complex.px", fs::read(root().join("projects/complex/src/complex.px")).unwrap().as_slice()),
      ("src/main.px", b"module Main\n\nuses\n  Complex\n\nfunctions\n  f() {\n    Complex.add\n  }\n"),
    ]);
    let ran = polar(dir.path(), &["build", "--emit=types", "src/main.px"]);
    let complex = "{ i: Float, r: Float } as Complex";

    assert_eq!(ran.code, 0, "{}", ran.err);
    assert_eq!(
      ran.out,
      format!(
        "f : function() -> function({complex}, {complex}) -> {complex}\n"
      )
    );
  }

  #[test]
  fn transitive_imports() {
    let ran = polar(&fixture("transitive"), &["check"]);

    assert_eq!(ran.code, 0, "{}", ran.err);
  }

  #[test]
  fn transitive_types_are_real() {
    let dir = project(&[
      ("src/c.px", fs::read(fixture("transitive/src/c.px")).unwrap().as_slice()),
      ("src/b.px", fs::read(fixture("transitive/src/b.px")).unwrap().as_slice()),
      ("src/main.px", b"module Main\n\nuses\n  B\n\nfunctions\n  main() {\n    B.describe(1)\n  }\n"),
    ]);
    let ran = polar(dir.path(), &["check", "src/main.px"]);

    assert_eq!(ran.code, 1);
    assert!(ran.err.contains("POLAR0501"), "{}", ran.err);
    assert!(ran.err.contains("expected `Shape`"), "{}", ran.err);
  }

  #[test]
  fn error_reported_once() {
    let ran = polar(&fixture("error_once"), &["check"]);

    assert_eq!(ran.code, 1);
    assert_eq!(ran.err.matches("POLAR0501").count(), 1, "{}", ran.err);
    assert!(ran.err.contains("b.px"), "{}", ran.err);
  }

  #[test]
  fn import_cycle() {
    let ran = polar(&fixture("cycle"), &["check", "src/a.px"]);

    assert_eq!(ran.code, 1);
    assert!(ran.err.contains("POLAR0314"), "{}", ran.err);
    assert!(ran.err.contains("A → B → A"), "{}", ran.err);
    assert!(!ran.err.contains("POLAR0303"), "{}", ran.err);
  }

  #[test]
  fn projects_check() {
    for name in ["complex", "todo", "mean_time_of_day"] {
      let ran = polar(&root().join("projects").join(name), &["check"]);

      assert_eq!(ran.code, 0, "{name}: {}", ran.err);
    }
  }
}
