use std::{
  fmt::Write as _,
  io::{BufRead, BufReader, Write},
  path::PathBuf,
  process::{Child, ChildStdin, Command, Stdio},
  sync::{
    atomic::{AtomicUsize, Ordering},
    mpsc::{self, Receiver},
  },
  time::Duration,
};

pub fn node_binary() -> PathBuf {
  let node = std::env::var_os("POLAR_NODE")
    .map_or_else(|| PathBuf::from("node"), PathBuf::from);

  let runs = Command::new(&node)
    .arg("--version")
    .output()
    .is_ok_and(|out| out.status.success());

  assert!(
    runs,
    "Node.js not found: tried `{}` (set POLAR_NODE to a node binary, or put \
     `node` on PATH)",
    node.display()
  );

  node
}

pub fn node_check_module(source: &str) -> Result<(), String> {
  static NEXT: AtomicUsize = AtomicUsize::new(0);

  let path = std::env::temp_dir().join(format!(
    "polar-check-{}-{}.mjs",
    std::process::id(),
    NEXT.fetch_add(1, Ordering::Relaxed)
  ));

  std::fs::write(&path, source).expect("write the module to check");

  let out = Command::new(node_binary())
    .arg("--check")
    .arg(&path)
    .output()
    .expect("spawn node --check");

  let _ = std::fs::remove_file(&path);

  if out.status.success() {
    Ok(())
  } else {
    Err(String::from_utf8_lossy(&out.stderr).into_owned())
  }
}

const EVALUATOR: &str = r#"
const rl = require("node:readline").createInterface({ input: process.stdin });
const render = (v) =>
  typeof v === "number" && Object.is(v, -0) ? "-0" : String(v);
rl.on("line", (line) => {
  let out;
  try {
    out = render(new Function("return " + line)());
  } catch (e) {
    out = "throw: " + e.message;
  }
  process.stdout.write(out.replace(/\n/g, " ") + "\n");
});
"#;

pub struct Evaluator {
  child: Child,
  stdin: ChildStdin,
  lines: Receiver<String>,
}

impl Evaluator {
  const TIMEOUT: Duration = Duration::from_secs(10);

  pub fn new() -> Self {
    Self::with_script(EVALUATOR)
  }

  pub fn with_script(script: &str) -> Self {
    let mut child = Command::new(node_binary())
      .args(["-e", script])
      .stdin(Stdio::piped())
      .stdout(Stdio::piped())
      .stderr(Stdio::inherit())
      .spawn()
      .expect("spawn the node evaluator");

    let stdin = child.stdin.take().expect("evaluator stdin");
    let stdout = child.stdout.take().expect("evaluator stdout");
    let (tx, lines) = mpsc::channel();

    std::thread::spawn(move || {
      for line in BufReader::new(stdout).lines() {
        let Ok(line) = line else { break };

        if tx.send(line).is_err() {
          break;
        }
      }
    });

    Self { child, stdin, lines }
  }

  pub fn eval(&mut self, expression: &str) -> String {
    assert!(!expression.contains('\n'), "one expression per line");

    writeln!(self.stdin, "{expression}").expect("write to the evaluator");
    self.stdin.flush().expect("flush the evaluator");

    self.lines.recv_timeout(Self::TIMEOUT).unwrap_or_else(|err| {
      panic!("the evaluator did not answer `{expression}`: {err}")
    })
  }
}

impl Drop for Evaluator {
  fn drop(&mut self) {
    let _ = self.child.kill();
    let _ = self.child.wait();
  }
}

pub fn file_url(path: &std::path::Path) -> String {
  let mut url = String::from("file://");

  for byte in path.to_str().expect("a UTF-8 path").bytes() {
    if byte.is_ascii_alphanumeric() || b"-._~/".contains(&byte) {
      url.push(char::from(byte));
    } else {
      let _ = write!(url, "%{byte:02X}");
    }
  }

  url
}

pub fn runtime_url() -> String {
  let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
    .join("../runtime/runtime.js")
    .canonicalize()
    .expect("runtime/runtime.js exists");

  file_url(&path)
}

pub struct TempDir {
  pub path: PathBuf,
}

impl TempDir {
  pub const PREFIX: &str = "polar-exec-";

