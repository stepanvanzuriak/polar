use super::{
  Compiled, Plan, compile_imports, compile_source, has_errors, read_imports,
  render, run::needs_node,
};
use crate::{
  CliError, Ctx,
  files::{Input, Source, runtime_specifier},
  guard,
  project::{self, Package},
  runtime::{REPL_RUNNER_JS, RUNTIME_JS},
};
use polar_compiler::{
  CompileOptions, HostOption, Stage, shared::diagnostic::Diagnostic,
};
use std::{
  collections::HashSet,
  fmt::Write as _,
  fs,
  io::{BufRead, BufReader, IsTerminal, Write},
  path::{Path, PathBuf},
  process::{Child, ChildStdin, ChildStdout, Command, Stdio},
  sync::Arc,
};
use walkdir::WalkDir;

const MARK: &str = "\u{1}polar-repl ";
const DEFAULT_STD: [&str; 8] =
  ["Json", "List", "Map", "Math", "Option", "Ref", "Result", "Table"];
const HELP: &str = "\
:type <expr>   show the type of an expression without evaluating it
:reload        recompile the project and keep the bindings that still check
:help          show this help
:quit          leave the REPL (Ctrl-D works too)
let x = <expr> bind a name for the lines that follow";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Printer {
  Show,
  Json,
  Nothing,
}

struct Binding {
  name: String,
  line: String,
}

struct Session {
  node: PathBuf,
  dir: tempfile::TempDir,
  src: PathBuf,
  package: Option<Arc<Package>>,
  uses: Vec<String>,
  modules: Vec<String>,
  bindings: Vec<Binding>,
  count: usize,
  written: HashSet<PathBuf>,
  std_done: Vec<String>,
  child: Option<Runner>,
}

struct Runner {
  process: Child,
  stdin: ChildStdin,
  stdout: BufReader<ChildStdout>,
}

impl Drop for Runner {
  fn drop(&mut self) {
    let _ = self.process.kill();
    let _ = self.process.wait();
  }
}

enum Outcome {
  Done(String),
  Failed,
}

pub(crate) fn repl(
  ctx: &mut Ctx<'_, '_>,
  path: Option<&Path>,
  setup: Option<&str>,
  expr: Option<&str>,
) -> Result<u8, CliError> {
  let cwd = ctx.io.cwd.clone();
  let project = project::load(path.unwrap_or(Path::new("")), &cwd)?;

  if let Some(project) = &project {
    if project.library {
      return Err(project::library(&project.name));
    }
  }

  let setup = setup.map(parse_setup).transpose()?;
  let node = needs_node(ctx)?;
  let dir = tempfile::Builder::new()
    .prefix("polar-repl-")
    .tempdir()
    .map_err(|e| CliError::write("a temporary directory", &e))?;
  let (src, package) = match &project {
    Some(p) => (p.src.clone(), Some(p.package.clone())),
    None => (cwd.join(path.unwrap_or(Path::new(""))), None),
  };
  let mut session = Session {
    node,
    dir,
    src,
    package,
    uses: Vec::new(),
    modules: Vec::new(),
    bindings: Vec::new(),
    count: 0,
    written: HashSet::new(),
    std_done: Vec::new(),
    child: None,
  };

  session.scan();
  session.start(ctx)?;

  if let Some((module, function)) = &setup {
    let text = session
      .module_text(&format!("  setup() {{\n    {module}.{function}()\n  }}\n"));

    match session.run(ctx, &text, "setup", true)? {
      Outcome::Done(_) => {}
      Outcome::Failed => {
        let _ =
          writeln!(ctx.io.err, "error: `--setup {module}.{function}` failed");
        return Ok(1);
      }
    }
  }

  let mut failed = false;

  if let Some(expr) = expr {
    for line in expr.lines() {
      failed |= session.line(ctx, line)? == Flow::Error;
    }

    return Ok(u8::from(failed));
  }

  let interactive = std::io::stdin().is_terminal();
  let stdin = std::io::stdin();
  let mut lines = stdin.lock().lines();

  loop {
    if interactive {
      let _ = write!(ctx.io.out, "polar> ");
      let _ = ctx.io.out.flush();
    }

    let Some(Ok(line)) = lines.next() else { break };

    match session.line(ctx, &line)? {
      Flow::Quit => break,
      Flow::Error => failed = true,
      Flow::Ok => {}
    }
  }

  if interactive {
    let _ = writeln!(ctx.io.out);
  }

  Ok(u8::from(failed && !interactive))
}

