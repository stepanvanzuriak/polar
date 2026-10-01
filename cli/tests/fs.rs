use std::{fs, path::Path, process::Command};
use tempfile::TempDir;

const POLAR: &str = env!("CARGO_BIN_EXE_polar");

const PROGRAM: &str = "uses
  Std.Fs
  Std.List
  Std.Path
  Std.Result

hosts
  Node

functions
  done(result: Result<FsError, {}>) -> String {
    match result {
      Ok(_) -> \"ok\",
      Err(e) -> \"err #{e.code}\",
    }
  }

  text(result: Result<FsError, String>) -> String {
    match result {
      Ok(s) -> \"ok #{s}\",
      Err(e) -> \"err #{e.code}\",
    }
  }

  names(result: Result<FsError, List<String>>) -> String {
    match result {
      Ok(xs) -> \"ok #{List.join(xs, \",\")}\",
      Err(e) -> \"err #{e.code}\",
    }
  }

  copy(from: String, to: String) -> Result<FsError, {}> / {Fs} {
    Fs.read(from) |> Result.and_then(function(text) {
      Fs.mkdir_all(Path.dirname(to)) |> Result.and_then(function(_) { Fs.write(to, text) })
    })
  }

  main() -> {} / {Fs} {
BODY
  }

exports
  main
";

struct Ran {
  dir: TempDir,
  code: Option<i32>,
  out: String,
  err: String,
}

fn run(body: &str, setup: &[&str]) -> Ran {
  let dir = tempfile::tempdir().unwrap();

  for path in setup {
    let path = dir.path().join(path);

    if path.to_string_lossy().ends_with('/') {
      fs::create_dir_all(&path).unwrap();
    } else {
      fs::create_dir_all(path.parent().unwrap()).unwrap();
      fs::write(&path, "").unwrap();
    }
  }

  fs::write(dir.path().join("a.px"), PROGRAM.replace("BODY", body)).unwrap();

  let tmp = tempfile::tempdir().unwrap();
  let out = Command::new(POLAR)
    .args(["run", "a.px"])
    .current_dir(dir.path())
    .env("TMPDIR", tmp.path())
    .env_remove("POLAR_DEBUG")
    .output()
    .unwrap();

  Ran {
    dir,
    code: out.status.code(),
    out: String::from_utf8_lossy(&out.stdout).into_owned(),
    err: String::from_utf8_lossy(&out.stderr).into_owned(),
  }
}

fn lines(body: &str, setup: &[&str]) -> (String, TempDir) {
  let ran = run(body, setup);

  assert_eq!(ran.code, Some(0), "{}", ran.err);
  (ran.out, ran.dir)
}

#[test]
fn read_missing() {
  let (out, _) = lines(
    "    match Fs.read(\"nope\") {\n      Ok(_) -> Log.info(\"ok\"),\n      Err(e) -> Log.info(\"#{e.code} #{e.path}\"),\n    }",
    &[],
  );

  assert_eq!(out, "ENOENT nope\n");
}

#[test]
fn write_read() {
  let (out, dir) = lines(
    "    Log.info(done(Fs.write(\"f.txt\", \"café\")))\n    Log.info(text(Fs.read(\"f.txt\")))\n    Log.info(done(Fs.write(\"f.txt\", \"é\")))\n    Log.info(text(Fs.read(\"f.txt\")))",
    &[],
  );

  assert_eq!(out, "ok\nok café\nok\nok é\n");
  assert_eq!(fs::read_to_string(dir.path().join("f.txt")).unwrap(), "é");
}

#[test]
fn write_no_parent() {
  let (out, dir) = lines(
    "    Log.info(done(Fs.write(\"x/y/z\", \"t\")))\n    Log.info(done(Fs.append(\"x/y/z\", \"t\")))",
    &[],
  );

  assert_eq!(out, "err ENOENT\nerr ENOENT\n");
  assert!(!dir.path().join("x").exists());
}

#[test]
fn append() {
  let (out, _) = lines(
    "    Log.info(done(Fs.append(\"log\", \"a\")))\n    Log.info(done(Fs.append(\"log\", \"b\")))\n    Log.info(text(Fs.read(\"log\")))",
    &[],
  );

  assert_eq!(out, "ok\nok\nok ab\n");
}

