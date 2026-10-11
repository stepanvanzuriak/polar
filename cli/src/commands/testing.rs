use super::{
  Plan, TestCase, build::Written, build_into, compile_source, render,
  write_output, write_std,
};
use crate::{
  CliError, Ctx,
  files::{
    Input, Source, discover, normalize, runtime_specifier, write_atomic,
  },
  project::{self, Contexts, Target},
  runtime::LAUNCHER_JS,
};
use polar_compiler::shared::{
  diagnostic::Severity, modules::module_path, text::string_literal,
};
use serde::Deserialize;
use std::{
  fmt::Write as _,
  fs,
  io::{BufRead, BufReader, ErrorKind},
  path::{Path, PathBuf},
  process::{Command, ExitStatus, Stdio},
  time::Instant,
};

pub(crate) struct Options<'a> {
  pub filter: Option<&'a str>,
  pub timeout: u64,
  pub json: bool,
  pub bless: bool,
  pub projects: &'a [String],
  pub jobs: usize,
  pub args: &'a [String],
}

const MARK: char = '\u{1e}';
const MAIN: &str = "_test_main";

pub(crate) fn test(
  ctx: &mut Ctx<'_, '_>,
  path: Option<&Path>,
  options: &Options<'_>,
) -> Result<u8, CliError> {
  let cwd = ctx.io.cwd.clone();

  if let Some(members) =
    project::workspace_members(path.unwrap_or(Path::new("")), &cwd)?
  {
    return super::workspace::test(ctx, &members, options);
  }

  if !options.projects.is_empty() {
    return Err(CliError::Message(
      "`--project` picks a member of a workspace; this folder is not one"
        .to_string(),
    ));
  }

  one(ctx, path, options)
}

#[allow(clippy::too_many_lines)]
fn one(
  ctx: &mut Ctx<'_, '_>,
  path: Option<&Path>,
  options: &Options<'_>,
) -> Result<u8, CliError> {
  let cwd = ctx.io.cwd.clone();
  let Some(project) = project::load(path.unwrap_or(Path::new("")), &cwd)?
  else {
    return Err(CliError::Message(
      "`polar test` runs inside a project (a folder with polar.toml)"
        .to_string(),
    ));
  };

  if project.library {
    return Err(project::library(&project.name));
  }

  let node = super::run::needs_node(ctx)?;
  let root = normalize(&cwd.join(&project.dir));
  let out = root.join(crate::plugin::DIR).join("test").join("dist");

  match fs::remove_dir_all(&out) {
    Err(e) if e.kind() != ErrorKind::NotFound => {
      return Err(CliError::write(out.display().to_string(), &e));
    }
    _ => {}
  }

  let target = Target {
    paths: vec![project.src.clone()],
    out,
    project: Some(project.name.clone()),
    hosts: project.hosts.clone(),
    library: false,
    start: None,
    fmt_exclude: Vec::new(),
  };
  let host = (!project.hosts.is_empty()).then_some("Node");
  let browser = (project.hosts.iter().any(|h| h == "Browser")
    && project.hosts.iter().any(|h| h == "Node")
    && mentions_browser(&normalize(&cwd.join(&project.src))))
  .then(|| root.join(crate::plugin::DIR).join("test").join("browser"));
  let Some(written) = build_into(ctx, &[target], host, false, Some(&root))?
  else {
    return Ok(1);
  };
  let Some(built) = written.first() else {
    return Ok(1);
  };

  if let Some(dir) = &browser {
    match fs::remove_dir_all(dir) {
      Err(e) if e.kind() != ErrorKind::NotFound => {
        return Err(CliError::write(dir.display().to_string(), &e));
      }
      _ => {}
    }

    let target = Target {
      paths: vec![project.src.clone()],
      out: dir.clone(),
      project: Some(project.name.clone()),
      hosts: project.hosts.clone(),
      library: false,
      start: None,
      fmt_exclude: Vec::new(),
    };

    if build_into(ctx, &[target], Some("Browser"), false, None)?.is_none() {
      return Ok(1);
    }
  }

  let failing = failing_files(ctx, &project)?;
  let wanted =
    |name: &str| options.filter.is_none_or(|text| name.contains(text));
  let chosen: Vec<&Failing> =
    failing.iter().filter(|f| wanted(&f.name)).collect();

  if built.tests.is_empty() && failing.is_empty() && !options.json {
    let _ =
      writeln!(ctx.io.out, "no tests found (looked for *_test.px under src)");
  }

  let cases: Vec<Case> = built
    .tests
    .iter()
    .map(Case::from)
    .filter(|case| options.filter.is_none_or(|text| case.name.contains(text)))
    .collect();
  let mut report = Report {
    color: ctx.color && ctx.io.stdout_is_tty,
    timeout: options.timeout,
    filtered: built.tests.len() - cases.len() + failing.len() - chosen.len(),
    json: options.json,
    places: cases
      .iter()
      .map(|case| (case.name.clone(), (case.file.clone(), case.line)))
      .chain(chosen.iter().map(|f| (f.name.clone(), (f.display.clone(), 1))))
      .collect(),
    ran: Vec::new(),
    start: Instant::now(),
  };

  if misnamed(ctx, &cases) {
    return Ok(1);
  }

  let total = cases.len() + chosen.len();
  let main = if cases.is_empty() {
    None
  } else {
    let src = normalize(&cwd.join(&project.src));

    match compile_main(ctx, &src, built, &cases, options.timeout)? {
      Some(main) => Some(main),
      None => return Ok(1),
    }
  };

  Report::running(ctx, options.json, total);
  report.start = Instant::now();

  for failing in &chosen {
    let ran = expect_failure(ctx, failing, options.bless)?;

    report.ended(ctx, ran);
  }

  if let Some(main) = main {
    let entry = project
      .start()
      .map_or_else(|| PathBuf::from("main.js"), |start| start.main);

    run(
      ctx,
      &node,
      &main,
      options.args,
      &cases,
      &Env { browser: browser.as_deref(), main: &entry },
      &mut report,
    )?;
  }

  Ok(report.finish(ctx, total))
}

