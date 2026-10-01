use crate::{
  CliError, Compiler, Ctx,
  commands::{Compiled, compile_input, has_errors, render},
  files::{Input, discover, normalize, runtime_specifier},
  plural,
};
use notify_debouncer_full::{
  DebounceEventResult, new_debouncer,
  notify::{EventKind, RecursiveMode},
};
use polar_compiler::{
  shared::diagnostic::Diagnostic, shared::source::SourceFile,
};
use std::{
  collections::{BTreeMap, HashSet},
  io::Write,
  path::{Path, PathBuf},
  sync::{
    Mutex, OnceLock,
    mpsc::{self, Sender},
  },
  thread::JoinHandle,
  time::{Duration, Instant},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchMode {
  Build { out: PathBuf },
  Check { out: Option<PathBuf> },
}

impl WatchMode {
  fn out(&self) -> Option<&Path> {
    match self {
      Self::Build { out } => Some(out),
      Self::Check { out } => out.as_deref(),
    }
  }
}

pub struct WatchOptions {
  pub paths: Vec<PathBuf>,
  pub cwd: PathBuf,
  pub mode: WatchMode,
  pub debounce: Duration,
  pub compiler: Compiler,
  pub color: bool,
  pub clear_screen: bool,
  pub debug: bool,
  pub err: Box<dyn Write + Send>,
  pub on_cycle: Box<dyn FnMut(&CycleResult) + Send>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CycleResult {
  pub compiled: usize,
  pub errors: usize,
  pub written: Vec<PathBuf>,
  pub removed: Vec<PathBuf>,
}

enum Msg {
  Changed(Vec<PathBuf>),
  Stop,
}

pub struct Watch {
  tx: Sender<Msg>,
  thread: Option<JoinHandle<()>>,
  debouncer: Option<Box<dyn std::any::Any + Send>>,
}

impl Watch {
  /// Runs a first cycle, then starts watching `options.paths` for changes.
  ///
  /// # Errors
  ///
  /// Fails if the file watcher cannot be created or a path cannot be watched.
  pub fn start(options: WatchOptions) -> Result<Self, CliError> {
    let (tx, rx) = mpsc::channel::<Msg>();
    let events = tx.clone();
    let mut debouncer = new_debouncer(
      options.debounce,
      None,
      move |result: DebounceEventResult| {
        if let Ok(events_batch) = result {
          let paths = events_batch
            .into_iter()
            .filter(|e| !matches!(e.event.kind, EventKind::Access(_)))
            .flat_map(|e| e.event.paths)
            .collect();

          let _ = events.send(Msg::Changed(paths));
        }
      },
    )
    .map_err(|e| CliError::Message(format!("cannot watch for changes: {e}")))?;

    let deps = dependency_dirs(&options);

    for path in
      options.paths.iter().map(|p| options.cwd.join(p)).chain(deps.clone())
    {
      debouncer.watch(&path, RecursiveMode::Recursive).map_err(|e| {
        CliError::Message(format!("cannot watch {}: {e}", path.display()))
      })?;
    }

    let mut cycler = Cycler::new(options, deps);

    cycler.cycle(None);

    let thread = std::thread::spawn(move || {
      while let Ok(Msg::Changed(paths)) = rx.recv() {
        let relevant = cycler.relevant(&paths);
        let in_deps =
          relevant.iter().any(|p| cycler.deps.iter().any(|d| p.starts_with(d)));

        if in_deps {
          cycler.cycle(None);
        } else if !relevant.is_empty() {
          cycler.cycle(Some(&relevant));
        }
      }
    });

    Ok(Self { tx, thread: Some(thread), debouncer: Some(Box::new(debouncer)) })
  }

  pub fn stop(mut self) {
    self.shutdown();
  }

  fn shutdown(&mut self) {
    self.debouncer.take();
    let _ = self.tx.send(Msg::Stop);

    if let Some(thread) = self.thread.take() {
      let _ = thread.join();
    }
  }
}

impl Drop for Watch {
  fn drop(&mut self) {
    self.shutdown();
  }
}

struct Known {
  file: SourceFile,
  diagnostics: Vec<Diagnostic>,
  output: PathBuf,
}

fn dependency_dirs(options: &WatchOptions) -> Vec<PathBuf> {
  let out = options.mode.out().map(|out| normalize(&options.cwd.join(out)));
  let inputs =
    discover(&options.paths, &options.cwd, out.as_deref()).unwrap_or_default();
  let mut dirs: Vec<PathBuf> = Vec::new();

  for input in &inputs {
    for dir in input.package.iter().flat_map(|p| p.dirs()) {
      if dir.is_dir() && !dirs.contains(&dir) {
        dirs.push(dir);
      }
    }
  }

  dirs
}

struct Cycler {
  options: WatchOptions,
  out: Option<PathBuf>,
  known: BTreeMap<PathBuf, Known>,
  deps: Vec<PathBuf>,
}

impl Cycler {
  fn new(options: WatchOptions, deps: Vec<PathBuf>) -> Self {
    let out = options.mode.out().map(|out| normalize(&options.cwd.join(out)));

    Self { options, out, known: BTreeMap::new(), deps }
  }

  fn relevant(&self, paths: &[PathBuf]) -> HashSet<PathBuf> {
    let roots: Vec<PathBuf> = self
      .options
      .paths
      .iter()
      .map(|p| normalize(&self.options.cwd.join(p)))
      .chain(self.deps.iter().cloned())
      .collect();

    let skipped = |p: &Path| {
      let below =
        roots.iter().find_map(|root| p.strip_prefix(root).ok()).unwrap_or(p);

      below.components().any(|c| {
        let name = c.as_os_str().to_string_lossy();

        matches!(name.as_ref(), "node_modules" | "snapshots" | "target")
          || (name.starts_with('.') && name != "." && name != "..")
      })
    };

    paths
      .iter()
      .map(|p| normalize(p))
      .filter(|p| !self.out.as_ref().is_some_and(|out| p.starts_with(out)))
      .filter(|p| !skipped(p))
      .filter(|p| p.extension().is_some_and(|e| e == "px") || p.is_dir())
      .collect()
  }

  fn cycle(&mut self, changed: Option<&HashSet<PathBuf>>) {
    let start = Instant::now();
    let mut result = CycleResult::default();
    let mut report = Vec::new();

    match discover(&self.options.paths, &self.options.cwd, self.out.as_deref())
    {
      Ok(inputs) => self.update(&inputs, changed, &mut result, &mut report),
      Err(e) => {
        e.report(&mut report, self.options.debug);
        result.errors += 1;
      }
    }

    let err = &mut self.options.err;

    if self.options.clear_screen {
      let _ = write!(err, "\x1b[2J\x1b[H");
    }

    let files: Vec<_> = self
      .known
      .values()
      .map(|k| (&k.file, k.diagnostics.as_slice()))
      .collect();

    render(err, self.options.color, &files);
    let _ = err.write_all(&report);

    result.errors += self
      .known
      .values()
      .flat_map(|k| &k.diagnostics)
      .filter(|d| {
        d.severity == polar_compiler::shared::diagnostic::Severity::Error
      })
      .count();

    let errors = match result.errors {
      0 => "no errors".to_string(),
      n => plural(n, "error"),
    };
    let now = jiff::Zoned::now().strftime("%H:%M:%S").to_string();

    let _ = writeln!(
      err,
      "[{now}] {} in {}ms — {errors}",
      plural(result.compiled, "file"),
      start.elapsed().as_millis()
    );
    let _ = err.flush();

    (self.options.on_cycle)(&result);
  }

  fn update(
    &mut self,
    inputs: &[Input],
    changed: Option<&HashSet<PathBuf>>,
    result: &mut CycleResult,
    report: &mut Vec<u8>,
  ) {
    let present: HashSet<&Path> =
      inputs.iter().map(|i| i.path.as_path()).collect();

    let gone: Vec<PathBuf> = self
      .known
      .keys()
      .filter(|p| !present.contains(p.as_path()))
      .cloned()
      .collect();

    for path in gone {
      if let (Some(known), Some(out)) = (self.known.remove(&path), &self.out) {
        if matches!(self.options.mode, WatchMode::Build { .. }) {
          for ext in ["js", "js.map"] {
            let output = out.join(known.output.with_extension(ext));

            if std::fs::remove_file(&output).is_ok() {
              result.removed.push(output);
            }
          }
        }
      }
    }

    let wanted = |input: &Input| match changed {
      None => true,
      Some(changed) => {
        !self.known.contains_key(&input.path)
          || changed.iter().any(|c| input.path.starts_with(c))
      }
    };
    let todo: Vec<Input> =
      inputs.iter().filter(|i| wanted(i)).cloned().collect();
    let mut deps_written: Vec<PathBuf> = Vec::new();

    for input in todo {
      result.compiled += 1;

      let compiled = match compile_input(
        self.options.compiler,
        &input,
        runtime_specifier(&input.output),
        &crate::commands::Plan {
          out_dir: match self.options.mode {
            WatchMode::Build { .. } => self.out.clone(),
            WatchMode::Check { .. } => None,
          },
          ..crate::commands::Plan::default()
        },
      ) {
        Ok(compiled) => compiled,
        Err(e) => {
          e.report(report, self.options.debug);
          result.errors += 1;
          self.known.remove(&input.path);
          continue;
        }
      };

      if let (WatchMode::Build { .. }, Some(out)) =
        (&self.options.mode, &self.out)
      {
        if !has_errors(&compiled.diagnostics) {
          self.write(out, &compiled, result, report);
          self.write_deps(out, &compiled, &mut deps_written, result, report);
        }
      }

      let Compiled { file, diagnostics, .. } = compiled;

      self.known.insert(
        input.path.clone(),
        Known { file, diagnostics, output: input.output },
      );
    }
  }

  fn write_deps(
    &self,
    out: &Path,
    compiled: &Compiled,
    done: &mut Vec<PathBuf>,
    result: &mut CycleResult,
    report: &mut Vec<u8>,
  ) {
    let plan = crate::commands::Plan {
      out_dir: Some(out.to_path_buf()),
      ..crate::commands::Plan::default()
    };

    for import in &compiled.imports {
      if !import.output.starts_with("_deps") || done.contains(&import.path) {
        continue;
      }

      done.push(import.path.clone());

      match compile_input(
        self.options.compiler,
        import,
        runtime_specifier(&import.output),
        &plan,
      ) {
        Ok(dep) if !has_errors(&dep.diagnostics) => {
          self.write(out, &dep, result, report);
        }
        Ok(_) => {}
        Err(e) => {
          e.report(report, self.options.debug);
          result.errors += 1;
        }
      }
    }
  }

  fn write(
    &self,
    out: &Path,
    compiled: &Compiled,
    result: &mut CycleResult,
    report: &mut Vec<u8>,
  ) {
    let std_imports = compiled
      .output
      .as_ref()
      .map(|o| o.std_imports.clone())
      .unwrap_or_default();
    let written = crate::commands::write_runtime(out)
      .and_then(|()| {
        crate::commands::write_std(self.options.compiler, out, std_imports)
      })
      .and_then(|std| Ok((std, crate::commands::write_output(out, compiled)?)))
      .and_then(|(std, paths)| {
        Ok((std, paths, crate::commands::write_externs(out, compiled)?))
      });

    match written {
      Ok((std, paths, externs)) => {
        result.written.extend(std);
        result.written.extend(paths);
        result.written.extend(externs);
      }
      Err(e) => {
        e.report(report, self.options.debug);
        result.errors += 1;
      }
    }
  }
}

struct Forward(Sender<Main>);

enum Main {
  Output(Vec<u8>),
  Stop,
}

impl Write for Forward {
  fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
    let _ = self.0.send(Main::Output(buf.to_vec()));
    Ok(buf.len())
  }

  fn flush(&mut self) -> std::io::Result<()> {
    Ok(())
  }
}

pub(crate) fn foreground(
  ctx: &mut Ctx<'_, '_>,
  paths: &[PathBuf],
  mode: WatchMode,
) -> Result<u8, CliError> {
  let (tx, rx) = mpsc::channel::<Main>();
  let stop = tx.clone();

  on_ctrl_c(move || {
    let _ = stop.send(Main::Stop);
  });

  let watch = Watch::start(WatchOptions {
    paths: paths.to_vec(),
    cwd: ctx.io.cwd.clone(),
    mode,
    debounce: Duration::from_millis(50),
    compiler: ctx.compiler,
    color: ctx.color,
    clear_screen: ctx.io.stderr_is_tty,
    debug: ctx.debug,
    err: Box::new(Forward(tx)),
    on_cycle: Box::new(|_| {}),
  })?;

  while let Ok(Main::Output(bytes)) = rx.recv() {
    let _ = ctx.io.err.write_all(&bytes);
    let _ = ctx.io.err.flush();
  }

  watch.stop();

  while let Ok(Main::Output(bytes)) = rx.try_recv() {
    let _ = ctx.io.err.write_all(&bytes);
  }

  Ok(0)
}

type Handler = Box<dyn Fn() + Send>;

pub(crate) fn on_ctrl_c(f: impl Fn() + Send + 'static) {
  static HANDLER: OnceLock<Mutex<Option<Handler>>> = OnceLock::new();

  let slot = HANDLER.get_or_init(|| {
    let _ = ctrlc::set_handler(|| {
      let handler = HANDLER.get().and_then(|slot| slot.lock().ok());

      if let Some(handler) = handler.as_ref().and_then(|h| h.as_ref()) {
        handler();
      }
    });

    Mutex::new(None)
  });

  if let Ok(mut handler) = slot.lock() {
    *handler = Some(Box::new(f));
  }
}
