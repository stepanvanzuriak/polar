use std::{
  fs,
  path::{Path, PathBuf},
  process::{Command, Output},
};

const POLAR: &str = env!("CARGO_BIN_EXE_polar");

struct Ran {
  code: Option<i32>,
  out: String,
  err: String,
}

struct World {
  dir: tempfile::TempDir,
}

impl World {
  fn new() -> Self {
    Self { dir: tempfile::tempdir().unwrap() }
  }

  fn path(&self, rel: &str) -> PathBuf {
    self.dir.path().join(rel)
  }

  fn home(&self) -> PathBuf {
    self.path("home")
  }

  fn polar(&self, cwd: &str, args: &[&str]) -> Ran {
    self.polar_env(cwd, args, &[])
  }

  fn polar_env(&self, cwd: &str, args: &[&str], env: &[(&str, &str)]) -> Ran {
    let mut command = Command::new(POLAR);

    command
      .args(args)
      .current_dir(self.path(cwd))
      .env("POLAR_HOME", self.home())
      .env_remove("POLAR_DEBUG")
      .env_remove("POLAR_OFFLINE");

    for (k, v) in env {
      command.env(k, v);
    }

    let out: Output = command.output().expect("run polar");

    Ran {
      code: out.status.code(),
      out: String::from_utf8_lossy(&out.stdout).into_owned(),
      err: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
  }

  fn write(&self, files: &[(&str, &str)]) {
    for (path, text) in files {
      let path = self.path(path);

      fs::create_dir_all(path.parent().unwrap()).unwrap();
      fs::write(path, text).unwrap();
    }
  }

  fn git(&self, dir: &str, args: &[&str]) -> String {
    let out = Command::new("git")
      .args([
        "-c",
        "user.name=t",
        "-c",
        "user.email=t@t",
        "-c",
        "commit.gpgsign=false",
      ])
      .args(args)
      .current_dir(self.path(dir))
      .output()
      .unwrap();

    assert!(
      out.status.success(),
      "git {args:?}: {}",
      String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
  }

  /// A repository at `dir` holding `files`, committed and tagged.
  fn repo(&self, dir: &str, files: &[(&str, &str)], tags: &[&str]) -> String {
    fs::create_dir_all(self.path(dir)).unwrap();
    self.git(dir, &["init", "-q", "-b", "main"]);
    self.write(
      &files
        .iter()
        .map(|(p, t)| (format!("{dir}/{p}"), *t))
        .map(|(p, t)| (Box::leak(p.into_boxed_str()) as &str, t))
        .collect::<Vec<_>>(),
    );
    self.commit(dir, tags)
  }

  fn commit(&self, dir: &str, tags: &[&str]) -> String {
    self.git(dir, &["add", "-A"]);
    self.git(dir, &["commit", "-q", "-m", "c"]);

    for tag in tags {
      self.git(dir, &["tag", tag]);
    }

    self.git(dir, &["rev-parse", "HEAD"])
  }

  fn address(&self, dir: &str) -> String {
    format!("file://{}", self.path(dir).display())
  }
}

fn greet(name: &str, says: &str, deps: &str) -> Vec<(String, String)> {
  vec![
    (
      "polar.toml".into(),
      format!(
        "[package]\nname = \"{name}\"\nmodule = \"{}\"\n{deps}",
        capital(name)
      ),
    ),
    (
      "src/hello.px".into(),
      format!(
        "module Hello\n\nfunctions\n  say() -> String {{\n    \"{says}\"\n  }}\n\nexports\n  say\n"
      ),
    ),
  ]
}

fn capital(name: &str) -> String {
  let mut c = name.chars();

  c.next().unwrap().to_uppercase().chain(c).collect()
}

fn files_ref(files: &[(String, String)]) -> Vec<(&str, &str)> {
  files.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect()
}

fn repo_of(
  w: &World,
  dir: &str,
  name: &str,
  says: &str,
  deps: &str,
  tags: &[&str],
) -> String {
  let files = greet(name, says, deps);

  w.repo(dir, &files_ref(&files), tags)
}

fn app(w: &World, dependencies: &str) {
  w.write(&[
    ("app/polar.toml", &format!("[project]\nname = \"app\"\n\n[dependencies]\n{dependencies}")),
    (
      "app/src/main.px",
      "module Main\n\nuses\n  Greet.Hello\n\nfunctions\n  main() {\n    Log.info(Hello.say())\n  }\n\nexports\n  main\n",
    ),
  ]);
}

fn lock(w: &World) -> String {
  fs::read_to_string(w.path("app/polar.lock")).unwrap_or_default()
}

mod shape {
  use super::*;

  fn check(deps: &str) -> Ran {
    let w = World::new();

    app(&w, deps);
    w.polar("app", &["fetch"])
  }

  #[test]
  fn git_and_path_together_are_rejected() {
    let ran = check(
      "greet = { git = \"x/y\", path = \"../g\", version = \"v1.0.0\" }\n",
    );

    assert_eq!(ran.code, Some(1));
    assert!(
      ran.err.contains("`greet`") && ran.err.contains("both `git` and `path`"),
      "{}",
      ran.err
    );
  }

  #[test]
  fn git_without_a_pin_is_rejected() {
    let ran = check("greet = { git = \"x/y\" }\n");

    assert_eq!(ran.code, Some(1));
    assert!(
      ran.err.contains("`greet`") && ran.err.contains("exactly one"),
      "{}",
      ran.err
    );
  }

  #[test]
  fn a_v2_tag_needs_a_v2_address() {
    let ran = check("greet = { git = \"x/y\", version = \"v2.0.0\" }\n");

    assert_eq!(ran.code, Some(1));
    assert!(ran.err.contains("/v2"), "{}", ran.err);
  }
}

#[test]
fn add_picks_the_highest_tag_by_semver() {
  let w = World::new();

  repo_of(
    &w,
    "greet",
    "greet",
    "one",
    "",
    &["v1.0.0", "v1.2.0", "v1.10.0", "v1.9.0"],
  );
  w.write(&[("app/polar.toml", "[project]\nname = \"app\"\n")]);
  fs::create_dir_all(w.path("app/src")).unwrap();

  let ran = w.polar("app", &["add", &w.address("greet")]);

  assert_eq!(ran.code, Some(0), "{}", ran.err);

  let manifest = fs::read_to_string(w.path("app/polar.toml")).unwrap();

  assert!(manifest.contains("greet = { git = "), "{manifest}");
  assert!(manifest.contains("version = \"v1.10.0\""), "{manifest}");
  assert!(lock(&w).contains("version = \"v1.10.0\""));
}

#[test]
fn add_without_tags_explains() {
  let w = World::new();

  repo_of(&w, "greet", "greet", "one", "", &[]);
  w.write(&[("app/polar.toml", "[project]\nname = \"app\"\n")]);

  let ran = w.polar("app", &["add", &w.address("greet")]);

  assert_eq!(ran.code, Some(1));
  assert!(ran.err.contains("no version tags"), "{}", ran.err);
}

#[test]
fn the_lock_is_sorted_and_stable() {
  let w = World::new();
  let a = w.address("greet");

  repo_of(&w, "greet", "greet", "hi", "", &["v1.0.0"]);
  repo_of(&w, "abc", "abc", "x", "", &["v1.0.0"]);
  app(
    &w,
    &format!(
      "greet = {{ git = \"{a}\", version = \"v1.0.0\" }}\nabc = {{ git = \"{}\", version = \"v1.0.0\" }}\n",
      w.address("abc")
    ),
  );

  assert_eq!(w.polar("app", &["fetch"]).code, Some(0));

  let first = lock(&w);

  assert!(
    first.find("name = \"abc\"").unwrap()
      < first.find("name = \"greet\"").unwrap()
  );
  assert!(
    first.contains("commit = \"") && first.contains("checksum = \"sha256:")
  );

  let ran = w.polar("app", &["fetch"]);

  assert_eq!(ran.code, Some(0));
  assert_eq!(first, lock(&w));
  assert!(!ran.out.contains("updated"), "{}", ran.out);
}

#[test]
fn the_highest_requested_version_wins() {
  let w = World::new();
  let shared = w.address("shared");

  repo_of(&w, "shared", "shared", "v1", "", &["v1.1.0"]);
  w.write(&[("shared/src/hello.px", "module Hello\n\nfunctions\n  say() -> String {\n    \"v13\"\n  }\n\nexports\n  say\n")]);
  w.commit("shared", &["v1.3.0"]);

  let dep =
    format!("shared = {{ git = \"{shared}\", version = \"v1.1.0\" }}\n");

  repo_of(
    &w,
    "left",
    "left",
    "l",
    &format!("\n[dependencies]\n{dep}"),
    &["v1.0.0"],
  );
  repo_of(
    &w,
    "right",
    "right",
    "r",
    &format!(
      "\n[dependencies]\nshared = {{ git = \"{shared}\", version = \"v1.3.0\" }}\n"
    ),
    &["v1.0.0"],
  );
  app(
    &w,
    &format!(
      "left = {{ git = \"{}\", version = \"v1.0.0\" }}\nright = {{ git = \"{}\", version = \"v1.0.0\" }}\n",
      w.address("left"),
      w.address("right")
    ),
  );

  let ran = w.polar("app", &["fetch"]);

  assert_eq!(ran.code, Some(0), "{}", ran.err);

  let text = lock(&w);

  assert_eq!(text.matches("name = \"shared\"").count(), 1, "{text}");
  assert!(text.contains("version = \"v1.3.0\""), "{text}");
  assert!(!text.contains("v1.1.0"), "{text}");
}

#[test]
fn a_cached_file_that_changed_fails_verification() {
  let w = World::new();

  repo_of(&w, "greet", "greet", "hi", "", &["v1.0.0"]);
  app(
    &w,
    &format!(
      "greet = {{ git = \"{}\", version = \"v1.0.0\" }}\n",
      w.address("greet")
    ),
  );
  assert_eq!(w.polar("app", &["fetch"]).code, Some(0));

  let commit = lock(&w)
    .lines()
    .find_map(|l| {
      l.strip_prefix("commit = \"").map(|c| c.trim_end_matches('"').to_string())
    })
    .unwrap();
  let dir = fs::read_dir(w.home().join("pkg"))
    .unwrap()
    .flatten()
    .map(|e| e.path())
    .find_map(|p| find_commit(&p, &commit))
    .unwrap();

  fs::write(dir.join("src/hello.px"), "tampered").unwrap();

  let ran = w.polar("app", &["fetch", "--verify"]);

  assert_eq!(ran.code, Some(1));
  assert!(
    ran.err.contains("checksum mismatch") && ran.err.contains("sha256:"),
    "{}",
    ran.err
  );
}

fn find_commit(dir: &Path, commit: &str) -> Option<PathBuf> {
  if dir.file_name().is_some_and(|n| n == commit) {
    return Some(dir.to_path_buf());
  }

  fs::read_dir(dir).ok()?.flatten().find_map(|e| find_commit(&e.path(), commit))
}

#[test]
fn a_name_that_is_not_the_package_is_rejected() {
  let w = World::new();

  repo_of(&w, "bar", "bar", "x", "", &["v1.0.0"]);
  app(
    &w,
    &format!(
      "foo = {{ git = \"{}\", version = \"v1.0.0\" }}\n",
      w.address("bar")
    ),
  );

  let ran = w.polar("app", &["fetch"]);

  assert_eq!(ran.code, Some(1));
  assert!(ran.err.contains("is the package `bar`"), "{}", ran.err);
}

#[test]
fn offline_with_a_cold_cache_never_touches_the_network() {
  let w = World::new();

  repo_of(&w, "greet", "greet", "hi", "", &["v1.0.0"]);
  app(
    &w,
    &format!(
      "greet = {{ git = \"{}\", version = \"v1.0.0\" }}\n",
      w.address("greet")
    ),
  );
  assert_eq!(w.polar("app", &["fetch"]).code, Some(0));
  fs::remove_dir_all(w.home()).unwrap();

  let ran = w.polar("app", &["--offline", "check"]);

  assert_eq!(ran.code, Some(1));
  assert!(ran.err.contains("--offline"), "{}", ran.err);

  let ran = w.polar_env("app", &["check"], &[("POLAR_OFFLINE", "1")]);

  assert_eq!(ran.code, Some(1));
  assert!(ran.err.contains("--offline"), "{}", ran.err);
}

#[test]
fn git_cycles_are_reported() {
  let w = World::new();
  let (a, b) = (w.address("a"), w.address("b"));

  repo_of(
    &w,
    "a",
    "a",
    "x",
    &format!(
      "\n[dependencies]\nb = {{ git = \"{b}\", version = \"v1.0.0\" }}\n"
    ),
    &[],
  );
  repo_of(
    &w,
    "b",
    "b",
    "x",
    &format!(
      "\n[dependencies]\na = {{ git = \"{a}\", version = \"v1.0.0\" }}\n"
    ),
    &["v1.0.0"],
  );
  w.git("a", &["tag", "v1.0.0"]);
  app(&w, &format!("a = {{ git = \"{a}\", version = \"v1.0.0\" }}\n"));

  let ran = w.polar("app", &["fetch"]);

  assert_eq!(ran.code, Some(1));
  assert!(ran.err.contains("package cycle: a → b → a"), "{}", ran.err);
}

#[test]
fn a_failed_fetch_leaves_nothing_that_looks_complete() {
  let w = World::new();

  repo_of(&w, "greet", "greet", "hi", "", &["v1.0.0"]);
  app(
    &w,
    &format!(
      "greet = {{ git = \"{}\", version = \"v1.0.0\" }}\n",
      w.address("greet")
    ),
  );
  assert_eq!(w.polar("app", &["fetch"]).code, Some(0));

  let text = lock(&w).replace(
    lock(&w).lines().find(|l| l.starts_with("commit")).unwrap(),
    "commit = \"0000000000000000000000000000000000000000\"",
  );

  fs::write(w.path("app/polar.lock"), text).unwrap();

  let ran = w.polar("app", &["check"]);

  assert_eq!(ran.code, Some(1));
  assert!(ran.err.contains("no commit"), "{}", ran.err);
  assert!(
    find_commit(
      &w.home().join("pkg"),
      "0000000000000000000000000000000000000000"
    )
    .is_none()
  );
}

#[test]
fn a_project_runs_a_git_dependency() {
  let w = World::new();

  repo_of(&w, "greet", "greet", "hello from git", "", &["v1.0.0"]);
  app(
    &w,
    &format!(
      "greet = {{ git = \"{}\", version = \"v1.0.0\" }}\n",
      w.address("greet")
    ),
  );
  assert_eq!(w.polar("app", &["fetch"]).code, Some(0));

  let ran = w.polar("app", &["run"]);

  assert_eq!(ran.code, Some(0), "{}", ran.err);
  assert!(ran.out.contains("hello from git"), "{}{}", ran.out, ran.err);
}

#[test]
fn run_fetches_a_locked_package_with_a_cold_cache() {
  let w = World::new();

  repo_of(&w, "greet", "greet", "cold", "", &["v1.0.0"]);
  app(
    &w,
    &format!(
      "greet = {{ git = \"{}\", version = \"v1.0.0\" }}\n",
      w.address("greet")
    ),
  );
  assert_eq!(w.polar("app", &["fetch"]).code, Some(0));
  fs::remove_dir_all(w.home()).unwrap();

  let ran = w.polar("app", &["run"]);

  assert_eq!(ran.code, Some(0), "{}", ran.err);
  assert!(ran.out.contains("cold"), "{}", ran.out);
}

#[test]
fn a_git_dependency_missing_from_the_lock_says_to_fetch() {
  let w = World::new();

  repo_of(&w, "greet", "greet", "hi", "", &["v1.0.0"]);
  app(
    &w,
    &format!(
      "greet = {{ git = \"{}\", version = \"v1.0.0\" }}\n",
      w.address("greet")
    ),
  );

  let ran = w.polar("app", &["check"]);

  assert_eq!(ran.code, Some(1));
  assert!(ran.err.contains("run `polar fetch`"), "{}", ran.err);
}

#[test]
fn update_moves_to_the_newest_tag_and_remove_drops_it() {
  let w = World::new();

  repo_of(&w, "greet", "greet", "one", "", &["v1.0.0"]);
  app(
    &w,
    &format!(
      "greet = {{ git = \"{}\", version = \"v1.0.0\" }}\n",
      w.address("greet")
    ),
  );
  assert_eq!(w.polar("app", &["fetch"]).code, Some(0));
  w.write(&[("greet/src/hello.px", "module Hello\n\nfunctions\n  say() -> String {\n    \"two\"\n  }\n\nexports\n  say\n")]);
  w.commit("greet", &["v1.1.0"]);

  let ran = w.polar("app", &["update"]);

  assert_eq!(ran.code, Some(0), "{}", ran.err);
  assert!(
    fs::read_to_string(w.path("app/polar.toml")).unwrap().contains("v1.1.0")
  );
  assert!(lock(&w).contains("v1.1.0"));
  assert!(w.polar("app", &["run"]).out.contains("two"));

  let ran = w.polar("app", &["remove", "greet"]);

  assert_eq!(ran.code, Some(0), "{}", ran.err);
  assert!(
    !fs::read_to_string(w.path("app/polar.toml")).unwrap().contains("greet")
  );
  assert!(!w.path("app/polar.lock").exists());
}