fn diagnostic_text(
  d: &polar_compiler::shared::diagnostic::Diagnostic,
) -> String {
  let mut text = d.message.clone();

  for label in std::iter::once(&d.primary).chain(&d.secondary) {
    if let Some(message) = &label.message {
      text.push('\n');
      text.push_str(message);
    }
  }

  text
}

struct Failing {
  name: String,
  display: String,
  input: Input,
}

fn failing_files(
  ctx: &Ctx<'_, '_>,
  project: &project::Project,
) -> Result<Vec<Failing>, CliError> {
  let inputs = discover(
    std::slice::from_ref(&project.src),
    &ctx.io.cwd,
    Some(&project.out),
  )?;
  let mut out = Vec::new();

  for input in inputs {
    let text = fs::read_to_string(&input.path)
      .map_err(|e| CliError::read(&input.display, &e))?;

    if !crate::expect::is_expected_failure(&text) {
      continue;
    }

    let parts: Vec<String> = input
      .output
      .components()
      .map(|c| c.as_os_str().to_string_lossy().into_owned())
      .collect();

    out.push(Failing {
      name: parts.join("::"),
      display: input.display.clone(),
      input,
    });
  }

  Ok(out)
}

fn expect_failure(
  ctx: &mut Ctx<'_, '_>,
  failing: &Failing,
  bless: bool,
) -> Result<Ran, CliError> {
  let text = fs::read_to_string(&failing.input.path)
    .map_err(|e| CliError::read(&failing.display, &e))?;
  let verdict = |outcome, message: Option<String>| Ran {
    name: failing.name.clone(),
    outcome,
    ms: None,
    message,
    at: Some(failing.display.clone()),
    output: String::new(),
  };
  let expected = match crate::expect::parse(&text) {
    Ok(expected) => expected,
    Err((line, message)) => {
      let mut ran = verdict(Outcome::Error, Some(message));

      ran.at = Some(format!("{}:{line}", failing.display));
      return Ok(ran);
    }
  };
  let started = Instant::now();
  let compiled = super::compile_one(
    ctx,
    &failing.input,
    "./_polar/runtime.js".to_string(),
    &Plan::checking(),
  )?;
  let found: Vec<(String, usize, String)> = compiled
    .diagnostics
    .iter()
    .filter(|d| d.severity == Severity::Error)
    .map(|d| {
      (
        d.code.to_string(),
        compiled.file.position_at(d.primary.span.start).line + 1,
        diagnostic_text(d),
      )
    })
    .collect();

  if bless {
    if found.is_empty() {
      let mut ran = verdict(
        Outcome::Fail,
        Some(
          "the file compiled without errors, so there is nothing to bless"
            .to_string(),
        ),
      );

      ran.ms = Some(started.elapsed().as_secs_f64() * 1000.0);
      return Ok(ran);
    }

    crate::expect::bless(&failing.input.path, &text, &found)
      .map_err(|e| CliError::write(&failing.display, &e))?;
  } else {
    let problems = crate::expect::mismatches(&expected, &found);

    if !problems.is_empty() {
      let mut ran = verdict(Outcome::Fail, Some(problems.join("\n")));

      ran.ms = Some(started.elapsed().as_secs_f64() * 1000.0);
      return Ok(ran);
    }
  }

  let mut ran = verdict(Outcome::Pass, None);

  ran.ms = Some(started.elapsed().as_secs_f64() * 1000.0);
  ran.at = None;
  Ok(ran)
}