#[derive(PartialEq, Eq)]
enum Flow {
  Ok,
  Error,
  Quit,
}

fn parse_setup(text: &str) -> Result<(String, String), CliError> {
  match text.rsplit_once('.') {
    Some((module, function)) if !module.is_empty() && !function.is_empty() => {
      Ok((module.to_string(), function.to_string()))
    }
    _ => Err(CliError::Message(format!(
      "`--setup {text}` should look like `Module.function`"
    ))),
  }
}

impl Session {
  fn scan(&mut self) {
    self.uses.clear();
    self.modules.clear();

    let mut seen: HashSet<String> = HashSet::new();
    let mut entries: Vec<String> = Vec::new();
    let mut files: Vec<PathBuf> = WalkDir::new(&self.src)
      .max_depth(if self.package.is_some() { usize::MAX } else { 0 })
      .sort_by_file_name()
      .into_iter()
      .filter_entry(|e| {
        e.depth() == 0
          || !e.file_name().to_string_lossy().starts_with(['.', '_'])
            && !matches!(
              e.file_name().to_string_lossy().as_ref(),
              "node_modules" | "target" | "snapshots"
            )
      })
      .filter_map(Result::ok)
      .filter(|e| e.file_type().is_file())
      .map(walkdir::DirEntry::into_path)
      .filter(|p| {
        p.extension().is_some_and(|e| e == "px")
          && !p.to_string_lossy().ends_with("_test.px")
      })
      .collect();

    files.sort();

    for file in &files {
      let Ok(text) = fs::read_to_string(file) else { continue };
      let mut in_uses = false;

      for line in text.lines() {
        if let Some(name) = line.strip_prefix("module ") {
          let name = name.trim().to_string();

          if name != "Repl" && !self.modules.contains(&name) {
            self.modules.push(name.clone());
            entries.push(name);
          }

          continue;
        }

        if line.trim_end() == "uses" {
          in_uses = true;
        } else if !line.trim().is_empty()
          && !line.starts_with(char::is_whitespace)
        {
          in_uses = false;
        } else if in_uses && !line.trim().is_empty() {
          entries.push(line.trim().to_string());
        }
      }
    }

    for std in DEFAULT_STD {
      entries.push(format!("Std.{std}"));
    }

    seen.insert("Prelude".to_string());

    for entry in entries {
      if entry.starts_with("//") {
        continue;
      }

      if seen.insert(bound_name(&entry)) {
        self.uses.push(entry);
      }
    }
  }

  fn module_text(&self, functions: &str) -> String {
    let mut text = String::from("module Repl\n\nuses\n");

    for entry in &self.uses {
      let _ = writeln!(text, "  {entry}");
    }

    text.push_str("  Std.Prelude { show }\n\nhosts\n  Node\n\nfunctions\n");
    text.push_str(functions);
    text.push_str("\nexports\n  Node\n");

    for name in ["setup", "value", "shown"] {
      if functions.contains(&format!("  {name}() {{")) {
        let _ = writeln!(text, "  {name}");
      }
    }

    text
  }

  fn functions(lets: &[String], last: &str, printer: Printer) -> String {
    let mut text = String::from("  value() {\n");

    for line in lets {
      let _ = writeln!(text, "    {line}");
    }

    let _ = write!(text, "    {{ v: {last} }}\n  }}\n");

    match printer {
      Printer::Show => {
        text.push_str("\n  shown() {\n    show(value().v)\n  }\n");
      }
      Printer::Json => {
        text.push_str("\n  shown() {\n    Json.encode(value().v)\n  }\n");
      }
      Printer::Nothing => {}
    }

    text
  }

