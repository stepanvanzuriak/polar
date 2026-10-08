use super::run::{
  Prepared, keep_if_debug, needs_node, prepare_file, prepare_launch,
};
use crate::{
  CliError, Ctx,
  files::{discover, normalize},
  project::{self, RunTarget},
};
use notify_debouncer_full::{
  DebounceEventResult, new_debouncer,
  notify::{EventKind, RecursiveMode},
};
use std::{
  path::{Path, PathBuf},
  process::{Child, Command},
  sync::mpsc,
  time::{Duration, Instant},
};

const POLL: Duration = Duration::from_millis(200);
const GRACE: Duration = Duration::from_secs(5);

enum Event {
  Changed(Vec<PathBuf>),
  Stop,
}

struct Running {
  child: Child,
  prepared: Prepared,
}

#[derive(Default)]
struct Inputs {
  trees: Vec<PathBuf>,
  shallow: Vec<PathBuf>,
  sources: Vec<PathBuf>,
  files: Vec<PathBuf>,
  out: Option<PathBuf>,
}

pub(crate) fn watch(
  ctx: &mut Ctx<'_, '_>,
  file: Option<&Path>,
  host: Option<&str>,
  args: &[String],
  debounce: Duration,
) -> Result<u8, CliError> {
  let node = needs_node(ctx)?;
  let target = project::run_target(file, &ctx.io.cwd)?;
  let inputs = inputs_of(ctx, &target)?;
  let (tx, rx) = mpsc::channel::<Event>();
  let stop = tx.clone();

  crate::watch::on_ctrl_c(move || {
    let _ = stop.send(Event::Stop);
  });

  let mut debouncer =
    new_debouncer(debounce, None, move |result: DebounceEventResult| {
      if let Ok(batch) = result {
        let paths = batch
          .into_iter()
          .filter(|e| !matches!(e.event.kind, EventKind::Access(_)))
          .flat_map(|e| e.event.paths)
          .collect();

        let _ = tx.send(Event::Changed(paths));
      }
    })
    .map_err(|e| CliError::Message(format!("cannot watch for changes: {e}")))?;

  for (path, mode) in inputs
    .trees
    .iter()
    .map(|p| (p, RecursiveMode::Recursive))
    .chain(inputs.shallow.iter().map(|p| (p, RecursiveMode::NonRecursive)))
  {
    debouncer.watch(path, mode).map_err(|e| {
      CliError::Message(format!("cannot watch {}: {e}", path.display()))
    })?;
  }

  let mut running: Option<Running> = None;

  rebuild(ctx, &node, file, host, args, &mut running, true)?;

  loop {
    match rx.recv_timeout(POLL) {
      Ok(Event::Stop) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
      Ok(Event::Changed(paths)) => {
        let relevant = inputs.relevant(&paths, &ctx.io.cwd);

        if relevant.is_empty() {
          continue;
        }

        for path in &relevant {
          let _ = writeln!(ctx.io.err, "change: {}", shown(path, &ctx.io.cwd));
        }

        rebuild(ctx, &node, file, host, args, &mut running, false)?;
      }
      Err(mpsc::RecvTimeoutError::Timeout) => exited(ctx, &mut running),
    }
  }

  drop(debouncer);

  if let Some(running) = running.take() {
    stop_launcher(running, ctx);
  }

  Ok(0)
}

fn rebuild(
  ctx: &mut Ctx<'_, '_>,
  node: &Path,
  file: Option<&Path>,
  host: Option<&str>,
  args: &[String],
  running: &mut Option<Running>,
  first: bool,
) -> Result<(), CliError> {
  let start = Instant::now();
  let built = project::run_target(file, &ctx.io.cwd)
    .and_then(|target| prepare(ctx, &target, host));
  let prepared = match built {
    Ok(Some(prepared)) => prepared,
    Ok(None) => {
      let _ = writeln!(ctx.io.err, "{}", kept(running.is_some()));
      return Ok(());
    }
    Err(e) if first => return Err(e),
    Err(e) => {
      e.report(ctx.io.err, ctx.debug);
      let _ = writeln!(ctx.io.err, "{}", kept(running.is_some()));
      return Ok(());
    }
  };
  let elapsed = start.elapsed().as_millis();
  let restarted = running.is_some();

  if let Some(old) = running.take() {
    stop_launcher(old, ctx);
  }

  let child =
    prepared.command(node, args, &ctx.io.cwd).spawn().map_err(|e| {
      CliError::Message(format!("cannot run {}: {e}", node.display()))
    })?;

  *running = Some(Running { child, prepared });

  let what = if restarted { "restarted" } else { "started" };

  let _ = writeln!(ctx.io.err, "built in {elapsed}ms, {what}");
  Ok(())
}

fn kept(running: bool) -> &'static str {
  if running {
    "build failed, keeping the running launcher"
  } else {
    "build failed, waiting for changes"
  }
}