fn mentions_browser(dir: &Path) -> bool {
  let Ok(entries) = fs::read_dir(dir) else { return false };

  entries.flatten().any(|entry| {
    let path = entry.path();

    if path.is_dir() {
      return mentions_browser(&path);
    }

    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();

    Path::new(name).extension().is_some_and(|ext| ext == "px")
      && !name.ends_with("_test.px")
      && fs::read_to_string(&path).is_ok_and(|text| text.contains("Browser"))
  })
}

struct Env<'a> {
  browser: Option<&'a Path>,
  main: &'a Path,
}

struct Case {
  name: String,
  file: String,
  line: usize,
  module: String,
  declared: Option<String>,
  function: String,
  each: bool,
}

impl From<&TestCase> for Case {
  fn from(test: &TestCase) -> Self {
    let parts: Vec<String> = test
      .output
      .components()
      .map(|c| c.as_os_str().to_string_lossy().into_owned())
      .collect();

    Self {
      name: format!("{}::{}", parts.join("::"), test.name),
      file: test.file.clone(),
      line: test.line,
      module: module_path(parts.iter().map(String::as_str)),
      declared: test.module.clone(),
      function: test.name.clone(),
      each: test.each,
    }
  }
}

fn misnamed(ctx: &mut Ctx<'_, '_>, cases: &[Case]) -> bool {
  let mut seen: Vec<&str> = Vec::new();

  for case in cases {
    let expected = case.module.rsplit('.').next().unwrap_or(&case.module);
    let Some(declared) = &case.declared else { continue };

    if declared != expected && !seen.contains(&case.file.as_str()) {
      seen.push(&case.file);
      let _ = writeln!(
        ctx.io.err,
        "error: `{}` declares `module {declared}`, but `polar test` imports it \
         as `{}`\n  = help: rename it `module {expected}`",
        case.file, case.module
      );
    }
  }

  !seen.is_empty()
}

fn generate(cases: &[Case], host: Option<&str>, timeout: u64) -> String {
  let mut modules: Vec<&str> = Vec::new();

  for case in cases {
    if !modules.contains(&case.module.as_str()) {
      modules.push(&case.module);
    }
  }

  let mut text =
    String::from("module TestMain\n\nuses\n  Std.List\n  Std.Test\n");

  for (i, module) in modules.iter().enumerate() {
    let _ = writeln!(text, "  {module} as Suite{i}");
  }

  if let Some(host) = host {
    let _ = write!(text, "\nhosts\n  {host}\n");
  }

  text.push_str("\nfunctions\n  main() {\n    Test.run_all(\n      [\n");

  for case in cases {
    let suite = modules.iter().position(|m| *m == case.module).unwrap_or(0);
    let run = if case.each {
      format!(
        "function() {{ Suite{suite}.around_each(Suite{suite}.{}) }}",
        case.function
      )
    } else {
      format!("Suite{suite}.{}", case.function)
    };

    let _ = writeln!(
      text,
      "        {{ file: {}, name: {}, line: {}, run: {run} }},",
      string_literal(&case.file),
      string_literal(&case.name),
      case.line,
    );
  }

  let _ = write!(
    text,
    "      ],\n      {timeout},\n    )\n  }}\n\nexports\n  main\n"
  );
  text
}

