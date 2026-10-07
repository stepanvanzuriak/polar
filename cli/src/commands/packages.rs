use crate::{
  CliError, Ctx,
  files::write_atomic,
  pkg::{self, Lock, Pin, Refresh, Requirement, Settings, Version},
  project::{self, CONFIG},
};
use std::{collections::BTreeSet, path::Path};

fn message<T>(text: String) -> Result<T, CliError> {
  Err(CliError::Message(text))
}

fn manifest_path(cwd: &Path) -> std::path::PathBuf {
  cwd.join(CONFIG)
}

fn read_manifest(cwd: &Path) -> Result<String, CliError> {
  let path = manifest_path(cwd);

  std::fs::read_to_string(&path).map_err(|e| CliError::read(CONFIG, &e))
}

fn write_manifest(cwd: &Path, text: &str) -> Result<(), CliError> {
  write_atomic(&manifest_path(cwd), text.as_bytes())
    .map_err(|e| CliError::write(CONFIG, &e))
}

/// The line index range `[start, end)` of the `[dependencies]` body.
fn dependencies_span(lines: &[String]) -> Option<(usize, usize)> {
  let start = lines.iter().position(|l| l.trim() == "[dependencies]")? + 1;
  let end = lines[start..]
    .iter()
    .position(|l| l.trim_start().starts_with('['))
    .map_or(lines.len(), |i| start + i);

  Some((start, end))
}

fn line_of(lines: &[String], name: &str) -> Option<usize> {
  let (start, end) = dependencies_span(lines)?;

  (start..end).find(|&i| {
    let line = lines[i].trim_start();

    line
      .strip_prefix(name)
      .is_some_and(|rest| rest.trim_start().starts_with('='))
  })
}

fn lines_of(text: &str) -> Vec<String> {
  text.lines().map(str::to_string).collect()
}

fn join(lines: &[String]) -> String {
  format!("{}\n", lines.join("\n"))
}

fn pin_text(pin: &Pin) -> String {
  match pin {
    Pin::Version(v) => format!("version = \"{}\"", v.text()),
    Pin::Rev(r) => format!("rev = \"{r}\""),
    Pin::Branch(b) => format!("branch = \"{b}\""),
  }
}

fn dependency_line(name: &str, req: &Requirement) -> String {
  format!("{name} = {{ git = \"{}\", {} }}", req.address, pin_text(&req.pin))
}

fn say(ctx: &mut Ctx<'_, '_>, text: &str) {
  let _ = writeln!(ctx.io.out, "{text}");
}

fn relock(ctx: &mut Ctx<'_, '_>, refresh: &Refresh) -> Result<Lock, CliError> {
  let cwd = ctx.io.cwd.clone();
  let (root, deps) = project::manifest_dependencies(Path::new(""), &cwd)?;
  let old = Lock::load(&root)?;
  let settings = Settings::current();
  let new = pkg::resolve(&root, &deps, &old, &settings, refresh)?;

  if new.save(&root)? {
    say(ctx, &format!("updated {}", pkg::LOCK));
  }

  for entry in &new.packages {
    if !old.packages.contains(entry) {
      say(ctx, &format!("  {} {}", entry.name, entry.shown()));
    }
  }

  Ok(new)
}

pub(crate) fn fetch(
  ctx: &mut Ctx<'_, '_>,
  verify: bool,
) -> Result<u8, CliError> {
  let lock = relock(ctx, &Refresh::Nothing)?;

  if verify {
    let settings = Settings::current();

    for entry in &lock.packages {
      pkg::verify(entry, &settings)?;
    }

    say(
      ctx,
      &format!("verified {}", crate::plural(lock.packages.len(), "package")),
    );
  }

  Ok(0)
}