  fn start(&mut self, ctx: &mut Ctx<'_, '_>) -> Result<(), CliError> {
    self.child = None;

    let runtime = self.dir.path().join("_polar").join("runtime.js");
    let runner = self.dir.path().join("repl_runner.mjs");

    for (path, text) in [(&runtime, RUNTIME_JS), (&runner, REPL_RUNNER_JS)] {
      path
        .parent()
        .map_or(Ok(()), fs::create_dir_all)
        .and_then(|()| fs::write(path, text))
        .map_err(|e| CliError::write(path.display().to_string(), &e))?;
    }

    let mut process = Command::new(&self.node)
      .arg("--enable-source-maps")
      .arg(&runner)
      .arg(self.dir.path())
      .current_dir(&ctx.io.cwd)
      .stdin(Stdio::piped())
      .stdout(Stdio::piped())
      .stderr(Stdio::inherit())
      .spawn()
      .map_err(|e| {
        CliError::Message(format!("cannot run {}: {e}", self.node.display()))
      })?;
    let stdin = process.stdin.take().expect("piped stdin");
    let stdout = BufReader::new(process.stdout.take().expect("piped stdout"));

    self.child = Some(Runner { process, stdin, stdout });

    Ok(())
  }

  fn input(&self) -> Input {
    Input {
      display: "repl".to_string(),
      path: self.src.join("repl.px"),
      output: PathBuf::from(format!("repl_{}", self.count)),
      package: self.package.clone(),
    }
  }

  fn check(
    &self,
    ctx: &mut Ctx<'_, '_>,
    text: &str,
  ) -> Result<(Option<String>, Vec<Diagnostic>), CliError> {
    let input = self.input();
    let (modules, _) = read_imports(&input, text)?;
    let options = CompileOptions {
      modules,
      host: Plan::checking().host,
      plugins: input.plugins(),
      ..CompileOptions::default()
    };
    let dump = ctx.compiler.dump_stage;
    let result = guard(&input.display, || {
      dump(text, &input.display, Stage::Types, &options)
    })
    .map_err(CliError::Ice)?;

    Ok((result.output, result.diagnostics))
  }

  fn show_errors(
    ctx: &mut Ctx<'_, '_>,
    text: &str,
    diagnostics: &[Diagnostic],
  ) {
    let errors: Vec<Diagnostic> = diagnostics
      .iter()
      .filter(|d| {
        d.severity == polar_compiler::shared::diagnostic::Severity::Error
      })
      .cloned()
      .collect();
    let file = polar_compiler::shared::source::SourceFile::new("repl", text);

    render(ctx.io.err, ctx.color, &[(&file, &errors)]);
  }

  fn type_of(
    &self,
    ctx: &mut Ctx<'_, '_>,
    lets: &[String],
    last: &str,
  ) -> Result<Option<String>, CliError> {
    let text = self.module_text(&Self::functions(lets, last, Printer::Nothing));
    let (output, diagnostics) = self.check(ctx, &text)?;

    if has_errors(&diagnostics) || output.is_none() {
      Self::show_errors(ctx, &text, &diagnostics);
      return Ok(None);
    }

    Ok(output.as_deref().and_then(value_type))
  }

  fn run(
    &mut self,
    ctx: &mut Ctx<'_, '_>,
    text: &str,
    call: &str,
    quiet_errors: bool,
  ) -> Result<Outcome, CliError> {
    let Some(file) = self.build(ctx, text)? else { return Ok(Outcome::Failed) };
    let _ = quiet_errors;

    self.request(ctx, &file, call)
  }

  fn build(
    &mut self,
    ctx: &mut Ctx<'_, '_>,
    text: &str,
  ) -> Result<Option<String>, CliError> {
    self.count += 1;

    let input = self.input();
    let plan =
      Plan { host: HostOption::Fixed(Some("Node".to_string())), out_dir: None };
    let root = compile_source(
      ctx.compiler,
      &input,
      Source { text: text.to_string(), invalid: None },
      "./_polar/runtime.js".to_string(),
      &plan,
    )?;
    let mut compiled: Vec<Compiled> = vec![root];

    compile_imports(
      ctx.compiler,
      &mut compiled,
      |i| runtime_specifier(&i.output),
      &plan,
    )?;

    let mut broken = false;

    for c in &compiled {
      let errors: Vec<Diagnostic> = c
        .diagnostics
        .iter()
        .filter(|d| {
          d.severity == polar_compiler::shared::diagnostic::Severity::Error
        })
        .cloned()
        .collect();

      if !errors.is_empty() {
        broken = true;
        render(ctx.io.err, ctx.color, &[(&c.file, &errors)]);
      }
    }

    if broken || compiled.iter().any(|c| c.output.is_none()) {
      return Ok(None);
    }

    let mut std_imports = Vec::new();

    for c in &compiled {
      let Some(output) = &c.output else { continue };
      let js = self.dir.path().join(c.input.output.with_extension("js"));

      if c.input.output == input.output || self.written.insert(js.clone()) {
        js.parent()
          .map_or(Ok(()), fs::create_dir_all)
          .and_then(|()| fs::write(&js, &output.js))
          .map_err(|e| CliError::write(js.display().to_string(), &e))?;
      }

      std_imports.extend(
        output
          .std_imports
          .iter()
          .filter(|n| !self.std_done.contains(n))
          .cloned(),
      );
    }

    self.std_done.extend(std_imports.iter().cloned());
    super::write_std(ctx.compiler, self.dir.path(), std_imports)?;

    Ok(Some(format!("repl_{}.js", self.count)))
  }

