pub const RUNTIME_JS: &str = include_str!("../../runtime/runtime.js");
pub const LAUNCHER_JS: &str = include_str!("js/launcher.mjs");
pub const TEST_RUNNER_JS: &str = include_str!("js/test_runner.mjs");
pub const TEST_REPORTER_JS: &str = include_str!("js/test_reporter.mjs");
pub const REPL_RUNNER_JS: &str = include_str!("js/repl_runner.mjs");
pub const START_JS: &str = include_str!("js/start.mjs");

#[cfg(test)]
mod tests {
  #[test]
  fn runtime_is_embedded() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../runtime/runtime.js");
    let on_disk = std::fs::read_to_string(path).expect("read runtime.js");

    assert_eq!(super::RUNTIME_JS, on_disk);
  }
}
