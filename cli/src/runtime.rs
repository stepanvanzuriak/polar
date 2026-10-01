pub const RUNTIME_JS: &str = include_str!("../../runtime/runtime.js");
pub const LAUNCHER_JS: &str = include_str!("launcher.mjs");
pub const START_JS: &str = include_str!("start.mjs");

#[cfg(test)]
mod tests {
  #[test]
  fn runtime_is_embedded() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../runtime/runtime.js");
    let on_disk = std::fs::read_to_string(path).expect("read runtime.js");

    assert_eq!(super::RUNTIME_JS, on_disk);
  }
}