  fn request(
    &mut self,
    ctx: &mut Ctx<'_, '_>,
    file: &str,
    call: &str,
  ) -> Result<Outcome, CliError> {
    let Some(runner) = self.child.as_mut() else {
      return Err(CliError::Message("the REPL's Node process is gone".into()));
    };
    let request = serde_json::json!({ "file": file, "call": call });
    let gone = || CliError::Message("the REPL's Node process exited".into());

    writeln!(runner.stdin, "{request}").map_err(|_| gone())?;
    runner.stdin.flush().map_err(|_| gone())?;

    loop {
      let mut line = String::new();

      if runner.stdout.read_line(&mut line).map_err(|_| gone())? == 0 {
        return Err(gone());
      }

      let Some(json) = line.strip_prefix(MARK) else {
        let _ = ctx.io.out.write_all(line.as_bytes());
        continue;
      };
      let reply: serde_json::Value =
        serde_json::from_str(json.trim_end()).unwrap_or_default();

      if reply["ok"].as_bool() == Some(true) {
        return Ok(Outcome::Done(
          reply["value"].as_str().unwrap_or_default().to_string(),
        ));
      }

      let _ = writeln!(
        ctx.io.err,
        "error: {}",
        reply["error"].as_str().unwrap_or("the line failed")
      );

      return Ok(Outcome::Failed);
    }
  }

  fn line(
    &mut self,
    ctx: &mut Ctx<'_, '_>,
    line: &str,
  ) -> Result<Flow, CliError> {
    let line = line.trim();

    if line.is_empty() {
      return Ok(Flow::Ok);
    }

    if let Some(meta) = line.strip_prefix(':') {
      return self.meta(ctx, meta.trim());
    }

    let lets: Vec<String> =
      self.bindings.iter().map(|b| b.line.clone()).collect();
    let (name, last) = match binding_name(line) {
      Some(name) => (Some(name.to_string()), name.to_string()),
      None if line.starts_with("let ") => {
        let _ = writeln!(
          ctx.io.err,
          "error: the REPL binds a plain name: `let name = expression`"
        );
        return Ok(Flow::Error);
      }
      None => (None, line.to_string()),
    };
    let mut all = lets.clone();

    if name.is_some() {
      all.push(line.to_string());
    }

    let Some(shown_type) = self.type_of(ctx, &all, &last)? else {
      return Ok(Flow::Error);
    };

    let mut printer = Printer::Show;
    let mut file = None;

    for attempt in [Printer::Show, Printer::Json, Printer::Nothing] {
      printer = attempt;

      let text = self.module_text(&Self::functions(&all, &last, attempt));

      if attempt == Printer::Nothing {
        file = self.build(ctx, &text)?;
        break;
      }

      let (_, diagnostics) = self.check(ctx, &text)?;
      let only_printing = diagnostics
        .iter()
        .filter(|d| {
          d.severity == polar_compiler::shared::diagnostic::Severity::Error
        })
        .all(|d| {
          d.message.contains("`Show`")
            || d.message.contains("`Json`")
            || d.message.contains("Json.encode")
        });

      if has_errors(&diagnostics) && only_printing {
        continue;
      }

      file = self.build(ctx, &text)?;
      break;
    }

    let Some(file) = file else { return Ok(Flow::Error) };
    let call = if printer == Printer::Nothing { "value" } else { "shown" };

    match self.request(ctx, &file, call)? {
      Outcome::Failed => Ok(Flow::Error),
      Outcome::Done(value) => {
        let value = if shown_type == "{}" {
          "{}".to_string()
        } else if printer == Printer::Nothing {
          format!("<no Show for {shown_type}>")
        } else if shown_type == "String" {
          serde_json::Value::String(value).to_string()
        } else {
          value
        };

        match name {
          Some(name) => {
            let _ = writeln!(ctx.io.out, "{name}: {shown_type} = {value}");

            self.bindings.retain(|b| b.name != name);
            self.bindings.push(Binding { name, line: line.to_string() });
          }
          None => {
            let _ = writeln!(ctx.io.out, "{shown_type} = {value}");
          }
        }

        Ok(Flow::Ok)
      }
    }
  }