fn compile_main(
  ctx: &mut Ctx<'_, '_>,
  src: &Path,
  built: &Written,
  cases: &[Case],
  timeout: u64,
) -> Result<Option<PathBuf>, CliError> {
  let path = src.join(format!("{MAIN}.px"));
  let package = Contexts::default().of(&path, &ctx.io.cwd)?;
  let input = Input {
    display: format!("{MAIN}.px"),
    path,
    output: PathBuf::from(MAIN),
    package,
  };
  let source = Source {
    text: generate(cases, built.host.as_deref(), timeout),
    invalid: None,
  };
  let plan = Plan {
    host: polar_compiler::HostOption::Fixed(built.host.clone()),
    out_dir: Some(built.dir.clone()),
  };
  let runtime = runtime_specifier(&input.output);
  let compiled = compile_source(ctx.compiler, &input, source, runtime, &plan)?;

  if compiled.has_errors() || compiled.output.is_none() {
    render(ctx.io.err, ctx.color, &[(&compiled.file, &compiled.diagnostics)]);
    let _ = writeln!(
      ctx.io.err,
      "note: `polar test` generates `{MAIN}.px` itself, so this error is a \
       bug in polar test"
    );
    return Ok(None);
  }

  write_output(&built.dir, &compiled)?;
  write_std(ctx.compiler, &built.dir, super::build::std_imports(&compiled))?;

  let launcher = launcher(&built.dir);

  write_atomic(&launcher, LAUNCHER_JS.as_bytes())
    .map_err(|e| CliError::write(launcher.display().to_string(), &e))?;

  Ok(Some(built.dir.join(format!("{MAIN}.js"))))
}

fn launcher(dir: &Path) -> PathBuf {
  dir.join("_polar").join("launcher.mjs")
}

#[derive(Deserialize)]
#[serde(tag = "event", rename_all = "lowercase")]
enum Event {
  Start {
    name: String,
  },
  End {
    name: String,
    outcome: Outcome,
    ms: Option<f64>,
    message: Option<String>,
    at: Option<String>,
  },
}

#[derive(Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum Outcome {
  Pass,
  Fail,
  Error,
  Timeout,
}

struct Ran {
  name: String,
  outcome: Outcome,
  ms: Option<f64>,
  message: Option<String>,
  at: Option<String>,
  output: String,
}

fn run(
  ctx: &mut Ctx<'_, '_>,
  node: &Path,
  main: &Path,
  args: &[String],
  cases: &[Case],
  env: &Env<'_>,
  report: &mut Report,
) -> Result<(), CliError> {
  let dir = main.parent().unwrap_or(Path::new("."));

  crate::watch::on_ctrl_c(|| {});

  let mut command = Command::new(node);

  command.env("POLAR_TEST_MAIN", env.main);

  if let Some(browser) = env.browser {
    command.env("POLAR_TEST_BROWSER_DIST", browser);
  }

  let mut child = command
    .arg("--enable-source-maps")
    .args(args)
    .arg(launcher(dir))
    .arg(main)
    .arg(format!("{MAIN}.px"))
    .current_dir(&ctx.io.cwd)
    .stdout(Stdio::piped())
    .spawn()
    .map_err(|e| {
      CliError::Message(format!("cannot run {}: {e}", node.display()))
    })?;
  let Some(stdout) = child.stdout.take() else {
    return Err(CliError::Message("cannot read the test output".to_string()));
  };
  let mut current: Option<(String, String)> = None;

  for line in BufReader::new(stdout).lines() {
    let line = line.map_err(|e| {
      CliError::Message(format!("cannot read the test output: {e}"))
    })?;
    let event = line
      .strip_prefix(MARK)
      .and_then(|json| serde_json::from_str::<Event>(json).ok());

    match event {
      Some(Event::Start { name }) => {
        if let Some((name, output)) = current.take() {
          report.ended(ctx, unfinished(name, output, None));
        }

        current = Some((name, String::new()));
      }
      Some(Event::End { name, outcome, ms, message, at }) => {
        let output = current
          .take()
          .filter(|(started, _)| *started == name)
          .map(|(_, output)| output)
          .unwrap_or_default();

        report.ended(ctx, Ran { name, outcome, ms, message, at, output });
      }
      None => match &mut current {
        Some((_, output)) => {
          output.push_str(&line);
          output.push('\n');
        }
        None => {
          let _ = writeln!(ctx.io.out, "{line}");
        }
      },
    }
  }

  let status = child.wait().map_err(|e| {
    CliError::Message(format!("cannot run {}: {e}", node.display()))
  })?;

  if let Some((name, output)) = current.take() {
    report.ended(ctx, unfinished(name, output, Some(status)));
  }

  for case in cases {
    if !report.ran.iter().any(|ran| ran.name == case.name) {
      report
        .ended(ctx, unfinished(case.name.clone(), String::new(), Some(status)));
    }
  }

  Ok(())
}

