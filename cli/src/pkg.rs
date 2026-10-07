//! Git dependencies: addresses, versions, `polar.lock`, the shared cache and
//! the resolver (Go-style minimal version selection).

use crate::{CliError, files::write_atomic};
use serde::{Deserialize, Serialize};
use std::fmt::Write as _;
use std::{
  collections::{BTreeMap, BTreeSet},
  fs,
  path::{Path, PathBuf},
  process::Command,
  sync::atomic::{AtomicBool, Ordering},
};

pub const LOCK: &str = "polar.lock";

static OFFLINE: AtomicBool = AtomicBool::new(false);

pub fn set_offline(offline: bool) {
  OFFLINE.store(offline, Ordering::Relaxed);
}

#[derive(Debug, Clone)]
pub struct Settings {
  pub home: PathBuf,
  pub offline: bool,
}

impl Settings {
  #[must_use]
  pub fn current() -> Self {
    let home = std::env::var_os("POLAR_HOME").map_or_else(
      || {
        std::env::var_os("HOME").map_or_else(
          || PathBuf::from(".polar"),
          |h| Path::new(&h).join(".polar"),
        )
      },
      PathBuf::from,
    );
    let offline = OFFLINE.load(Ordering::Relaxed)
      || std::env::var("POLAR_OFFLINE").is_ok_and(|v| v == "1");

    Self { home, offline }
  }
}

fn fail<T>(message: String) -> Result<T, CliError> {
  Err(CliError::Message(message))
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version {
  pub major: u64,
  pub minor: u64,
  pub patch: u64,
}

impl Version {
  #[must_use]
  pub fn parse(text: &str) -> Option<Self> {
    let mut parts = text.strip_prefix('v')?.split('.');
    let mut next = || -> Option<u64> {
      let part = parts.next()?;

      if part.is_empty() || !part.chars().all(|c| c.is_ascii_digit()) {
        return None;
      }

      part.parse().ok()
    };
    let version = Self { major: next()?, minor: next()?, patch: next()? };

    parts.next().is_none().then_some(version)
  }

  #[must_use]
  pub fn text(&self) -> String {
    format!("v{}.{}.{}", self.major, self.minor, self.patch)
  }
}

/// A language release, `major.minor.patch`; a `-pre` suffix is ignored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Release {
  pub major: u64,
  pub minor: u64,
  pub patch: u64,
}

impl Release {
  #[must_use]
  pub fn parse(text: &str) -> Option<Self> {
    let core = text.split(['-', '+']).next()?;
    let mut parts = core.split('.').map(|p| p.parse::<u64>().ok());
    let release = Self {
      major: parts.next()??,
      minor: parts.next()??,
      patch: parts.next()??,
    };

    parts.next().is_none().then_some(release)
  }