#[test]
fn mkdir_all_twice() {
  let (out, dir) = lines(
    "    Log.info(done(Fs.mkdir_all(\"p/q/r\")))\n    Log.info(done(Fs.mkdir_all(\"p/q/r\")))\n    Log.info(\"#{Fs.is_dir(\"p/q/r\")} #{Fs.exists(\"p/q/r\")} #{Fs.exists(\"p/q/s\")}\")",
    &[],
  );

  assert_eq!(out, "ok\nok\ntrue true false\n");
  assert!(dir.path().join("p/q/r").is_dir());
}

#[test]
fn list_sorted() {
  let (out, _) = lines(
    "    Log.info(names(Fs.list(\"d\")))\n    Log.info(names(Fs.list(\"missing\")))\n    Log.info(\"#{Fs.is_dir(\"d/b\")} #{Fs.exists(\"d/b\")}\")",
    &["d/b", "d/a", "d/c/"],
  );

  assert_eq!(out, "ok a,b,c\nerr ENOENT\nfalse true\n");
}

#[test]
fn walk_relative_sorted() {
  let (out, _) = lines(
    "    Log.info(names(Fs.walk(\"t\")))\n    Log.info(names(Fs.walk(\"t/\")))",
    &["t/y", "t/x/2", "t/x/1", "t/empty/"],
  );

  assert_eq!(out, "ok x/1,x/2,y\nok x/1,x/2,y\n");
}

#[cfg(unix)]
#[test]
fn walk_skips_symlinked_dirs() {
  let ran = run("    Log.info(names(Fs.walk(\"t\")))", &["t/a", "real/b"]);

  std::os::unix::fs::symlink(
    ran.dir.path().join("real"),
    ran.dir.path().join("t/link"),
  )
  .unwrap();

  let tmp = tempfile::tempdir().unwrap();
  let out = Command::new(POLAR)
    .args(["run", "a.px"])
    .current_dir(ran.dir.path())
    .env("TMPDIR", tmp.path())
    .output()
    .unwrap();

  assert_eq!(String::from_utf8_lossy(&out.stdout), "ok a,link\n");
}

#[test]
fn remove() {
  let (out, dir) = lines(
    "    Log.info(done(Fs.remove(\"f\")))\n    Log.info(done(Fs.remove(\"e\")))\n    Log.info(done(Fs.remove(\"full\")))\n    Log.info(done(Fs.remove(\"f\")))",
    &["f", "e/", "full/x"],
  );

  assert_eq!(out, "ok\nok\nerr ENOTEMPTY\nerr ENOENT\n");
  assert!(dir.path().join("full/x").exists());
}

#[test]
fn remove_all_missing() {
  let (out, dir) = lines(
    "    Log.info(done(Fs.remove_all(\"gone\")))\n    Log.info(done(Fs.remove_all(\"tree\")))",
    &["tree/a/b", "tree/c"],
  );

  assert_eq!(out, "ok\nok\n");
  assert!(!Path::new(&dir.path().join("tree")).exists());
}

#[test]
fn copy_example() {
  let (out, dir) = lines(
    "    Log.info(done(copy(\"src/a.txt\", \"out/deep/b.txt\")))\n    Log.info(done(copy(\"src/none\", \"out/c.txt\")))",
    &["src/a.txt"],
  );

  assert_eq!(out, "ok\nerr ENOENT\n");
  assert!(dir.path().join("out/deep/b.txt").is_file());
  assert!(!dir.path().join("out/c.txt").exists());
}

#[test]
fn browser_rejected() {
  let dir = tempfile::tempdir().unwrap();
  let src = "uses\n  Std.Fs\n\nhosts\n  Browser\n\nfunctions\n  main() {\n    Log.info(\"#{Fs.exists(\"x\")}\")\n  }\n\nexports\n  main\n";

  fs::write(dir.path().join("b.px"), src).unwrap();

  let out = Command::new(POLAR)
    .args(["build", "b.px", "--out", "dist"])
    .current_dir(dir.path())
    .output()
    .unwrap();
  let err = String::from_utf8_lossy(&out.stderr);

  assert_eq!(out.status.code(), Some(1));
  assert!(err.contains("`main` can't run on `Browser`: it uses `Fs`"), "{err}");
}