/// `address`, `address@v1.2.3`, `address@<40 hex>` or `address@branch`.
pub(crate) fn add(ctx: &mut Ctx<'_, '_>, spec: &str) -> Result<u8, CliError> {
  let cwd = ctx.io.cwd.clone();
  let settings = Settings::current();
  let (address, at) = match spec.rsplit_once('@') {
    Some((a, v)) if !a.is_empty() && !v.is_empty() && a != "git" => {
      (a, Some(v))
    }
    _ => (spec, None),
  };
  let remote = pkg::remote(address, &settings)?;
  let pin = match at {
    None => Pin::Version(pkg::latest(address, &remote)?),
    Some(v) => match Version::parse(v) {
      Some(version) => Pin::Version(version),
      None if v.len() == 40 && v.chars().all(|c| c.is_ascii_hexdigit()) => {
        Pin::Rev(v.to_string())
      }
      None => Pin::Branch(v.to_string()),
    },
  };
  let req = Requirement { address: address.to_string(), pin };
  // The dependency's key is the package's own name, so fetch it to learn it.
  let name = package_name(&req, &remote, &settings)?;
  let text = read_manifest(&cwd)?;
  let mut lines = lines_of(&text);

  if line_of(&lines, &name).is_some() {
    return message(format!(
      "`{name}` is already a dependency; use `polar update {name}`"
    ));
  }

  let line = dependency_line(&name, &req);

  if let Some((_, end)) = dependencies_span(&lines) {
    let mut at = end;

    while at > 0 && lines[at - 1].trim().is_empty() {
      at -= 1;
    }

    lines.insert(at, line);
  } else {
    lines.push(String::new());
    lines.push("[dependencies]".into());
    lines.push(line);
  }

  write_manifest(&cwd, &join(&lines))?;
  relock(ctx, &Refresh::Nothing)?;
  say(ctx, &format!("added {name}"));

  Ok(0)
}

fn package_name(
  req: &Requirement,
  remote: &pkg::Remote,
  settings: &Settings,
) -> Result<String, CliError> {
  let mut entry = pkg::Entry {
    name: String::new(),
    git: req.address.clone(),
    ..pkg::Entry::default()
  };

  match &req.pin {
    Pin::Version(v) => {
      entry.commit = remote.tags.get(&v.text()).cloned().ok_or_else(|| {
        CliError::Message(format!(
          "`{}` has no tag `{}`",
          req.address,
          v.text()
        ))
      })?;
    }
    Pin::Rev(r) => entry.commit.clone_from(r),
    Pin::Branch(b) => {
      entry.commit = remote.branches.get(b).cloned().ok_or_else(|| {
        CliError::Message(format!("`{}` has no branch `{b}`", req.address))
      })?;
    }
  }

  let dir = pkg::ensure_cached(&entry, settings)?;
  let text = std::fs::read_to_string(dir.join(CONFIG))
    .map_err(|e| CliError::read(format!("{}/{CONFIG}", req.address), &e))?;
  let table: toml::Table = toml::from_str(&text).map_err(|e| {
    CliError::Message(format!(
      "invalid {}/{CONFIG}: {}",
      req.address,
      e.message()
    ))
  })?;

  table
    .get("package")
    .and_then(|p| p.get("name"))
    .and_then(toml::Value::as_str)
    .map(str::to_string)
    .ok_or_else(|| {
      CliError::Message(format!(
        "`{}` isn't a package: its {CONFIG} has no `[package]`",
        req.address
      ))
    })
}

pub(crate) fn remove(
  ctx: &mut Ctx<'_, '_>,
  name: &str,
) -> Result<u8, CliError> {
  let cwd = ctx.io.cwd.clone();
  let mut lines = lines_of(&read_manifest(&cwd)?);
  let Some(at) = line_of(&lines, name) else {
    return message(format!("`{name}` isn't a dependency"));
  };

  lines.remove(at);
  write_manifest(&cwd, &join(&lines))?;
  relock(ctx, &Refresh::Nothing)?;
  say(ctx, &format!("removed {name}"));

  Ok(0)
}

pub(crate) fn update(
  ctx: &mut Ctx<'_, '_>,
  name: Option<&str>,
) -> Result<u8, CliError> {
  let cwd = ctx.io.cwd.clone();
  let settings = Settings::current();
  let (_, deps) = project::manifest_dependencies(Path::new(""), &cwd)?;
  let mut lines = lines_of(&read_manifest(&cwd)?);
  let mut refresh = BTreeSet::new();

  if let Some(name) = name {
    if deps.get(name).is_none_or(|v| v.get("git").is_none()) {
      return message(format!("`{name}` isn't a git dependency"));
    }
  }

  for (key, value) in &deps {
    let Some(table) = value.as_table().filter(|t| t.contains_key("git")) else {
      continue;
    };

    if name.is_some_and(|n| n != key) {
      continue;
    }

    let req = pkg::requirement(key, table, CONFIG)?;

    refresh.insert(key.clone());

    if let Pin::Version(_) = req.pin {
      let newest =
        pkg::latest(&req.address, &pkg::remote(&req.address, &settings)?)?;
      let new = Requirement { pin: Pin::Version(newest), ..req };

      if let Some(at) = line_of(&lines, key) {
        lines[at] = dependency_line(key, &new);
      }
    }
  }

  write_manifest(&cwd, &join(&lines))?;

  let refresh =
    if name.is_some() { Refresh::Some(refresh) } else { Refresh::All };

  relock(ctx, &refresh)?;

  Ok(0)
}