fn unfinished(name: String, output: String, status: Option<ExitStatus>) -> Ran {
  let message = match status.and_then(|s| s.code()) {
    Some(code) => {
      format!(
        "the test process exited with code {code} before this test finished"
      )
    }
    None => "the test process stopped before this test finished".to_string(),
  };

  Ran {
    name,
    outcome: Outcome::Error,
    ms: None,
    message: Some(message),
    at: None,
    output,
  }
}

struct Report {
  color: bool,
  timeout: u64,
  filtered: usize,
  json: bool,
  places: std::collections::HashMap<String, (String, usize)>,
  ran: Vec<Ran>,
  start: Instant,
}

impl Report {
  fn paint(&self, code: u8, text: &str) -> String {
    if self.color {
      format!("\x1b[{code}m{text}\x1b[0m")
    } else {
      text.to_string()
    }
  }

  fn running(ctx: &mut Ctx<'_, '_>, json: bool, count: usize) {
    if json {
      return;
    }

    let tests = if count == 1 { "test" } else { "tests" };
    let _ = write!(ctx.io.out, "\nrunning {count} {tests}\n");
  }

  fn ended(&mut self, ctx: &mut Ctx<'_, '_>, ran: Ran) {
    if self.json {
      let (file, line) = self.places.get(&ran.name).cloned().unzip();
      let outcome = match ran.outcome {
        Outcome::Pass => "pass",
        Outcome::Fail => "fail",
        Outcome::Error => "error",
        Outcome::Timeout => "timeout",
      };
      let object = serde_json::json!({
        "event": "test",
        "name": ran.name,
        "file": file,
        "line": line,
        "outcome": outcome,
        "ms": ran.ms,
        "message": ran.message,
        "at": ran.at,
        "output": ran.output,
      });
      let _ = writeln!(ctx.io.out, "{object}");
      let _ = ctx.io.out.flush();

      self.ran.push(ran);
      return;
    }

    let label = match ran.outcome {
      Outcome::Pass => self.paint(32, "ok"),
      Outcome::Fail => self.paint(31, "FAILED"),
      Outcome::Error => self.paint(31, "ERROR"),
      Outcome::Timeout => self.paint(31, "TIMEOUT"),
    };
    let _ = writeln!(ctx.io.out, "test {} ... {label}", ran.name);
    let _ = ctx.io.out.flush();

    self.ran.push(ran);
  }

  fn detail(&self, ran: &Ran) -> String {
    let message = match ran.outcome {
      Outcome::Timeout => format!("timed out after {}ms", self.timeout),
      _ => ran.message.clone().unwrap_or_default(),
    };

    match &ran.at {
      Some(at) => format!("failed at {at}:\n{message}"),
      None => message,
    }
  }

  fn finish(&self, ctx: &mut Ctx<'_, '_>, count: usize) -> u8 {
    let failures: Vec<&Ran> =
      self.ran.iter().filter(|r| r.outcome != Outcome::Pass).collect();
    let passed = self.ran.len() - failures.len();
    let mut text = String::new();

    if !failures.is_empty() {
      text.push_str("\nfailures:\n");

      for ran in &failures {
        let _ = write!(
          text,
          "\n---- {} ----\n{}\n{}",
          ran.name,
          self.detail(ran),
          ran.output
        );
      }

      text.push_str("\nfailures:\n");

      for ran in &failures {
        let _ = writeln!(text, "    {}", ran.name);
      }
    }

    let ok = failures.is_empty() && self.ran.len() == count;

    if self.json {
      let object = serde_json::json!({
        "event": "result",
        "ok": ok,
        "passed": passed,
        "failed": failures.len(),
        "filtered": self.filtered,
        "seconds": self.start.elapsed().as_secs_f64(),
      });
      let _ = writeln!(ctx.io.out, "{object}");

      return u8::from(!ok);
    }

    let status =
      if ok { self.paint(32, "ok") } else { self.paint(31, "FAILED") };
    let _ = write!(
      text,
      "\ntest result: {status}. {passed} passed; {} failed; {} filtered out; \
       finished in {:.2}s\n\n",
      failures.len(),
      self.filtered,
      self.start.elapsed().as_secs_f64(),
    );
    let _ = write!(ctx.io.out, "{text}");

    u8::from(!ok)
  }
}
