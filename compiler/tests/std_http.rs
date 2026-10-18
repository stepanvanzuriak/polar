mod common;

use std::path::PathBuf;

#[test]
fn std_http_multipart_program_runs_as_expected() {
  let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
    .join("tests/fixtures/programs/std_http_multipart.px");
  let src = std::fs::read_to_string(&path).expect("read std_http_multipart.px");
  let expected = std::fs::read_to_string(path.with_extension("expected.txt"))
    .expect("read std_http_multipart.expected.txt");
  let actual = common::node::run_program(&src, "test.px", &[])
    .unwrap_or_else(|err| panic!("{err}"));

  assert_eq!(actual, expected);
}
