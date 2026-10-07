use polar_cli::pkg::{Version, cache_key, sha256_hex, url};
use std::path::PathBuf;

#[test]
fn sha256_known_answers() {
  assert_eq!(
    sha256_hex(b""),
    "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
  );
  assert_eq!(
    sha256_hex(b"abc"),
    "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
  );
}

#[test]
fn versions_order_numerically() {
  assert!(Version::parse("v1.10.0") > Version::parse("v1.2.0"));
  assert_eq!(Version::parse("1.0.0"), None);
  assert_eq!(Version::parse("v1.0"), None);
}

#[test]
fn addresses_expand() {
  assert_eq!(url("github.com/a/b"), "https://github.com/a/b.git");
  assert_eq!(url("file:///x/y"), "file:///x/y");
  assert_eq!(cache_key("github.com/a/b"), PathBuf::from("github.com/a/b"));
}

#[test]
fn releases_compare_by_major_and_minor() {
  use polar_cli::pkg::Release;

  let r = |t| Release::parse(t).unwrap();

  assert!(r("0.0.9").older_than(r("0.1.0")));
  assert!(r("0.1.0").older_than(r("1.0.0")));
  assert!(!r("0.1.0").older_than(r("0.1.5")));
  assert!(!r("0.2.0").older_than(r("0.1.0")));
  assert_eq!(r("0.1.0-rc1"), r("0.1.0"));
  assert!(Release::parse("0.1").is_none());
  assert!(Release::parse("v0.1.0").is_none());
}
