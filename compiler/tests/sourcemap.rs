use polar_compiler::{
  CompileOptions,
  backend::js::sourcemap::{OriginalLocation, SourceMapBuilder, encode_vlq},
  compile,
  shared::source::SourceFile,
};
use proptest::prelude::*;
use std::process::Command;

mod common;

use common::node::node_binary;

fn vlq(n: i64) -> String {
  let mut out = String::new();

  encode_vlq(n, &mut out);
  out
}

#[test]
fn vlq_values() {
  assert_eq!(vlq(0), "A");
  assert_eq!(vlq(1), "C");
  assert_eq!(vlq(-1), "D");
  assert_eq!(vlq(15), "e");
  assert_eq!(vlq(16), "gB");
  assert_eq!(vlq(-16), "hB");
}

fn decode(s: &str) -> i64 {
  const B64: &str =
    "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
  let (mut value, mut shift) = (0i64, 0);

  for c in s.chars() {
    let digit = i64::try_from(B64.find(c).expect("base64 digit")).unwrap();

    value |= (digit & 31) << shift;
    shift += 5;
  }

  if value & 1 == 1 { -(value >> 1) } else { value >> 1 }
}

proptest! {
  #[test]
  fn vlq_round_trip(n in -(1i64 << 31)..(1i64 << 31)) {
    prop_assert_eq!(decode(&vlq(n)), n);
  }
}

fn find_entries(map: &str, points: &[(usize, usize)]) -> Vec<String> {
  let script = format!(
    r#"const {{ SourceMap }} = require("node:module");
const map = new SourceMap({map});
for (const [l, c] of {points}) {{
  const e = map.findEntry(l, c);
  console.log(e && e.originalLine !== undefined
    ? `${{e.originalLine}}:${{e.originalColumn}} ${{e.name ?? ""}}`.trim()
    : "none");
}}"#,
    points = format!("{points:?}").replace('(', "[").replace(')', "]"),
  );
  let out = Command::new(node_binary())
    .arg("-e")
    .arg(script)
    .output()
    .expect("run node");

  assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));

  String::from_utf8(out.stdout).unwrap().lines().map(str::to_string).collect()
}

#[test]
fn mappings_resolve_through_node() {
  let file = SourceFile::new("a.px", "x = 1\n😀 yz\n");
  let mut b = SourceMapBuilder::default();
  let at = |offset, name| Some(OriginalLocation { file: &file, offset, name });

  b.add_mapping(0, 0, at(0, None));
  b.add_mapping(0, 4, at(4, Some("x")));
  b.add_mapping(0, 4, at(0, None));
  b.add_mapping(0, 8, None);
  b.add_mapping(2, 2, at(11, None));

  let map = b.build().to_json();
  let found = find_entries(&map, &[(0, 0), (0, 4), (0, 8), (2, 2)]);

  assert_eq!(found[..3], ["0:0", "0:4 x", "none"], "{map}");
  assert!(found[3].starts_with("1:3"), "{map}: {found:?}");
  assert!(map.contains("\"mappings\":\"AAAA,IAAIA,I;;EACD\""), "{map}");
}

#[test]
fn sources_content_round_trips() {
  let src = "functions\n  main() {\n    Log.info(\"héllo 😀\")\n  }\n\nexports\n  main\n";
  let out = compile(src, "hello.px", &CompileOptions::default());
  let script = format!(
    "const m = {}; process.stdout.write(m.sourcesContent[0]);",
    out.sourcemap
  );
  let node =
    Command::new(node_binary()).arg("-e").arg(script).output().unwrap();

  assert_eq!(String::from_utf8(node.stdout).unwrap(), src);
}

#[test]
fn compile_maps_the_main_function() {
  let src =
    "functions\n  main() {\n    Log.info(\"x\")\n  }\n\nexports\n  main\n";
  let out = compile(src, "hello.px", &CompileOptions::default());

  assert!(
    out.sourcemap.contains("\"sources\":[\"hello.px\"]"),
    "{}",
    out.sourcemap
  );
  assert!(out.sourcemap.contains("\"mappings\":\""));
  assert!(!out.sourcemap.contains("\"mappings\":\"\""), "{}", out.sourcemap);
}

#[test]
fn inlined_std_code_maps_to_call_site() {
  let src = "uses\n  Std.Option\n\nfunctions\n  inc(n: Option<Int>) -> Option<Int> {\n    Option.map(n, function(x) { x + 1 })\n  }\n\nexports\n  inc\n";
  let out = compile(src, "inline.px", &CompileOptions::default());

  assert!(out.diagnostics.is_empty(), "{:?}", out.diagnostics);
  assert!(!out.js.contains("$Option.map"), "{}", out.js);

  let lines: Vec<&str> = out.js.lines().collect();
  let start = lines
    .iter()
    .position(|l| l.starts_with("export function inc("))
    .expect("inc is emitted");
  let end =
    start + lines[start..].iter().position(|l| *l == "}").expect("inc ends");
  let points: Vec<(usize, usize)> = (start + 1..end)
    .map(|line| (line, lines[line].len() - lines[line].trim_start().len()))
    .collect();

  let entries = find_entries(&out.sourcemap, &points);

  assert!(entries.iter().any(|e| e != "none"), "{entries:?}");

  for entry in entries.iter().filter(|e| *e != "none") {
    assert!(
      entry.starts_with("5:"),
      "{entry} in {points:?}\n{}\n{}",
      out.js,
      out.sourcemap
    );
  }
}
