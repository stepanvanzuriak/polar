use std::{fs, path::Path, process::Command};
use tempfile::TempDir;

const POLAR: &str = env!("CARGO_BIN_EXE_polar");

const PROGRAM: &str = "uses
  Std.Bytes
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

#[test]
fn write_bytes_roundtrip() {
  let (out, dir) = lines(
    "    let data = Bytes.from_list([0, 255, 16])\n    Log.info(done(Fs.write_bytes(\"f.bin\", data)))\n    match Fs.read_bytes(\"f.bin\") {\n      Ok(back) -> Log.info(\"same #{back == data}\"),\n      Err(e) -> Log.info(e.code),\n    }",
    &[],
  );

  assert_eq!(out, "ok\nsame true\n");
  assert_eq!(fs::read(dir.path().join("f.bin")).unwrap(), [0, 255, 16]);
}

#[test]
fn temp_dirs_distinct() {
  let (out, _) = lines(
    "    match Fs.temp_dir(\"polar-\") {\n      Err(e) -> Log.info(e.code),\n      Ok(a) -> match Fs.temp_dir(\"polar-\") {\n        Err(e) -> Log.info(e.code),\n        Ok(b) -> {\n          Log.info(\"distinct #{a != b} #{Fs.is_dir(a)}\")\n          Log.info(done(Fs.remove_all(a)))\n          Log.info(done(Fs.remove_all(b)))\n        }\n      }\n    }",
    &[],
  );

  assert_eq!(out, "distinct true true\nok\nok\n");
}

#[test]
fn copy_file_and_tree() {
  let (out, dir) = lines(
    "    Log.info(done(Fs.copy(\"src\", \"out/tree\")))\n    Log.info(text(Fs.read(\"out/tree/d/b.txt\")))\n    Log.info(done(Fs.copy(\"src/a.txt\", \"one.txt\")))\n    Log.info(done(Fs.copy(\"none\", \"x\")))",
    &["src/a.txt", "src/d/b.txt"],
  );

  assert_eq!(out, "ok\nok \nok\nerr ENOENT\n");
  assert!(dir.path().join("out/tree/a.txt").is_file());
  assert!(dir.path().join("one.txt").is_file());
  assert!(!dir.path().join("x").exists());
}

#[cfg(unix)]
#[test]
fn copy_keeps_mode() {
  use std::os::unix::fs::PermissionsExt;

  let ran =
    run("    Log.info(done(Fs.copy(\"src\", \"dst\")))", &["src/run.sh"]);

  assert_eq!(ran.code, Some(0), "{}", ran.err);
  fs::set_permissions(
    ran.dir.path().join("src/run.sh"),
    fs::Permissions::from_mode(0o750),
  )
  .unwrap();

  let tmp = tempfile::tempdir().unwrap();
  Command::new(POLAR)
    .args(["run", "a.px"])
    .current_dir(ran.dir.path())
    .env("TMPDIR", tmp.path())
    .output()
    .unwrap();

  let mode = fs::metadata(ran.dir.path().join("dst/run.sh"))
    .unwrap()
    .permissions()
    .mode();

  assert_eq!(mode & 0o777, 0o750);
}

#[cfg(unix)]
#[test]
fn symlink() {
  let (out, dir) = lines(
    "    Log.info(done(Fs.symlink(\"real.txt\", \"link\")))\n    Log.info(text(Fs.read(\"link\")))\n    Log.info(done(Fs.symlink(\"real.txt\", \"link\")))\n    Log.info(done(Fs.symlink(\"real.txt\", \"no/dir/link\")))",
    &["real.txt"],
  );

  assert_eq!(out, "ok\nok \nerr EEXIST\nerr ENOENT\n");
  assert_eq!(
    fs::read_link(dir.path().join("link")).unwrap(),
    Path::new("real.txt")
  );
}

#[test]
fn set_mtime() {
  let (out, dir) = lines(
    "    Log.info(done(Fs.set_mtime(\"f\", 1500000000000)))\n    Log.info(done(Fs.set_mtime(\"gone\", 1500000000000)))",
    &["f"],
  );
  let modified = fs::metadata(dir.path().join("f"))
    .unwrap()
    .modified()
    .unwrap()
    .duration_since(std::time::UNIX_EPOCH)
    .unwrap();

  assert_eq!(out, "ok\nerr ENOENT\n");
  assert_eq!(modified.as_secs(), 1_500_000_000);
}

#[cfg(unix)]
#[test]
fn chmod() {
  use std::os::unix::fs::PermissionsExt;

  let (out, dir) = lines(
    "    Log.info(done(Fs.chmod(\"f\", 493)))\n    Log.info(done(Fs.chmod(\"gone\", 493)))",
    &["f"],
  );
  let mode = fs::metadata(dir.path().join("f")).unwrap().permissions().mode();

  assert_eq!(out, "ok\nerr ENOENT\n");
  assert_eq!(mode & 0o777, 0o755);
}