  fn meta(
    &mut self,
    ctx: &mut Ctx<'_, '_>,
    meta: &str,
  ) -> Result<Flow, CliError> {
    let (command, rest) = meta.split_once(' ').unwrap_or((meta, ""));

    match command {
      "quit" | "q" => Ok(Flow::Quit),
      "help" | "h" => {
        let _ = writeln!(ctx.io.out, "{HELP}");
        Ok(Flow::Ok)
      }
      "type" | "t" => {
        let lets: Vec<String> =
          self.bindings.iter().map(|b| b.line.clone()).collect();

        if rest.trim().is_empty() {
          let _ = writeln!(ctx.io.err, "error: `:type` needs an expression");
          return Ok(Flow::Error);
        }

        match self.type_of(ctx, &lets, rest.trim())? {
          Some(shown) => {
            let _ = writeln!(ctx.io.out, "{shown}");
            Ok(Flow::Ok)
          }
          None => Ok(Flow::Error),
        }
      }
      "reload" | "r" => {
        self.scan();
        self.written.clear();
        self.std_done.clear();
        self.start(ctx)?;

        let old = std::mem::take(&mut self.bindings);
        let mut kept: Vec<Binding> = Vec::new();

        for binding in old {
          let mut lets: Vec<String> =
            kept.iter().map(|b| b.line.clone()).collect();

          lets.push(binding.line.clone());

          let text = self.module_text(&Self::functions(
            &lets,
            &binding.name,
            Printer::Nothing,
          ));
          let (_, diagnostics) = self.check(ctx, &text)?;

          if has_errors(&diagnostics) {
            let _ = writeln!(
              ctx.io.err,
              "note: dropped `{}`, which no longer type checks",
              binding.name
            );
          } else {
            kept.retain(|b| b.name != binding.name);
            kept.push(binding);
          }
        }

        self.bindings = kept;
        let _ = writeln!(ctx.io.out, "reloaded");

        Ok(Flow::Ok)
      }
      other => {
        let _ = writeln!(
          ctx.io.err,
          "error: unknown command `:{other}` (try `:help`)"
        );
        Ok(Flow::Error)
      }
    }
  }
}

fn binding_name(line: &str) -> Option<&str> {
  let rest = line.strip_prefix("let ")?.trim_start();
  let end = rest
    .find(|c: char| !(c.is_alphanumeric() || c == '_'))
    .unwrap_or(rest.len());
  let name = &rest[..end];
  let after = rest[end..].trim_start();

  (!name.is_empty()
    && !name.starts_with(|c: char| c.is_ascii_digit())
    && (after.starts_with('=') || after.starts_with(':')))
  .then_some(name)
}

fn bound_name(entry: &str) -> String {
  if let Some((_, alias)) = entry.split_once(" as ") {
    return alias.split_whitespace().next().unwrap_or_default().to_string();
  }

  let path = entry.split(['{', ' ']).next().unwrap_or_default();

  path.rsplit('.').next().unwrap_or_default().to_string()
}

fn value_type(dump: &str) -> Option<String> {
  let line = dump.lines().find(|l| l.starts_with("value : "))?;
  let rest = line.strip_prefix("value : function() -> { v: ")?;
  let mut depth = 0usize;

  for (i, c) in rest.char_indices() {
    match c {
      '{' => depth += 1,
      '}' if depth == 0 => return Some(rest[..i].trim_end().to_string()),
      '}' => depth -= 1,
      _ => {}
    }
  }

  None
}
