mod common;

use std::path::PathBuf;

fn fixture() -> PathBuf {
  PathBuf::from(env!("CARGO_MANIFEST_DIR"))
    .join("tests/fixtures/programs/std_assert.px")
}

fn run(src: &str) -> String {
  common::node::run_program(src, "test.px", &[])
    .unwrap_or_else(|err| panic!("{err}"))
}

#[test]
fn std_assert_program_runs_as_expected() {
  let path = fixture();
  let src = std::fs::read_to_string(&path).expect("read std_assert.px");
  let expected = std::fs::read_to_string(path.with_extension("expected.txt"))
    .expect("read std_assert.expected.txt");

  assert_eq!(run(&src), expected);
}
