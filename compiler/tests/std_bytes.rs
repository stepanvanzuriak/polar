mod common;

use std::path::PathBuf;

fn run(src: &str) -> String {
  common::node::run_program(src, "test.px", &[])
    .unwrap_or_else(|err| panic!("{err}"))
}

fn fixture() -> PathBuf {
  PathBuf::from(env!("CARGO_MANIFEST_DIR"))
    .join("tests/fixtures/programs/std_bytes.px")
}

#[test]
fn std_bytes_program_runs_as_expected() {
  let path = fixture();
  let src = std::fs::read_to_string(&path).expect("read std_bytes.px");
  let expected = std::fs::read_to_string(path.with_extension("expected.txt"))
    .expect("read std_bytes.expected.txt");

  assert_eq!(run(&src), expected);
}
