use super::testing::Options;
use crate::{CliError, Ctx, project};
use std::{
  path::{Path, PathBuf},
  process::{Command, Output},
  sync::atomic::{AtomicUsize, Ordering},
  thread,
  time::Instant,
};
use walkdir::WalkDir;

struct Job {
  display: PathBuf,
  root: PathBuf,
}

#[derive(Default)]
struct Totals {
  passed: usize,
  failed: usize,
  filtered: usize,
}

#[allow(clippy::too_many_lines)]
pub(crate) fn test(
  ctx: &mut Ctx<'_, '_>,
  members: &[PathBuf],
  options: &Options<'_>,
) -> Result<u8, CliError> {
  let cwd = ctx.io.cwd.clone();
  let mut jobs: Vec<Job> = Vec::new();
  let mut names: Vec<String> = Vec::new();

  for dir in members {
    let member = project::member(dir, &cwd)?;

    names.push(member.name.clone());

    let chosen =
      options.projects.is_empty() || options.projects.contains(&member.name);

    if chosen
      && !member.library
      && has_tests(&crate::files::normalize(&cwd.join(&member.src)))
    {
      jobs.push(Job {
        display: dir.clone(),
        root: crate::files::normalize(&cwd.join(dir)),
      });
    }
  }

  if let Some(unknown) =
    options.projects.iter().find(|name| !names.contains(name))
  {
    return Err(CliError::Message(format!(
      "the workspace has no project `{unknown}`; its members are {}",
      names.iter().map(|n| format!("`{n}`")).collect::<Vec<_>>().join(", ")
    )));
  }

  if jobs.is_empty() {
    let _ = writeln!(
      ctx.io.out,
      "no tests found (looked for *_test.px in the workspace members)"
    );
    return Ok(0);
  }

  let exe = std::env::current_exe().map_err(|e| {
    CliError::Message(format!("cannot find the polar executable: {e}"))
  })?;
  let mut args: Vec<String> = Vec::new();

  args.push(if ctx.color { "--color" } else { "--no-color" }.to_string());

  if crate::pkg::Settings::current().offline {
    args.push("--offline".to_string());
  }

  args.extend(["test".to_string(), "--timeout".to_string()]);
  args.push(options.timeout.to_string());

  if options.bless {
    args.push("--bless".to_string());
  }

  if options.json {
    args.extend(["--format".to_string(), "json".to_string()]);
  }

  if let Some(filter) = options.filter {
    args.extend(["--filter".to_string(), filter.to_string()]);
  }

  if !options.args.is_empty() {
    args.push("--".to_string());
    args.extend(options.args.iter().cloned());
  }

  let start = Instant::now();
  let outputs = run_all(&exe, &args, &jobs, options.jobs.max(1));
  let mut totals = Totals::default();
  let mut code = 0u8;

  for (job, output) in jobs.iter().zip(outputs) {
    if !options.json {
      let _ = writeln!(ctx.io.out, "\nproject {}", job.display.display());
    }

    match output {
      Ok(output) => {
        let stdout = String::from_utf8_lossy(&output.stdout);

        let _ = write!(ctx.io.out, "{stdout}");
        let _ = ctx.io.err.write_all(&output.stderr);

        let found = tally(&stdout, &mut totals, options.json);
        let exit =
          output.status.code().map_or(1, |c| u8::try_from(c).unwrap_or(1));

        if exit != 0 || !found {
          totals.failed += usize::from(!found);
          code = code.max(exit.max(1));
        }
      }
      Err(message) => {
        let _ = writeln!(ctx.io.err, "error: {message}");
        totals.failed += 1;
        code = code.max(1);
      }
    }
  }

  let status = if code == 0 { "ok" } else { "FAILED" };
  let projects = jobs.len();

  if options.json {
    let object = serde_json::json!({
      "event": "workspace",
      "ok": code == 0,
      "projects": projects,
      "passed": totals.passed,
      "failed": totals.failed,
      "filtered": totals.filtered,
      "seconds": start.elapsed().as_secs_f64(),
    });
    let _ = writeln!(ctx.io.out, "{object}");

    return Ok(code);
  }

  let _ = write!(
    ctx.io.out,
    "workspace result: {status}. {projects} {}; {} passed; {} failed; {} \
     filtered out; finished in {:.2}s\n\n",
    if projects == 1 { "project" } else { "projects" },
    totals.passed,
    totals.failed,
    totals.filtered,
    start.elapsed().as_secs_f64(),
  );

  Ok(code)
}

fn run_all(
  exe: &Path,
  args: &[String],
  jobs: &[Job],
  workers: usize,
) -> Vec<Result<Output, String>> {
  let next = AtomicUsize::new(0);
  let slots: Vec<std::sync::Mutex<Option<Result<Output, String>>>> =
    jobs.iter().map(|_| std::sync::Mutex::new(None)).collect();

  thread::scope(|scope| {
    for _ in 0..workers.min(jobs.len()) {
      scope.spawn(|| {
        loop {
          let i = next.fetch_add(1, Ordering::SeqCst);

          let Some(job) = jobs.get(i) else { break };
          let result = Command::new(exe)
            .args(args)
            .current_dir(&job.root)
            .output()
            .map_err(|e| {
              format!("cannot run polar in {}: {e}", job.display.display())
            });

          if let Ok(mut slot) = slots[i].lock() {
            *slot = Some(result);
          }
        }
      });
    }
  });

  slots
    .into_iter()
    .map(|slot| {
      slot
        .into_inner()
        .ok()
        .flatten()
        .unwrap_or_else(|| Err("the test run did not finish".to_string()))
    })
    .collect()
}

fn tally(stdout: &str, totals: &mut Totals, json: bool) -> bool {
  if json {
    let found = stdout
      .lines()
      .rev()
      .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
      .find(|v| v["event"] == "result");
    let Some(found) = found else { return false };
    let count = |key: &str| {
      usize::try_from(found[key].as_u64().unwrap_or_default())
        .unwrap_or_default()
    };

    totals.passed += count("passed");
    totals.failed += count("failed");
    totals.filtered += count("filtered");

    return true;
  }

  let Some(line) = stdout.lines().rev().find(|l| l.contains("test result:"))
  else {
    return false;
  };

  for part in line.split(['.', ';']) {
    let mut words = part.split_whitespace();
    let count = words.next().and_then(|w| w.parse::<usize>().ok());
    let label = words.next();

    match (count, label) {
      (Some(n), Some("passed")) => totals.passed += n,
      (Some(n), Some("failed")) => totals.failed += n,
      (Some(n), Some("filtered")) => totals.filtered += n,
      _ => {}
    }
  }

  true
}

fn has_tests(src: &Path) -> bool {
  WalkDir::new(src)
    .into_iter()
    .filter_entry(|entry| {
      !matches!(
        entry.file_name().to_str(),
        Some("dist" | ".polar" | "node_modules")
      )
    })
    .flatten()
    .any(|entry| {
      entry.file_type().is_file()
        && entry.file_name().to_string_lossy().ends_with("_test.px")
    })
}