fn prepare(
  ctx: &mut Ctx<'_, '_>,
  target: &RunTarget,
  host: Option<&str>,
) -> Result<Option<Prepared>, CliError> {
  match (target, host) {
    (RunTarget::Launch(project), None) => match &project.launch {
      Some(launch) => prepare_launch(ctx, project, launch),
      None => prepare_file(ctx, &project.main, None, Some(&project.hosts)),
    },
    (RunTarget::Launch(project), Some(host)) => {
      prepare_file(ctx, &project.main, Some(host), Some(&project.hosts))
    }
    (RunTarget::File { path, hosts }, host) => {
      prepare_file(ctx, path, host, hosts.as_deref())
    }
  }
}

fn exited(ctx: &mut Ctx<'_, '_>, running: &mut Option<Running>) {
  let Some(current) = running.as_mut() else { return };
  let Ok(Some(status)) = current.child.try_wait() else { return };
  let code =
    status.code().map_or_else(|| "a signal".to_string(), |c| c.to_string());

  let _ =
    writeln!(ctx.io.err, "launcher exited with {code}, waiting for changes");

  if let Some(done) = running.take() {
    keep_if_debug(ctx, done.prepared);
  }
}

fn stop_launcher(mut running: Running, ctx: &mut Ctx<'_, '_>) {
  if !matches!(running.child.try_wait(), Ok(Some(_))) {
    terminate(&running.child);

    let deadline = Instant::now() + GRACE;

    while Instant::now() < deadline {
      if matches!(running.child.try_wait(), Ok(Some(_))) {
        break;
      }

      std::thread::sleep(Duration::from_millis(20));
    }

    if !matches!(running.child.try_wait(), Ok(Some(_))) {
      let _ = running.child.kill();
    }

    let _ = running.child.wait();
  }

  keep_if_debug(ctx, running.prepared);
}

#[cfg(unix)]
fn terminate(child: &Child) {
  let _ = Command::new("kill")
    .arg("-TERM")
    .arg(child.id().to_string())
    .stdout(std::process::Stdio::null())
    .stderr(std::process::Stdio::null())
    .status();
}

#[cfg(not(unix))]
fn terminate(_: &Child) {}

fn shown(path: &Path, cwd: &Path) -> String {
  path.strip_prefix(cwd).unwrap_or(path).display().to_string()
}

fn inputs_of(
  ctx: &Ctx<'_, '_>,
  target: &RunTarget,
) -> Result<Inputs, CliError> {
  let cwd = &ctx.io.cwd;
  let mut inputs = Inputs::default();

  match target {
    RunTarget::Launch(project) => {
      let src = normalize(&cwd.join(&project.src));
      let dir = normalize(&cwd.join(&project.dir));

      inputs.out = Some(normalize(&cwd.join(&project.out)));
      inputs.trees.push(src.clone());
      inputs.sources.push(src);
      inputs.shallow.push(dir.clone());
      inputs.files.push(dir.join(project::CONFIG));

      for dep in project.package.dirs() {
        inputs.trees.push(dep.clone());
        inputs.sources.push(dep);
      }

      for extra in project.launch.iter().flat_map(|l| &l.watch) {
        let path = normalize(&dir.join(extra));

        if path.is_dir() {
          inputs.trees.push(path.clone());
        } else if let Some(parent) = path.parent() {
          inputs.shallow.push(parent.to_path_buf());
        }

        inputs.files.push(path);
      }
    }
    RunTarget::File { path, .. } => {
      let file = normalize(&cwd.join(path));
      let parent = file.parent().map(Path::to_path_buf).unwrap_or_default();

      inputs.trees.push(parent.clone());
      inputs.sources.push(parent);

      for input in discover(std::slice::from_ref(path), cwd, None)? {
        for dir in input.package.iter().flat_map(|p| p.dirs()) {
          inputs.trees.push(dir.clone());
          inputs.sources.push(dir);
        }
      }
    }
  }

  inputs.trees.dedup();
  inputs.shallow.dedup();
  inputs.trees.retain(|p| p.is_dir());
  inputs.shallow.retain(|p| p.is_dir());

  Ok(inputs)
}

impl Inputs {
  fn relevant(&self, paths: &[PathBuf], cwd: &Path) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = Vec::new();

    for path in paths.iter().map(|p| normalize(&cwd.join(p))) {
      if self.out.as_ref().is_some_and(|out| path.starts_with(out)) {
        continue;
      }

      let listed = self.files.iter().any(|f| path.starts_with(f));
      let source = path.extension().is_some_and(|e| e == "px")
        && self.sources.iter().any(|root| {
          path.strip_prefix(root).is_ok_and(|below| {
            !below.components().any(|c| {
              let name = c.as_os_str().to_string_lossy();

              matches!(name.as_ref(), "node_modules" | "target")
                || name.starts_with('.')
            })
          })
        });

      if (listed || source) && !found.contains(&path) {
        found.push(path);
      }
    }

    found
  }
}