  /// Older by major or minor: before 1.0 a minor bump is a breaking change,
  /// and a patch never is.
  #[must_use]
  pub fn older_than(self, other: Self) -> bool {
    (self.major, self.minor) < (other.major, other.minor)
  }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pin {
  Version(Version),
  Rev(String),
  Branch(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Requirement {
  pub address: String,
  pub pin: Pin,
}

/// The major a repository address asks for: `…/v2` is 2, anything else 0/1.
fn address_major(address: &str) -> Option<u64> {
  let last = address.rsplit('/').next()?;
  let n: u64 = last.strip_prefix('v')?.parse().ok()?;

  (n >= 2).then_some(n)
}

fn major_fits(address: &str, version: &Version) -> bool {
  match address_major(address) {
    Some(n) => version.major == n,
    None => version.major <= 1,
  }
}

/// Reads a `{ git = …, version|rev|branch = … }` dependency.
///
/// # Errors
///
/// Fails on a wrong shape, or a version whose major doesn't match the address.
pub fn requirement(
  key: &str,
  table: &toml::Table,
  shown: &str,
) -> Result<Requirement, CliError> {
  let bad = |why: &str| {
    CliError::Message(format!("invalid {shown}: the dependency `{key}` {why}"))
  };
  let text = |name: &str| -> Result<Option<String>, CliError> {
    match table.get(name) {
      None => Ok(None),
      Some(toml::Value::String(s)) => Ok(Some(s.clone())),
      Some(_) => Err(bad(&format!("has a `{name}` that isn't a string"))),
    }
  };

  if table.contains_key("path") {
    return Err(bad("can't have both `git` and `path`"));
  }

  for name in table.keys() {
    if !["git", "version", "rev", "branch"].contains(&name.as_str()) {
      return Err(bad(&format!("has an unknown field `{name}`")));
    }
  }

  let address = text("git")?.ok_or_else(|| bad("needs a `git` address"))?;
  let (version, rev, branch) =
    (text("version")?, text("rev")?, text("branch")?);
  let pin = match (version, rev, branch) {
    (Some(v), None, None) => {
      let parsed = Version::parse(&v).ok_or_else(|| {
        bad(&format!("has the version `{v}`; expected a tag like `v1.2.3`"))
      })?;

      if !major_fits(&address, &parsed) {
        return Err(bad(&format!(
          "is `{v}`, so its address must end in `/v{}`",
          parsed.major
        )));
      }

      Pin::Version(parsed)
    }
    (None, Some(r), None) => Pin::Rev(r),
    (None, None, Some(b)) => Pin::Branch(b),
    _ => {
      return Err(bad(
        "must pin exactly one of `version`, `rev` or `branch` next to `git`",
      ));
    }
  };

  Ok(Requirement { address, pin })
}

/// The URL `git` clones for an address.
#[must_use]
pub fn url(address: &str) -> String {
  if address.contains("://") || address.starts_with("git@") {
    address.to_string()
  } else {
    format!("https://{address}.git")
  }
}

/// A relative, filesystem-safe cache path for an address.
#[must_use]
pub fn cache_key(address: &str) -> PathBuf {
  let mut text = address.to_string();

  for prefix in ["https://", "http://", "ssh://", "git://", "file://"] {
    if let Some(rest) = text.strip_prefix(prefix) {
      text = rest.to_string();
      break;
    }
  }

  let text = text.strip_prefix("git@").unwrap_or(&text).replace(':', "/");
  let text = text.strip_suffix(".git").unwrap_or(&text);
  let mut out = PathBuf::new();

  for part in
    text.split('/').filter(|p| !p.is_empty() && *p != "." && *p != "..")
  {
    out.push(
      part
        .chars()
        .map(|c| {
          if c.is_ascii_alphanumeric() || "-_.".contains(c) { c } else { '_' }
        })
        .collect::<String>(),
    );
  }

  out
}

fn git(args: &[&str], cwd: Option<&Path>) -> Result<String, CliError> {
  let mut command = Command::new("git");

  command.args(args).env("GIT_TERMINAL_PROMPT", "0");

  if let Some(cwd) = cwd {
    command.current_dir(cwd);
  }

  let output = command.output().map_err(|e| {
    if e.kind() == std::io::ErrorKind::NotFound {
      CliError::Message(
        "`git` isn't installed, but git dependencies need it".into(),
      )
    } else {
      CliError::Message(format!("cannot run git: {e}"))
    }
  })?;

  if !output.status.success() {
    return fail(format!(
      "git {} failed: {}",
      args.first().copied().unwrap_or(""),
      String::from_utf8_lossy(&output.stderr).trim()
    ));
  }

  Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[derive(Debug, Default)]
pub struct Remote {
  pub tags: BTreeMap<String, String>,
  pub branches: BTreeMap<String, String>,
}

fn offline_error(what: &str, settings: &Settings) -> Result<(), CliError> {
  if settings.offline {
    return fail(format!("{what}, and `--offline` forbids fetching it"));
  }

  Ok(())
}

/// Lists the tags and branches of a repository.
///
/// # Errors
///
/// Fails offline, or if `git ls-remote` fails.
pub fn remote(address: &str, settings: &Settings) -> Result<Remote, CliError> {
  offline_error(&format!("`{address}` needs its tags listed"), settings)?;

  let listing = git(&["ls-remote", "--tags", "--heads", &url(address)], None)?;
  let mut out = Remote::default();

  for line in listing.lines() {
    let Some((commit, name)) = line.split_once('\t') else { continue };

    if let Some(tag) = name.strip_prefix("refs/tags/") {
      // An annotated tag lists the tag object, then the commit as `tag^{}`.
      if let Some(tag) = tag.strip_suffix("^{}") {
        out.tags.insert(tag.to_string(), commit.to_string());
      } else {
        out.tags.entry(tag.to_string()).or_insert_with(|| commit.to_string());
      }
    } else if let Some(branch) = name.strip_prefix("refs/heads/") {
      out.branches.insert(branch.to_string(), commit.to_string());
    }
  }

  Ok(out)
}

/// The highest semver tag that fits the address's major.
///
/// # Errors
///
/// Fails if the repository has no usable tag.
pub fn latest(address: &str, remote: &Remote) -> Result<Version, CliError> {
  remote
    .tags
    .keys()
    .filter_map(|t| Version::parse(t))
    .filter(|v| major_fits(address, v))
    .max()
    .ok_or_else(|| {
      CliError::Message(format!(
        "`{address}` has no version tags; use `{address}@<branch>` or \
         `{address}@<commit>`"
      ))
    })
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
  pub name: String,
  pub git: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub version: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub branch: Option<String>,
  pub commit: String,
  #[serde(default)]
  pub checksum: String,
}

impl Entry {
  /// Whether this locked package satisfies what a manifest asks for.
  #[must_use]
  pub fn satisfies(&self, want: &Requirement) -> bool {
    if self.git != want.address {
      return false;
    }

    match &want.pin {
      Pin::Version(v) => self
        .version
        .as_deref()
        .and_then(Version::parse)
        .is_some_and(|have| have >= *v),
      Pin::Rev(r) => {
        self.version.is_none() && self.branch.is_none() && self.commit == *r
      }
      Pin::Branch(b) => self.branch.as_deref() == Some(b.as_str()),
    }
  }

  #[must_use]
  pub fn shown(&self) -> String {
    let at = self
      .version
      .clone()
      .or_else(|| self.branch.clone())
      .unwrap_or_else(|| self.commit.chars().take(12).collect());

    format!("{}@{at}", self.git)
  }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Lock {
  #[serde(default, rename = "package")]
  pub packages: Vec<Entry>,
}

impl Lock {
  #[must_use]
  pub fn find(&self, name: &str) -> Option<&Entry> {
    self.packages.iter().find(|e| e.name == name)
  }

  /// Reads `polar.lock` in `dir`; no file is an empty lock.
  ///
  /// # Errors
  ///
  /// Fails if the file exists but can't be read or parsed.
  pub fn load(dir: &Path) -> Result<Self, CliError> {
    let path = dir.join(LOCK);

    if !path.is_file() {
      return Ok(Self::default());
    }

    let text = fs::read_to_string(&path)
      .map_err(|e| CliError::read(path.display().to_string(), &e))?;

    toml::from_str(&text).map_err(|e| {
      CliError::Message(format!("invalid {LOCK}: {}", e.message()))
    })
  }

  fn text(&self) -> Result<String, CliError> {
    let mut sorted = self.clone();

    sorted.packages.sort_by(|a, b| a.name.cmp(&b.name));

    let body = toml::to_string(&sorted)
      .map_err(|e| CliError::Message(format!("cannot write {LOCK}: {e}")))?;

    Ok(format!("# Written by `polar`. Commit it; don't edit it.\n\n{body}"))
  }

  /// Writes `polar.lock` in `dir` when it would change; an empty lock removes
  /// the file.
  ///
  /// # Errors
  ///
  /// Fails if the file can't be written.
  pub fn save(&self, dir: &Path) -> Result<bool, CliError> {
    let path = dir.join(LOCK);

    if self.packages.is_empty() {
      if path.is_file() {
        fs::remove_file(&path)
          .map_err(|e| CliError::write(path.display().to_string(), &e))?;

        return Ok(true);
      }

      return Ok(false);
    }

    let text = self.text()?;

    if fs::read_to_string(&path).is_ok_and(|old| old == text) {
      return Ok(false);
    }

    write_atomic(&path, text.as_bytes())
      .map_err(|e| CliError::write(path.display().to_string(), &e))?;

    Ok(true)
  }
}

fn package_dir(entry: &Entry, settings: &Settings) -> PathBuf {
  settings.home.join("pkg").join(cache_key(&entry.git)).join(&entry.commit)
}

/// The content hash of a package directory: every file's path and bytes, in
/// path order, minus git metadata and build output.
///
/// # Errors
///
/// Fails if a file can't be read.
pub fn checksum(dir: &Path) -> Result<String, CliError> {
  let mut files = Vec::new();

  collect(dir, dir, &mut files)?;
  files.sort();

  let mut bytes = Vec::new();

  for (rel, path) in files {
    let data = fs::read(&path)
      .map_err(|e| CliError::read(path.display().to_string(), &e))?;

    bytes.extend_from_slice(rel.as_bytes());
    bytes.push(0);
    bytes.extend_from_slice(data.len().to_string().as_bytes());
    bytes.push(0);
    bytes.extend_from_slice(&data);
  }

  Ok(format!("sha256:{}", sha256_hex(&bytes)))
}

fn collect(
  root: &Path,
  dir: &Path,
  out: &mut Vec<(String, PathBuf)>,
) -> Result<(), CliError> {
  let entries = fs::read_dir(dir)
    .map_err(|e| CliError::read(dir.display().to_string(), &e))?;

  for entry in entries.flatten() {
    let path = entry.path();
    let name = entry.file_name().to_string_lossy().into_owned();
    let top = dir == root;

    if top
      && [".git", ".polar", ".complete", "dist", "target"]
        .contains(&name.as_str())
    {
      continue;
    }

    if name == ".git" {
      continue;
    }

    if path.is_dir() {
      collect(root, &path, out)?;
    } else {
      let rel = path.strip_prefix(root).unwrap_or(&path);

      out.push((rel.to_string_lossy().replace('\\', "/"), path));
    }
  }

  Ok(())
}

/// Makes sure the locked package is in the cache, fetching it when it isn't,
/// and returns its directory.
///
/// # Errors
///
/// Fails offline with a cold cache, if git fails, or if the fetched tree
/// doesn't match the locked checksum.
pub fn ensure_cached(
  entry: &Entry,
  settings: &Settings,
) -> Result<PathBuf, CliError> {
  let dir = package_dir(entry, settings);

  if dir.join(".complete").is_file() {
    return Ok(dir);
  }

  offline_error(
    &format!(
      "`{}` (commit {}) isn't in the cache",
      entry.shown(),
      entry.commit
    ),
    settings,
  )?;

  let tmp_root = settings.home.join("tmp");

  fs::create_dir_all(&tmp_root)
    .map_err(|e| CliError::write(tmp_root.display().to_string(), &e))?;

  let tmp = tempfile::Builder::new()
    .prefix("fetch-")
    .tempdir_in(&tmp_root)
    .map_err(|e| CliError::write(tmp_root.display().to_string(), &e))?;
  let work = tmp.path().join("tree");
  let work_text = work.display().to_string();

  git(
    &[
      "clone",
      "--quiet",
      "--no-checkout",
      "--filter=blob:none",
      &url(&entry.git),
      &work_text,
    ],
    None,
  )?;
  git(&["checkout", "--quiet", "--detach", &entry.commit], Some(&work))
    .map_err(|_| {
      CliError::Message(format!(
        "`{}` has no commit {}",
        entry.git, entry.commit
      ))
    })?;
  fs::remove_dir_all(work.join(".git")).ok();

  let sum = checksum(&work)?;

  if !entry.checksum.is_empty() && entry.checksum != sum {
    return fail(format!(
      "checksum mismatch for `{}`: {LOCK} says {}, the fetched tree is {sum}",
      entry.shown(),
      entry.checksum
    ));
  }

  fs::write(work.join(".complete"), &sum)
    .map_err(|e| CliError::write(work.display().to_string(), &e))?;

  if let Some(parent) = dir.parent() {
    fs::create_dir_all(parent)
      .map_err(|e| CliError::write(parent.display().to_string(), &e))?;
  }

  // A concurrent fetch may have won; its result is the same tree.
  if fs::rename(&work, &dir).is_err() && !dir.join(".complete").is_file() {
    return fail(format!(
      "cannot move the fetched package into {}",
      dir.display()
    ));
  }

  Ok(dir)
}

/// Re-hashes a cached package and compares it with the lock.
///
/// # Errors
///
/// Fails on a mismatch, or if the package can't be read.
pub fn verify(entry: &Entry, settings: &Settings) -> Result<(), CliError> {
  let dir = ensure_cached(entry, settings)?;
  let sum = checksum(&dir)?;

  if sum != entry.checksum {
    return fail(format!(
      "checksum mismatch for `{}`: {LOCK} says {}, the cache has {sum}",
      entry.shown(),
      entry.checksum
    ));
  }

  Ok(())
}

pub struct Env {
  pub lock: Lock,
  pub settings: Settings,
}

/// What `resolve` should re-pin instead of keeping what the lock says.
pub enum Refresh {
  Nothing,
  All,
  Some(BTreeSet<String>),
}

impl Refresh {
  fn wants(&self, name: &str) -> bool {
    match self {
      Self::Nothing => false,
      Self::All => true,
      Self::Some(names) => names.contains(name),
    }
  }
}

struct Chosen {
  req: Requirement,
  entry: Entry,
  dir: PathBuf,
}

struct Resolver<'a> {
  old: &'a Lock,
  settings: &'a Settings,
  refresh: &'a Refresh,
  chosen: BTreeMap<String, Chosen>,
  remotes: BTreeMap<String, Remote>,
  seen_dirs: BTreeSet<PathBuf>,
}

/// Works out the whole dependency graph of a manifest and returns the lock.
///
/// # Errors
///
/// Fails on conflicting requirements, a missing tag, a name mismatch, a
/// dependency cycle, or any fetch error.
pub fn resolve(
  root: &Path,
  deps: &BTreeMap<String, toml::Value>,
  old: &Lock,
  settings: &Settings,
  refresh: &Refresh,
) -> Result<Lock, CliError> {
  let mut resolver = Resolver {
    old,
    settings,
    refresh,
    chosen: BTreeMap::new(),
    remotes: BTreeMap::new(),
    seen_dirs: BTreeSet::new(),
  };

  resolver.visit(deps, root, "polar.toml")?;

  let mut reachable = Vec::new();

  resolver.walk(deps, root, &mut Vec::new(), &mut reachable, "polar.toml")?;

  let mut packages: Vec<Entry> = Vec::new();

  for name in reachable {
    if !packages.iter().any(|e| e.name == name) {
      packages.push(resolver.chosen[&name].entry.clone());
    }
  }

  packages.sort_by(|a, b| a.name.cmp(&b.name));

  Ok(Lock { packages })
}

fn manifest_deps(
  dir: &Path,
  shown: &str,
) -> Result<(Option<String>, BTreeMap<String, toml::Value>), CliError> {
  let path = dir.join(crate::project::CONFIG);
  let text = fs::read_to_string(&path).map_err(|e| {
    CliError::read(format!("{shown}/{}", crate::project::CONFIG), &e)
  })?;
  let value: toml::Table = toml::from_str(&text).map_err(|e| {
    CliError::Message(format!("invalid {shown}/polar.toml: {}", e.message()))
  })?;
  let name = value
    .get("package")
    .and_then(|p| p.get("name"))
    .and_then(toml::Value::as_str)
    .map(str::to_string);
  let deps = match value.get("dependencies") {
    Some(toml::Value::Table(t)) => {
      t.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
    }
    _ => BTreeMap::new(),
  };

  Ok((name, deps))
}

impl Resolver<'_> {
  fn visit(
    &mut self,
    deps: &BTreeMap<String, toml::Value>,
    base: &Path,
    shown: &str,
  ) -> Result<(), CliError> {
    for (key, value) in deps {
      let Some(table) = value.as_table() else { continue };

      if table.contains_key("git") {
        let req = requirement(key, table, shown)?;

        self.want(key, req, shown)?;
      } else if let Some(path) = table.get("path").and_then(toml::Value::as_str)
      {
        let dir = crate::files::normalize(&base.join(path));

        if self.seen_dirs.insert(dir.clone())
          && dir.join(crate::project::CONFIG).is_file()
        {
          let (_, inner) = manifest_deps(&dir, &dir.display().to_string())?;

          self.visit(&inner, &dir, &dir.display().to_string())?;
        }
      }
    }

    Ok(())
  }

  fn want(
    &mut self,
    name: &str,
    req: Requirement,
    shown: &str,
  ) -> Result<(), CliError> {
    if let Some(have) = self.chosen.get(name) {
      if have.req.address != req.address {
        return fail(format!(
          "`{name}` is required from two places: `{}` and `{}`",
          have.req.address, req.address
        ));
      }

      match (&have.req.pin, &req.pin) {
        (a, b) if a == b => return Ok(()),
        (Pin::Version(a), Pin::Version(b)) => {
          if b <= a {
            return Ok(());
          }
        }
        _ => {
          return fail(format!(
            "conflicting requirements for `{name}` (from {shown}): \
             a version can't be combined with a different `rev` or `branch`"
          ));
        }
      }
    }

    let locked = self
      .old
      .find(name)
      .filter(|e| !self.refresh.wants(name) && e.satisfies(&req))
      .cloned();
    let entry = match locked {
      Some(entry) => entry,
      None => self.pin(name, &req)?,
    };
    let dir = ensure_cached(&entry, self.settings)?;
    let mut entry = entry;

    if entry.checksum.is_empty() {
      entry.checksum = checksum(&dir)?;
    }

    let (package, inner) = manifest_deps(&dir, &entry.shown())?;

    if package.as_deref() != Some(name) {
      return fail(format!(
        "the dependency `{name}` ({}) is the package `{}`; name it `{}`",
        entry.shown(),
        package.as_deref().unwrap_or("?"),
        package.as_deref().unwrap_or("?")
      ));
    }

    let shown = entry.shown();

    self
      .chosen
      .insert(name.to_string(), Chosen { req, entry, dir: dir.clone() });
    self.visit(&inner, &dir, &shown)
  }

  fn remote(&mut self, address: &str) -> Result<&Remote, CliError> {
    if !self.remotes.contains_key(address) {
      let listed = remote(address, self.settings)?;

      self.remotes.insert(address.to_string(), listed);
    }

    Ok(&self.remotes[address])
  }

  fn pin(&mut self, name: &str, req: &Requirement) -> Result<Entry, CliError> {
    let mut entry = Entry {
      name: name.to_string(),
      git: req.address.clone(),
      ..Entry::default()
    };

    match &req.pin {
      Pin::Version(v) => {
        let text = v.text();
        let commit = self.remote(&req.address)?.tags.get(&text).cloned();

        entry.commit = commit.ok_or_else(|| {
          CliError::Message(format!("`{}` has no tag `{text}`", req.address))
        })?;
        entry.version = Some(text);
      }
      Pin::Rev(rev) => {
        if rev.len() != 40 || !rev.chars().all(|c| c.is_ascii_hexdigit()) {
          return fail(format!(
            "the `rev` of `{name}` must be a full 40-character commit"
          ));
        }

        entry.commit.clone_from(rev);
      }
      Pin::Branch(branch) => {
        let commit = self.remote(&req.address)?.branches.get(branch).cloned();

        entry.commit = commit.ok_or_else(|| {
          CliError::Message(format!(
            "`{}` has no branch `{branch}`",
            req.address
          ))
        })?;
        entry.branch = Some(branch.clone());
      }
    }

    Ok(entry)
  }

  /// Collects the packages reachable from the root through the chosen
  /// versions, failing on a cycle.
  fn walk(
    &self,
    deps: &BTreeMap<String, toml::Value>,
    base: &Path,
    stack: &mut Vec<String>,
    out: &mut Vec<String>,
    shown: &str,
  ) -> Result<(), CliError> {
    for (key, value) in deps {
      let Some(table) = value.as_table() else { continue };

      if table.contains_key("git") {
        if let Some(start) = stack.iter().position(|n| n == key) {
          let mut cycle: Vec<String> = stack[start..].to_vec();

          cycle.push(key.clone());

          return fail(format!("package cycle: {}", cycle.join(" → ")));
        }

        let Some(chosen) = self.chosen.get(key) else { continue };
        let first = !out.contains(key);

        out.push(key.clone());

        if first {
          let (_, inner) = manifest_deps(&chosen.dir, &chosen.entry.shown())?;

          stack.push(key.clone());
          self.walk(&inner, &chosen.dir, stack, out, shown)?;
          stack.pop();
        }
      } else if let Some(path) = table.get("path").and_then(toml::Value::as_str)
      {
        let dir = crate::files::normalize(&base.join(path));

        if dir.join(crate::project::CONFIG).is_file() {
          let (_, inner) = manifest_deps(&dir, shown)?;

          self.walk(&inner, &dir, stack, out, shown)?;
        }
      }
    }

    Ok(())
  }
}

#[must_use]
#[allow(clippy::too_many_lines)]
pub fn sha256_hex(data: &[u8]) -> String {
  const K: [u32; 64] = [
    0x428a_2f98,
    0x7137_4491,
    0xb5c0_fbcf,
    0xe9b5_dba5,
    0x3956_c25b,
    0x59f1_11f1,
    0x923f_82a4,
    0xab1c_5ed5,
    0xd807_aa98,
    0x1283_5b01,
    0x2431_85be,
    0x550c_7dc3,
    0x72be_5d74,
    0x80de_b1fe,
    0x9bdc_06a7,
    0xc19b_f174,
    0xe49b_69c1,
    0xefbe_4786,
    0x0fc1_9dc6,
    0x240c_a1cc,
    0x2de9_2c6f,
    0x4a74_84aa,
    0x5cb0_a9dc,
    0x76f9_88da,
    0x983e_5152,
    0xa831_c66d,
    0xb003_27c8,
    0xbf59_7fc7,
    0xc6e0_0bf3,
    0xd5a7_9147,
    0x06ca_6351,
    0x1429_2967,
    0x27b7_0a85,
    0x2e1b_2138,
    0x4d2c_6dfc,
    0x5338_0d13,
    0x650a_7354,
    0x766a_0abb,
    0x81c2_c92e,
    0x9272_2c85,
    0xa2bf_e8a1,
    0xa81a_664b,
    0xc24b_8b70,
    0xc76c_51a3,
    0xd192_e819,
    0xd699_0624,
    0xf40e_3585,
    0x106a_a070,
    0x19a4_c116,
    0x1e37_6c08,
    0x2748_774c,
    0x34b0_bcb5,
    0x391c_0cb3,
    0x4ed8_aa4a,
    0x5b9c_ca4f,
    0x682e_6ff3,
    0x748f_82ee,
    0x78a5_636f,
    0x84c8_7814,
    0x8cc7_0208,
    0x90be_fffa,
    0xa450_6ceb,
    0xbef9_a3f7,
    0xc671_78f2,
  ];
  let mut h: [u32; 8] = [
    0x6a09_e667,
    0xbb67_ae85,
    0x3c6e_f372,
    0xa54f_f53a,
    0x510e_527f,
    0x9b05_688c,
    0x1f83_d9ab,
    0x5be0_cd19,
  ];
  let mut message = data.to_vec();
  let bits = (data.len() as u64).wrapping_mul(8);

  message.push(0x80);

  while message.len() % 64 != 56 {
    message.push(0);
  }

  message.extend_from_slice(&bits.to_be_bytes());

  for block in message.chunks(64) {
    let mut w = [0u32; 64];

    for (i, word) in block.chunks(4).enumerate() {
      w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
    }

    for i in 16..64 {
      let s0 = w[i - 15].rotate_right(7)
        ^ w[i - 15].rotate_right(18)
        ^ (w[i - 15] >> 3);
      let s1 = w[i - 2].rotate_right(17)
        ^ w[i - 2].rotate_right(19)
        ^ (w[i - 2] >> 10);

      w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
    }

    let mut v = h;

    for i in 0..64 {
      let s1 =
        v[4].rotate_right(6) ^ v[4].rotate_right(11) ^ v[4].rotate_right(25);
      let ch = (v[4] & v[5]) ^ (!v[4] & v[6]);
      let t1 = v[7]
        .wrapping_add(s1)
        .wrapping_add(ch)
        .wrapping_add(K[i])
        .wrapping_add(w[i]);
      let s0 =
        v[0].rotate_right(2) ^ v[0].rotate_right(13) ^ v[0].rotate_right(22);
      let maj = (v[0] & v[1]) ^ (v[0] & v[2]) ^ (v[1] & v[2]);
      let t2 = s0.wrapping_add(maj);

      v = [
        t1.wrapping_add(t2),
        v[0],
        v[1],
        v[2],
        v[3].wrapping_add(t1),
        v[4],
        v[5],
        v[6],
      ];
    }

    for (slot, add) in h.iter_mut().zip(v) {
      *slot = slot.wrapping_add(add);
    }
  }

  h.iter().fold(String::new(), |mut out, x| {
    let _ = write!(out, "{x:08x}");
    out
  })
}
