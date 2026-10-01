mod common;

use std::process::Command;

use common::node::node_binary;
use polar_compiler::backend::codegen::builtins::BUILTINS;

const LIST_MEMBERS: &str = r#"
import { pathToFileURL } from "node:url";
const rt = await import(pathToFileURL(process.argv[1]).href);
const forGeneratedCode = new Set(["bridge", "http"]);
for (const [module, value] of Object.entries(rt)) {
  if (typeof value !== "object" || value === null) continue;
  if (forGeneratedCode.has(module)) continue;
  for (const [member, f] of Object.entries(value)) {
    console.log(`${module} ${member} ${f.length}`);
  }
}
"#;

fn runtime_members() -> Vec<(String, String, usize)> {
  let runtime = concat!(env!("CARGO_MANIFEST_DIR"), "/../runtime/runtime.js");
  let out = Command::new(node_binary())
    .args(["--input-type=module", "-e", LIST_MEMBERS, runtime])
    .output()
    .expect("spawn node");

  assert!(
    out.status.success(),
    "node failed to list runtime members:\n{}",
    String::from_utf8_lossy(&out.stderr)
  );

  String::from_utf8(out.stdout)
    .expect("node output is UTF-8")
    .lines()
    .map(|line| {
      let mut parts = line.split(' ');
      let (Some(module), Some(member), Some(arity), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
      else {
        panic!("malformed line from node: {line:?}");
      };

      (module.to_owned(), member.to_owned(), arity.parse().expect("arity"))
    })
    .collect()
}

fn find<'a>(
  members: &'a [(String, String, usize)],
  module: &str,
  member: &str,
) -> Option<&'a (String, String, usize)> {
  members.iter().find(|(m, name, _)| m == module && name == member)
}

#[test]
fn every_builtin_exists() {
  let members = runtime_members();
  let missing: Vec<String> = BUILTINS
    .iter()
    .filter(|b| find(&members, b.module, b.member).is_none())
    .map(|b| format!("{}.{}", b.module, b.member))
    .collect();

  assert!(missing.is_empty(), "in BUILTINS but not in runtime.js: {missing:?}");
}

#[test]
fn arities_match() {
  let members = runtime_members();
  let mismatched: Vec<String> = BUILTINS
    .iter()
    .filter_map(|b| {
      let (_, _, arity) = find(&members, b.module, b.member)?;

      (*arity != b.arity).then(|| {
        format!(
          "{}.{}: BUILTINS says {}, runtime.js says {arity}",
          b.module, b.member, b.arity
        )
      })
    })
    .collect();

  assert!(mismatched.is_empty(), "arity mismatch: {mismatched:#?}");
}

#[test]
fn no_undeclared_members() {
  let undeclared: Vec<String> = runtime_members()
    .into_iter()
    .filter(|(module, member, _)| {
      !BUILTINS.iter().any(|b| b.module == module && b.member == member)
    })
    .map(|(module, member, _)| format!("{module}.{member}"))
    .collect();

  assert!(
    undeclared.is_empty(),
    "in runtime.js but not in BUILTINS: {undeclared:?}"
  );
}
