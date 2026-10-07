use crate::{CliError, files::normalize, pkg};
use polar_compiler::shared::modules;
use serde::Deserialize;
use std::{
  collections::{BTreeMap, HashMap},
  path::{Path, PathBuf},
  sync::Arc,
};

pub const CONFIG: &str = "polar.toml";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfigFile {
  project: Option<Section>,
  package: Option<PackageSection>,
  plugin: Option<PluginSection>,
  launcher: Option<LauncherSection>,
  run: Option<RunSection>,
  #[serde(default)]
  dependencies: BTreeMap<String, toml::Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LauncherSection {
  script: PathBuf,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RunSection {
  launcher: String,
  #[serde(default)]
  options: toml::Table,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PluginSection {
  zones: Vec<String>,
  #[serde(default)]
  dependencies: BTreeMap<String, toml::Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Section {
  name: String,
  #[serde(default = "default_src")]
  src: PathBuf,
  #[serde(default = "default_out")]
  out: PathBuf,
  #[serde(default = "default_main")]
  main: PathBuf,
  #[serde(default)]
  hosts: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PackageSection {
  name: String,
  module: String,
  polar: Option<String>,
  #[serde(default = "default_src")]
  src: PathBuf,
}

fn default_src() -> PathBuf {
  PathBuf::from("src")
}

fn default_out() -> PathBuf {
  PathBuf::from("dist")
}

fn default_main() -> PathBuf {
  PathBuf::from("src/main.px")
}

#[derive(Debug, PartialEq, Eq)]
pub struct Source {
  pub path: PathBuf,
  pub display: PathBuf,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Package {
  pub name: String,
  pub module: Option<String>,
  pub source: Source,
  pub plugin: Option<String>,
  pub plugins: Vec<String>,
  pub launcher: Option<PathBuf>,
  pub deps: Vec<Arc<Package>>,
}

impl Package {
  #[must_use]
  pub fn out_root(&self) -> PathBuf {
    match &self.module {
      Some(_) => Path::new("_deps").join(&self.name),
      None => PathBuf::new(),
    }
  }

  #[must_use]
  pub fn transitive(&self) -> Vec<Arc<Package>> {
    let mut out: Vec<Arc<Package>> = Vec::new();
    let mut todo: Vec<Arc<Package>> = self.deps.clone();

    while let Some(next) = todo.pop() {
      if out.iter().any(|p| p.name == next.name) {
        continue;
      }

      todo.extend(next.deps.iter().cloned());
      out.push(next);
    }

    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
  }

  #[must_use]
  pub fn dirs(&self) -> Vec<PathBuf> {
    self.transitive().iter().map(|p| p.source.path.clone()).collect()
  }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Project {
  pub name: String,
  pub dir: PathBuf,
  pub src: PathBuf,
  pub out: PathBuf,
  pub main: PathBuf,
  pub hosts: Vec<String>,
  pub package: Arc<Package>,
  pub library: bool,
  pub launch: Option<Launch>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launch {
  pub package: String,
  pub script: PathBuf,
  pub options: serde_json::Value,
}

/// Loads the project whose config sits in `dir`, or `None` if there is none.
///
/// # Errors
///
/// Fails if the config cannot be read or parsed, names an absolute path, or
/// one of its dependencies cannot be loaded.
#[allow(clippy::too_many_lines)]
pub fn load(dir: &Path, cwd: &Path) -> Result<Option<Project>, CliError> {
  let config = cwd.join(dir).join(CONFIG);

  if !config.is_file() {
    return Ok(None);
  }

  let shown = dir.join(CONFIG).display().to_string();
  let text =
    std::fs::read_to_string(&config).map_err(|e| CliError::read(&shown, &e))?;
  let parsed = parse(&text, &shown)?;
  let root = normalize(&cwd.join(dir));
  let mut stack = vec![root.clone()];
  let env = pkg::Env {
    lock: pkg::Lock::load(&root)?,
    settings: pkg::Settings::current(),
  };

  match (parsed.project, parsed.package) {
    (Some(_), Some(_)) => Err(invalid(
      &shown,
      "a manifest has a `[project]` or a `[package]` section, not both",
    )),
    (None, None) => {
      Err(invalid(&shown, "expected a `[project]` or a `[package]` section"))
    }
    (Some(section), None) => {
      for (key, path) in
        [("src", &section.src), ("out", &section.out), ("main", &section.main)]
      {
        if path.is_absolute() {
          return Err(invalid(
            &shown,
            &format!("`{key}` must be relative to the project directory"),
          ));
        }
      }

      if parsed.launcher.is_some() {
        return Err(invalid(
          &shown,
          "only a `[package]` provides a `[launcher]`; a project picks one with \
           `[run]`",
        ));
      }

      let place =
        Place { root: root.clone(), display: dir.to_path_buf(), env: &env };
      let deps =
        dependencies(&parsed.dependencies, &place, &shown, &mut stack)?;
      let launch =
        parsed.run.map(|run| launch_of(run, &deps, &shown)).transpose()?;
      let plugin =
        own_plugin(parsed.plugin.as_ref(), &place, &section.name, &shown)?;
      let package = Arc::new(Package {
        plugins: enabled(plugin.as_ref(), &deps, &shown)?,
        name: section.name.clone(),
        module: None,
        source: Source {
          path: normalize(&root.join(&section.src)),
          display: dir.join(&section.src),
        },
        plugin,
        launcher: None,
        deps,
      });

      collisions(&package, &shown)?;

      Ok(Some(Project {
        name: section.name,
        src: dir.join(&section.src),
        out: dir.join(&section.out),
        main: dir.join(&section.main),
        hosts: section.hosts,
        dir: dir.to_path_buf(),
        package,
        library: false,
        launch,
      }))
    }
    (None, Some(section)) => {
      if parsed.run.is_some() {
        return Err(invalid(
          &shown,
          "a `[package]` isn't run, so it has no `[run]` section",
        ));
      }

      let package = package_of(
        section,
        parsed.launcher.as_ref(),
        parsed.plugin.as_ref(),
        &parsed.dependencies,
        &Place { root, display: dir.to_path_buf(), env: &env },
        &shown,
        &mut stack,
      )?;
      let src = package.source.display.clone();

      Ok(Some(Project {
        name: package.name.clone(),
        src,
        out: dir.join(default_out()),
        main: dir.join(default_main()),
        hosts: Vec::new(),
        dir: dir.to_path_buf(),
        package: Arc::new(package),
        library: true,
        launch: None,
      }))
    }
  }
}

fn parse(text: &str, shown: &str) -> Result<ConfigFile, CliError> {
  toml::from_str(text)
    .map_err(|e| CliError::Message(format!("invalid {shown}: {}", e.message())))
}

fn invalid(shown: &str, message: &str) -> CliError {
  CliError::Message(format!("invalid {shown}: {message}"))
}

fn own_plugin(
  section: Option<&PluginSection>,
  place: &Place,
  name: &str,
  shown: &str,
) -> Result<Option<String>, CliError> {
  let Some(section) = section else { return Ok(None) };

  if section.zones.is_empty() {
    return Err(invalid(
      shown,
      "`[plugin]` needs at least one zone in `zones`",
    ));
  }

  crate::plugin::prepare(
    &place.root,
    &place.display,
    name,
    &section.zones,
    &section.dependencies,
    shown,
  )
  .map(Some)
}

fn enabled(
  own: Option<&String>,
  deps: &[Arc<Package>],
  shown: &str,
) -> Result<Vec<String>, CliError> {
  let libraries: Vec<String> = own
    .into_iter()
    .chain(deps.iter().filter_map(|d| d.plugin.as_ref()))
    .cloned()
    .collect();
  let mut seen: Vec<String> = Vec::new();

  for library in &libraries {
    for zone in polar_compiler::syntax::plugins::load(library)
      .map_err(CliError::Message)?
    {
      if seen.contains(&zone.keyword) {
        return Err(invalid(
          shown,
          &format!("two plugins define the zone `{}`", zone.keyword),
        ));
      }

      seen.push(zone.keyword);
    }
  }

  Ok(libraries)
}

struct Place<'a> {
  root: PathBuf,
  display: PathBuf,
  env: &'a pkg::Env,
}

fn dependencies(
  deps: &BTreeMap<String, toml::Value>,
  place: &Place<'_>,
  shown: &str,
  stack: &mut Vec<PathBuf>,
) -> Result<Vec<Arc<Package>>, CliError> {
  let mut out: Vec<Arc<Package>> = Vec::new();

  for (key, value) in deps {
    let package = match value {
      toml::Value::Table(table) if table.contains_key("git") => {
        git_package(key, table, place, shown, stack)?
      }
      toml::Value::Table(table) => match (table.get("path"), table.len()) {
        (Some(toml::Value::String(path)), 1) => path_package(
          key,
          &normalize(&place.root.join(path)),
          &place.display.join(path),
          place.env,
          shown,
          stack,
        )?,
        _ => return Err(dependency_shape(key, shown)),
      },
      _ => return Err(dependency_shape(key, shown)),
    };

    if package.name != *key {
      return Err(invalid(
        shown,
        &format!(
          "the dependency `{key}` is the package `{}`; name it `{}`",
          package.name, package.name
        ),
      ));
    }

    let module = package.module.clone().unwrap_or_default();

    if module == "Std" {
      return Err(invalid(
        shown,
        &format!(
          "the package `{}` claims the module `Std`, which is the standard library",
          package.name
        ),
      ));
    }

    if let Some(other) =
      out.iter().find(|p| p.module.as_deref() == Some(module.as_str()))
    {
      return Err(invalid(
        shown,
        &format!(
          "the packages `{}` and `{}` both claim the module `{module}`",
          other.name, package.name
        ),
      ));
    }

    out.push(Arc::new(package));
  }

  Ok(out)
}

fn dependency_shape(key: &str, shown: &str) -> CliError {
  invalid(
    shown,
    &format!(
      "the dependency `{key}` must be `{{ path = \"…\" }}` or `{{ git = \"…\", \
       version = \"…\" }}`"
    ),
  )
}

fn git_package(
  key: &str,
  table: &toml::Table,
  place: &Place<'_>,
  shown: &str,
  stack: &mut Vec<PathBuf>,
) -> Result<Package, CliError> {
  let want = pkg::requirement(key, table, shown)?;
  let Some(entry) = place.env.lock.find(key) else {
    return Err(invalid(
      shown,
      &format!(
        "the dependency `{key}` isn't in {}; run `polar fetch`",
        pkg::LOCK
      ),
    ));
  };

  if !entry.satisfies(&want) {
    return Err(invalid(
      shown,
      &format!(
        "{} doesn't satisfy the dependency `{key}`; run `polar fetch`",
        pkg::LOCK
      ),
    ));
  }

  let dir = pkg::ensure_cached(entry, &place.env.settings)?;

  path_package(
    key,
    &dir,
    &PathBuf::from(entry.shown()),
    place.env,
    shown,
    stack,
  )
}

fn path_package(
  key: &str,
  dir: &Path,
  display: &Path,
  env: &pkg::Env,
  shown: &str,
  stack: &mut Vec<PathBuf>,
) -> Result<Package, CliError> {
  if let Some(start) = stack.iter().position(|d| d == dir) {
    let mut cycle: Vec<String> =
      stack[start..].iter().map(|d| d.display().to_string()).collect();

    cycle.push(dir.display().to_string());

    return Err(CliError::Message(format!(
      "package cycle: {}",
      cycle.join(" → ")
    )));
  }

  let config = dir.join(CONFIG);
  let manifest = display.join(CONFIG).display().to_string();

  if !config.is_file() {
    return Err(invalid(
      shown,
      &format!(
        "the dependency `{key}` has no {CONFIG} at `{}`",
        display.display()
      ),
    ));
  }

  let text = std::fs::read_to_string(&config)
    .map_err(|e| CliError::read(&manifest, &e))?;
  let parsed = parse(&text, &manifest)?;

  if parsed.project.is_some() {
    return Err(invalid(
      shown,
      &format!(
        "the dependency `{key}` at `{}` is a project, not a package: its \
         {CONFIG} needs `[package]`",
        display.display()
      ),
    ));
  }

  let Some(section) = parsed.package else {
    return Err(invalid(&manifest, "expected a `[package]` section"));
  };

  if parsed.run.is_some() {
    return Err(invalid(
      &manifest,
      "a `[package]` isn't run, so it has no `[run]` section",
    ));
  }

  language_version(&section, display, &manifest)?;
  stack.push(dir.to_path_buf());

  let package = package_of(
    section,
    parsed.launcher.as_ref(),
    parsed.plugin.as_ref(),
    &parsed.dependencies,
    &Place { root: dir.to_path_buf(), display: display.to_path_buf(), env },
    &manifest,
    stack,
  );

  stack.pop();
  package
}

/// The version of Polar this build is.
pub const POLAR_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Warns when a dependency says it was written for an older Polar than this
/// one; a package that doesn't say is left alone.
fn language_version(
  section: &PackageSection,
  display: &Path,
  manifest: &str,
) -> Result<(), CliError> {
  let Some(declared) = &section.polar else { return Ok(()) };
  let Some(theirs) = pkg::Release::parse(declared) else {
    return Err(invalid(
      manifest,
      &format!("`polar = \"{declared}\"` must be a version like `0.1.0`"),
    ));
  };
  let ours = pkg::Release::parse(POLAR_VERSION).unwrap_or(theirs);

  if theirs.older_than(ours) {
    crate::plugin::notice(format!(
      "warning: the package `{}` ({}) was written for Polar {declared}, but \
       this is Polar {POLAR_VERSION}; it may not build or behave the same",
      section.name,
      display.display()
    ));
  } else if ours.older_than(theirs) {
    crate::plugin::notice(format!(
      "warning: the package `{}` ({}) needs Polar {declared}, but this is \
       Polar {POLAR_VERSION}; update Polar, or it may not build",
      section.name,
      display.display()
    ));
  }

  Ok(())
}

fn package_of(
  section: PackageSection,
  launcher: Option<&LauncherSection>,
  plugin: Option<&PluginSection>,
  deps: &BTreeMap<String, toml::Value>,
  place: &Place<'_>,
  shown: &str,
  stack: &mut Vec<PathBuf>,
) -> Result<Package, CliError> {
  let module = &section.module;
  let valid = module.chars().next().is_some_and(char::is_uppercase)
    && module.chars().all(char::is_alphanumeric);

  if !valid {
    return Err(invalid(
      shown,
      &format!(
        "`module = \"{module}\"` must be one capitalised name, like `Web`"
      ),
    ));
  }

  if section.src.is_absolute() {
    return Err(invalid(
      shown,
      "`src` must be relative to the package directory",
    ));
  }

  let source = Source {
    path: normalize(&place.root.join(&section.src)),
    display: place.display.join(&section.src),
  };

  let launcher = launcher
    .map(|launcher| launcher_script(launcher, place, shown))
    .transpose()?;
  let deps = dependencies(deps, place, shown, stack)?;
  let plugin = own_plugin(plugin, place, &section.name, shown)?;

  Ok(Package {
    plugins: enabled(plugin.as_ref(), &deps, shown)?,
    name: section.name,
    module: Some(section.module),
    source,
    plugin,
    launcher,
    deps,
  })
}

fn launcher_script(
  launcher: &LauncherSection,
  place: &Place,
  shown: &str,
) -> Result<PathBuf, CliError> {
  if launcher.script.is_absolute() {
    return Err(invalid(
      shown,
      "the launcher's `script` must be relative to the package directory",
    ));
  }

  let script = normalize(&place.root.join(&launcher.script));

  if !script.is_file() {
    return Err(invalid(
      shown,
      &format!(
        "the launcher script `{}` doesn't exist",
        place.display.join(&launcher.script).display()
      ),
    ));
  }

  Ok(script)
}

fn launch_of(
  run: RunSection,
  deps: &[Arc<Package>],
  shown: &str,
) -> Result<Launch, CliError> {
  let Some(package) = deps.iter().find(|d| d.name == run.launcher) else {
    return Err(invalid(
      shown,
      &format!(
        "`[run]` uses the launcher of `{}`, which isn't a dependency of this \
         project",
        run.launcher
      ),
    ));
  };
  let Some(script) = &package.launcher else {
    return Err(invalid(
      shown,
      &format!(
        "`[run]` uses the launcher of `{}`, but that package has no `[launcher]`",
        run.launcher
      ),
    ));
  };
  let options = serde_json::to_value(&run.options).map_err(|e| {
    invalid(shown, &format!("`[run]` options can't be passed on: {e}"))
  })?;

  Ok(Launch { package: run.launcher, script: script.clone(), options })
}

fn collisions(project: &Package, shown: &str) -> Result<(), CliError> {
  let Source { path: src, display } = &project.source;
  let all = project.transitive();

  for (i, package) in all.iter().enumerate() {
    let module = package.module.clone().unwrap_or_default();

    if let Some(other) =
      all[..i].iter().find(|p| p.module.as_deref() == Some(module.as_str()))
    {
      return Err(invalid(
        shown,
        &format!(
          "the packages `{}` and `{}` both claim the module `{module}`",
          other.name, package.name
        ),
      ));
    }

    let file = PathBuf::from(modules::file(&module));
    let dir = file.with_extension("");

    for own in [&file, &dir] {
      if src.join(own).exists() {
        return Err(invalid(
          shown,
          &format!(
            "`{}` and the package `{}` both claim the module `{module}`",
            display.join(own).display(),
            package.name
          ),
        ));
      }
    }
  }

  Ok(())
}

#[derive(Default)]
pub struct Contexts {
  cache: HashMap<PathBuf, Option<Arc<Package>>>,
}

impl Contexts {
  /// The package a file belongs to: the one whose `polar.toml` is nearest
  /// above it.
  ///
  /// # Errors
  ///
  /// Fails if that `polar.toml` cannot be loaded.
  pub fn of(
    &mut self,
    file: &Path,
    cwd: &Path,
  ) -> Result<Option<Arc<Package>>, CliError> {
    let mut walked = Vec::new();
    let mut found = None;

    for dir in file.ancestors().skip(1) {
      if let Some(cached) = self.cache.get(dir) {
        found.clone_from(cached);
        break;
      }

      walked.push(dir.to_path_buf());

      if dir.join(CONFIG).is_file() {
        found = load(dir, cwd)?.map(|p| p.package);
        break;
      }
    }

    for dir in walked {
      self.cache.insert(dir, found.clone());
    }

    Ok(found)
  }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
  pub paths: Vec<PathBuf>,
  pub out: PathBuf,
  pub project: Option<String>,
  pub hosts: Vec<String>,
  pub library: bool,
  pub start: Option<Start>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Start {
  pub dir: PathBuf,
  pub main: PathBuf,
  pub source: PathBuf,
  pub launch: Option<Launch>,
}

impl Project {
  #[must_use]
  pub fn start(&self) -> Option<Start> {
    if self.library {
      return None;
    }

    let main = self
      .main
      .strip_prefix(&self.src)
      .map_or_else(|_| PathBuf::from("main.js"), |m| m.with_extension("js"));
    let source =
      self.main.strip_prefix(&self.dir).unwrap_or(&self.main).to_path_buf();

    Some(Start {
      dir: self.dir.clone(),
      main,
      source,
      launch: self.launch.clone(),
    })
  }
}

/// Groups path arguments into build targets: one per project directory, plus
/// one for all the remaining plain paths.
///
/// # Errors
///
/// Fails if a project config is invalid, no path is given outside a project, or
/// `--out` is combined with more than one project.
pub fn targets(
  paths: &[PathBuf],
  cwd: &Path,
  out: Option<&Path>,
) -> Result<Vec<Target>, CliError> {
  if paths.is_empty() {
    return match load(Path::new(""), cwd)? {
      Some(project) => Ok(vec![project_target(project, out)]),
      None => Err(no_paths()),
    };
  }

  let mut targets = Vec::new();
  let mut plain = Vec::new();

  for path in paths {
    let project = if cwd.join(path).is_dir() { load(path, cwd)? } else { None };

    match project {
      Some(project) => targets.push(project_target(project, out)),
      None => plain.push(path.clone()),
    }
  }

  if out.is_some() && targets.len() > 1 {
    return Err(CliError::Message(
      "`--out` applies to one project at a time; each project writes to its own `out`"
        .to_string(),
    ));
  }

  if !plain.is_empty() {
    let out = out.map_or_else(default_out, Path::to_path_buf);

    targets.push(Target {
      paths: plain,
      out,
      project: None,
      hosts: Vec::new(),
      library: false,
      start: None,
    });
  }

  Ok(targets)
}

fn project_target(project: Project, out: Option<&Path>) -> Target {
  let start = project.start();

  Target {
    paths: vec![project.src],
    out: out.map_or(project.out, Path::to_path_buf),
    project: Some(project.name),
    hosts: project.hosts,
    library: project.library,
    start,
  }
}

fn no_paths() -> CliError {
  CliError::Message(format!(
    "a path is required: name the files or directories to use, or run inside a \
     project (a directory with a `{CONFIG}`)"
  ))
}

/// Resolves the file `polar run` executes: a project's `main`, or `file` itself.
///
/// # Errors
///
/// Fails if a project config is invalid, no file is given outside a project,
/// or the directory holds a package rather than a project.
pub fn run_file(file: Option<&Path>, cwd: &Path) -> Result<PathBuf, CliError> {
  run_target(file, cwd).map(|target| match target {
    RunTarget::File { path, .. } => path,
    RunTarget::Launch(project) => project.main,
  })
}

pub(crate) enum RunTarget {
  File { path: PathBuf, hosts: Option<Vec<String>> },
  Launch(Box<Project>),
}

pub(crate) fn run_target(
  file: Option<&Path>,
  cwd: &Path,
) -> Result<RunTarget, CliError> {
  let dir = file.unwrap_or(Path::new(""));

  if file.is_none() || cwd.join(dir).is_dir() {
    if let Some(project) = load(dir, cwd)? {
      if project.library {
        return Err(library(&project.name));
      }

      if project.launch.is_some() {
        return Ok(RunTarget::Launch(Box::new(project)));
      }

      return Ok(RunTarget::File {
        path: project.main,
        hosts: Some(project.hosts),
      });
    }
  }

  file
    .map(|f| RunTarget::File { path: f.to_path_buf(), hosts: None })
    .ok_or_else(no_paths)
}

pub(crate) fn library(name: &str) -> CliError {
  CliError::Message(format!(
    "`{name}` is a package, which isn't built or run on its own: `polar check` \
     it, or depend on it from a project"
  ))
}

/// The dependency table of the manifest in `dir`, for the package commands.
///
/// # Errors
///
/// Fails if the manifest is missing or can't be parsed.
pub fn manifest_dependencies(
  dir: &Path,
  cwd: &Path,
) -> Result<(PathBuf, BTreeMap<String, toml::Value>), CliError> {
  let root = normalize(&cwd.join(dir));
  let config = root.join(CONFIG);

  if !config.is_file() {
    return Err(CliError::Message(format!(
      "no {CONFIG} in {}",
      root.display()
    )));
  }

  let shown = dir.join(CONFIG).display().to_string();
  let text =
    std::fs::read_to_string(&config).map_err(|e| CliError::read(&shown, &e))?;
  let parsed = parse(&text, &shown)?;

  Ok((root, parsed.dependencies))
}
