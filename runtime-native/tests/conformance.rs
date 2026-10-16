use polar_compiler::{
  CompileOptions, compile_native,
  core::serial::{bundle_to_bytes, pack},
};
use std::{
  fs,
  path::{Path, PathBuf},
  process::Command,
};

fn repo() -> PathBuf {
  PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

struct Outcome {
  stdout: String,
  exit_code: i32,
}

const TEXT_FILE_BUSY: i32 = 26;

fn when_not_busy<T>(
  mut start: impl FnMut() -> std::io::Result<T>,
) -> std::io::Result<T> {
  for _ in 0..50 {
    match start() {
      Err(e) if e.raw_os_error() == Some(TEXT_FILE_BUSY) => {
        std::thread::sleep(std::time::Duration::from_millis(20));
      }
      result => return result,
    }
  }

  start()
}

fn run_native(px: &Path, args: &[&str]) -> Result<Outcome, String> {
  let source = fs::read_to_string(px).map_err(|e| e.to_string())?;
  let name = px.file_name().unwrap().to_string_lossy().into_owned();
  let out = compile_native(&source, &name, &CompileOptions::default());

  if let Some(e) = out.host_error {
    return Err(e);
  }

  let Some(bundle) = out.bundle else {
    let messages: Vec<String> =
      out.diagnostics.iter().map(|d| d.message.clone()).collect();

    return Err(format!("does not compile: {}", messages.join("; ")));
  };
  let rt = fs::read(env!("CARGO_BIN_EXE_polar-rt")).unwrap();
  let dir = tempfile::tempdir().unwrap();
  let exe = dir.path().join("app");

  fs::write(&exe, pack(&rt, &bundle_to_bytes(&bundle))).unwrap();

  #[cfg(unix)]
  {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(&exe, fs::Permissions::from_mode(0o755)).unwrap();
  }

  let output =
    when_not_busy(|| Command::new(&exe).args(args).env_clear().output())
      .map_err(|e| e.to_string())?;

  Ok(Outcome {
    stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
    exit_code: output.status.code().unwrap_or(-1),
  })
}

fn expected_for(px: &Path) -> Option<String> {
  let stem = px.file_stem()?.to_string_lossy().into_owned();
  let dir = px.parent()?;

  [format!("{stem}.expected.txt"), format!("{stem}.Node.expected.txt")]
    .iter()
    .find_map(|name| fs::read_to_string(dir.join(name)).ok())
}

fn skip_reason(px: &Path) -> Option<String> {
  sidecar_reason(&px.with_extension("targets"))
}

fn sidecar_reason(sidecar: &Path) -> Option<String> {
  let text = fs::read_to_string(sidecar).ok()?;
  let mut lines = text.lines();
  let targets = lines.next()?.trim().to_string();

  (!targets.split(',').any(|t| t.trim() == "native"))
    .then(|| lines.next().unwrap_or("no reason given").trim().to_string())
}

#[test]
fn every_fixture_matches_on_native() {
  let dir = repo().join("compiler/tests/fixtures/programs");
  let mut programs: Vec<PathBuf> = fs::read_dir(dir)
    .unwrap()
    .map(|e| e.unwrap().path())
    .filter(|p| p.extension().is_some_and(|e| e == "px"))
    .collect();

  programs.sort();

  let mut failures = Vec::new();
  let mut ran = 0;

  for px in &programs {
    let Some(expected) = expected_for(px) else { continue };

    if let Some(reason) = skip_reason(px) {
      eprintln!("skipped on native: {} ({reason})", px.display());
      continue;
    }

    ran += 1;

    match run_native(px, &[]) {
      Ok(out) if out.stdout == expected && out.exit_code == 0 => {}
      Ok(out) => failures.push(format!(
        "{}: exit {} and stdout differs\n--- expected\n{expected}--- native\n{}",
        px.display(),
        out.exit_code,
        out.stdout
      )),
      Err(e) => failures.push(format!("{}: {e}", px.display())),
    }
  }

  assert!(ran >= 8, "only {ran} fixtures ran");
  assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn a_wrong_expectation_fails_the_harness() {
  let px = repo().join("compiler/tests/fixtures/programs/hello.px");
  let out = run_native(&px, &[]).unwrap();

  assert_ne!(out.stdout, "goodbye\n");
  assert_eq!(out.stdout, "hello, world\n");
}

fn write_program(dir: &Path, name: &str, src: &str) -> PathBuf {
  let path = dir.join(name);

  fs::write(&path, src).unwrap();

  path
}

#[test]
fn deep_tail_recursion_and_folds_are_stack_safe() {
  let dir = tempfile::tempdir().unwrap();
  let px = write_program(
    dir.path(),
    "deep.px",
    "uses\n  Std.List\n\nfunctions\n  count(n: Int, acc: Int) -> Int {\n    if n == 0 { acc } else { count(n - 1, acc + 1) }\n  }\n\n  build(n: Int, acc: List<Int>) -> List<Int> {\n    if n == 0 { acc } else { build(n - 1, [n, ..acc]) }\n  }\n\n  main() {\n    Log.info(Int.to_string(count(200000, 0)))\n    Log.info(Int.to_string(List.length(List.map(build(10000, []), function(x) { x + 1 }))))\n  }\n\nexports\n  main\n",
  );
  let out = run_native(&px, &[]).unwrap();

  assert_eq!(out.stdout, "200000\n10000\n");
}

#[test]
fn uncaught_errors_exit_non_zero() {
  let dir = tempfile::tempdir().unwrap();
  let px = write_program(
    dir.path(),
    "boom.px",
    "types\n  Oops = Oops(String)\n\nfunctions\n  main() -> {} / {Throws<Oops>} {\n    Log.info(\"before\")\n    throw Oops(\"bad\")\n  }\n\nexports\n  main\n",
  );
  let out = run_native(&px, &[]).unwrap();

  assert_eq!(out.stdout, "before\n");
  assert_eq!(out.exit_code, 1);
}

#[test]
fn process_args_and_exit_code_pass_through() {
  let dir = tempfile::tempdir().unwrap();
  let px = write_program(
    dir.path(),
    "args.px",
    "uses\n  Std.List\n  Std.Process\n\nhosts\n  Node\n\nfunctions\n  main() -> {} / {Process} {\n    Log.info(List.join(Process.args(), \",\"))\n    Process.set_exit_code(3)\n  }\n\nexports\n  main\n",
  );
  let out = run_native(&px, &["a", "b"]).unwrap();

  assert_eq!(out.stdout, "a,b\n");
  assert_eq!(out.exit_code, 3);
}

#[test]
fn a_bare_runtime_prints_usage() {
  let out =
    Command::new(env!("CARGO_BIN_EXE_polar-rt")).env_clear().output().unwrap();

  assert_eq!(out.status.code(), Some(2));
  assert!(String::from_utf8_lossy(&out.stderr).contains("usage"));
}

fn run_project(dir: &Path) -> Result<String, String> {
  let args: Vec<std::ffi::OsString> =
    ["run", "--target", "native"].into_iter().map(Into::into).collect();
  let rt = env!("CARGO_BIN_EXE_polar-rt").to_string();
  let env = move |name: &str| match name {
    "POLAR_RT" => Some(rt.clone()),
    other => std::env::var(other).ok(),
  };
  let (mut out, mut err) = (Vec::new(), Vec::new());
  let mut io = polar_cli::Io {
    out: &mut out,
    err: &mut err,
    stdout_is_tty: false,
    stderr_is_tty: false,
    cwd: dir.to_path_buf(),
    env: &env,
  };
  let code = polar_cli::run(&args, &mut io);

  if code == 0 {
    Ok(String::from_utf8_lossy(&out).into_owned())
  } else {
    Err(String::from_utf8_lossy(&err).into_owned())
  }
}

fn own_bindings(dir: &Path) -> bool {
  fs::read_dir(dir).into_iter().flatten().flatten().any(|entry| {
    let path = entry.path();

    if path.is_dir() {
      own_bindings(&path)
    } else {
      path.extension().is_some_and(|e| e == "js")
    }
  })
}

#[test]
fn every_project_matches_on_native() {
  let mut projects: Vec<PathBuf> = fs::read_dir(repo().join("projects"))
    .unwrap()
    .map(|e| e.unwrap().path())
    .filter(|p| p.join("snapshots/main.txt").is_file())
    .collect();

  projects.sort();

  let mut failures = Vec::new();

  for dir in &projects {
    let name = dir.file_name().unwrap().to_string_lossy().into_owned();

    if own_bindings(&dir.join("src")) {
      eprintln!(
        "skipped on native: project {name} (it has its own JavaScript bindings)"
      );
      continue;
    }

    let expected = fs::read_to_string(dir.join("snapshots/main.txt")).unwrap();

    match run_project(dir) {
      Ok(stdout) if stdout == expected => {}
      Ok(stdout) => failures.push(format!(
        "{name}: differs\n--- expected\n{expected}--- native\n{stdout}"
      )),
      Err(e) => failures.push(format!("{name}: failed\n{e}")),
    }
  }

  assert!(projects.len() >= 3);
  assert!(failures.is_empty(), "{}", failures.join("\n"));
}

fn failing(body: &str) -> Outcome {
  let dir = tempfile::tempdir().unwrap();
  let src = format!(
    "functions\n  zero() -> Int {{\n    0\n  }}\n\n  big() -> Int {{\n    9007199254740991\n  }}\n\n  main() {{\n    {body}\n  }}\n\nexports\n  main\n"
  );
  let px = write_program(dir.path(), "t.px", &src);

  run_native(&px, &[]).unwrap()
}

#[test]
fn division_by_zero_and_overflow_fail_with_a_non_zero_exit() {
  for body in [
    "Log.info(Int.to_string(1 / zero()))",
    "Log.info(Int.to_string(1 % zero()))",
    "Log.info(Int.to_string(big() + 1))",
    "Log.info(Int.to_string(big() * 2))",
  ] {
    let out = failing(body);

    assert_eq!(out.exit_code, 1, "{body}");
    assert_eq!(out.stdout, "", "{body}");
  }
}

#[test]
fn float_division_by_zero_is_infinity() {
  let out = failing("Log.info(\"#{1.0 / 0.0}\")");

  assert_eq!(out.stdout, "Infinity\n");
}

fn free_port() -> u16 {
  std::net::TcpListener::bind("127.0.0.1:0")
    .unwrap()
    .local_addr()
    .unwrap()
    .port()
}

fn post(port: u16, path: &str, body: &str) -> (u16, String) {
  use std::io::{Read, Write};

  let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();

  write!(
    stream,
    "POST {path} HTTP/1.1\r\nHost: x\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
    body.len()
  )
  .unwrap();

  let mut text = String::new();

  stream.read_to_string(&mut text).unwrap();

  let status = text.split_whitespace().nth(1).unwrap().parse().unwrap();
  let body = text.split("\r\n\r\n").nth(1).unwrap_or("").to_string();

  (status, body)
}

struct Server(std::process::Child);

impl Drop for Server {
  fn drop(&mut self) {
    let _ = self.0.kill();
    let _ = self.0.wait();
  }
}

#[test]
fn a_native_executable_serves_bridged_effects() {
  let px = repo().join("projects/native_bridge_server/src/main.px");
  let source = fs::read_to_string(&px).unwrap();
  let out = compile_native(&source, "main.px", &CompileOptions::default());
  let bundle = out.bundle.expect("compiles");
  let dir = tempfile::tempdir().unwrap();
  let exe = dir.path().join("server");

  fs::write(
    &exe,
    pack(
      &fs::read(env!("CARGO_BIN_EXE_polar-rt")).unwrap(),
      &bundle_to_bytes(&bundle),
    ),
  )
  .unwrap();

  #[cfg(unix)]
  {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(&exe, fs::Permissions::from_mode(0o755)).unwrap();
  }

  let port = free_port();
  let _server = Server(
    when_not_busy(|| {
      Command::new(&exe)
        .args(["--serve", &port.to_string()])
        .stderr(std::process::Stdio::null())
        .spawn()
    })
    .unwrap(),
  );

  for _ in 0..100 {
    if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
      break;
    }

    std::thread::sleep(std::time::Duration::from_millis(50));
  }

  assert_eq!(
    post(port, "/_polar/Posts/count", "{\"args\":[]}"),
    (200, "{\"ok\":2}".to_string())
  );

  let (status, body) = post(port, "/_polar/Posts/fetch", "{\"args\":[1]}");

  assert_eq!(status, 200);
  assert!(body.starts_with("{\"ok\":{\"id\":1,"), "{body}");

  let (status, body) = post(port, "/_polar/Posts/fetch", "{\"args\":[42]}");

  assert_eq!(status, 200);
  assert!(body.contains("\"throw\":{\"tag\":\"Main.Missing\""), "{body}");

  assert_eq!(post(port, "/_polar/Posts/fetch", "{\"args\":[\"x\"]}").0, 400);
  assert_eq!(post(port, "/_polar/Nope/x", "{}").0, 404);
}