  pub fn new() -> Self {
    static NEXT: AtomicUsize = AtomicUsize::new(0);

    let path = std::env::temp_dir().join(format!(
      "{}{}-{}",
      Self::PREFIX,
      std::process::id(),
      NEXT.fetch_add(1, Ordering::Relaxed)
    ));

    std::fs::create_dir(&path).expect("create a fresh temp dir");
    Self { path }
  }
}

impl Drop for TempDir {
  fn drop(&mut self) {
    let _ = std::fs::remove_dir_all(&self.path);
  }
}

pub fn run_main_in(dir: &TempDir, js: &str) -> Result<String, String> {
  let module = dir.path.join("main.mjs");

  std::fs::write(&module, js).expect("write the compiled module");

  let out = Command::new(node_binary())
    .args([
      "--input-type=module",
      "-e",
      "const m = await import(process.argv[1]); await m.main();",
    ])
    .arg(file_url(&module))
    .output()
    .expect("spawn node");

  if out.status.success() {
    Ok(String::from_utf8(out.stdout).expect("UTF-8 stdout"))
  } else {
    Err(String::from_utf8_lossy(&out.stderr).into_owned())
  }
}

pub fn run_main(js: &str) -> Result<String, String> {
  run_main_in(&TempDir::new(), js)
}

pub fn run_program(
  src: &str,
  filename: &str,
  modules: &[polar_compiler::shared::modules::ModuleSource],
) -> Result<String, String> {
  run_program_with(src, filename, modules, None, &[])
}

pub fn run_program_with(
  src: &str,
  filename: &str,
  modules: &[polar_compiler::shared::modules::ModuleSource],
  host: Option<&str>,
  files: &[(&str, &str)],
) -> Result<String, String> {
  run_program_plugins(src, filename, modules, host, files, &[])
}

pub fn run_program_plugins(
  src: &str,
  filename: &str,
  modules: &[polar_compiler::shared::modules::ModuleSource],
  host: Option<&str>,
  files: &[(&str, &str)],
  plugins: &[&str],
) -> Result<String, String> {
  use polar_compiler::{CompileOptions, HostOption, compile, stdlib};

  let dir = TempDir::new();

  for (name, text) in files {
    std::fs::write(dir.path.join(name), text).expect("write a helper file");
  }
  let polar = dir.path.join("_polar");
  let std_dir = polar.join("std");

  std::fs::create_dir_all(&std_dir).expect("create _polar/std");
  std::fs::copy(
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../runtime/runtime.js"),
    polar.join("runtime.js"),
  )
  .expect("copy the runtime");

  let options = CompileOptions {
    runtime: "./_polar/runtime.js".to_string(),
    modules: modules.to_vec(),
    host: host
      .map_or(HostOption::Auto, |h| HostOption::Fixed(Some(h.to_string()))),
    plugins: plugins.iter().map(ToString::to_string).collect(),
    ..CompileOptions::default()
  };
  let main = compile(src, filename, &options);

  if !main.diagnostics.is_empty() {
    return Err(format!(
      "{filename} does not compile: {:#?}",
      main.diagnostics
    ));
  }

  let mut todo = main.std_imports.clone();

  for module in modules {
    let module_options =
      CompileOptions { plugins: module.plugins.clone(), ..options.clone() };
    let out =
      compile(&module.source, &format!("{}.px", module.path), &module_options);

    if !out.diagnostics.is_empty() {
      return Err(format!(
        "{} does not compile: {:#?}",
        module.path, out.diagnostics
      ));
    }

    todo.extend(out.std_imports.iter().cloned());

    let target = dir.path.join(module.specifier.trim_start_matches("./"));

    std::fs::write(target, out.js).expect("write a module");
  }

  let mut done: Vec<String> = Vec::new();

  while let Some(name) = todo.pop() {
    if done.contains(&name) {
      continue;
    }

    let std = stdlib::module(&name).expect("a std module");
    let std_options = CompileOptions {
      runtime: "../runtime.js".to_string(),
      ..CompileOptions::default()
    };
    let out = compile(std.source, &stdlib::filename(&name), &std_options);

    if !out.diagnostics.is_empty() {
      return Err(format!(
        "Std.{name} does not compile: {:#?}",
        out.diagnostics
      ));
    }

    std::fs::write(std_dir.join(format!("{name}.js")), &out.js)
      .expect("write a std module");
    todo.extend(out.std_imports);
    done.push(name);
  }

  run_main_in(&dir, &main.js).map_err(|e| format!("{}\n{e}", main.js))
}
