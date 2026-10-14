use polar_compiler::{
  CompileOptions, compile_core,
  core::{
    dump::dump_module,
    ir::CModule,
    serial::{from_bytes, to_bytes},
  },
};
use std::path::PathBuf;

fn fixture_programs() -> Vec<PathBuf> {
  let dir =
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/programs");
  let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)
    .unwrap()
    .map(|e| e.unwrap().path())
    .filter(|p| p.extension().is_some_and(|e| e == "px"))
    .collect();

  paths.sort();

  paths
}

fn lower_to_core(path: &PathBuf) -> Option<CModule> {
  let src = std::fs::read_to_string(path).unwrap();
  let name = path.file_name().unwrap().to_str().unwrap();

  compile_core(&src, name, &CompileOptions::default()).module
}

#[test]
fn every_fixture_round_trips() {
  let mut seen = 0;

  for path in fixture_programs() {
    let Some(module) = lower_to_core(&path) else { continue };
    let bytes = to_bytes(&module);
    let back = from_bytes(&bytes).unwrap();

    assert_eq!(bytes, to_bytes(&back), "{}", path.display());
    assert_eq!(dump_module(&module), dump_module(&back), "{}", path.display());

    seen += 1;
  }

  assert!(seen > 5, "only {seen} fixtures lowered");
}

#[test]
fn bad_magic_is_an_error() {
  assert!(from_bytes(b"not polar at all").unwrap_err().contains("magic"));
  assert!(from_bytes(b"").is_err());
}

#[test]
fn future_version_is_an_error() {
  let mut bytes = to_bytes(&CModule::default());

  bytes[7..11].copy_from_slice(&99u32.to_le_bytes());

  assert!(from_bytes(&bytes).unwrap_err().contains("newer"));
}

#[test]
fn types_and_effects_are_dropped() {
  let path =
    fixture_programs().into_iter().find(|p| p.ends_with("hello.px")).unwrap();
  let module = lower_to_core(&path).unwrap();
  let back = from_bytes(&to_bytes(&module)).unwrap();

  assert!(back.decls.iter().all(|d| d.effects.is_none()));
}

#[test]
fn pack_unpack_round_trips() {
  use polar_compiler::core::serial::{pack, unpack};

  for payload in [vec![], vec![7u8; 1000], (0..=255u8).collect::<Vec<_>>()] {
    let exe = pack(b"\x7fELF-not-really", &payload);

    assert_eq!(unpack(&exe), Some(payload.as_slice()));
  }
}

#[test]
fn unpack_of_a_plain_file_is_none() {
  use polar_compiler::core::serial::unpack;

  assert_eq!(unpack(b""), None);
  assert_eq!(unpack(b"just an executable, no payload at all"), None);
}
