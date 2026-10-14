use polar_compiler::stdlib::MODULES;
use polar_rt::bindings::lookup;

fn quoted_after<'a>(text: &'a str, marker: &str) -> Vec<&'a str> {
  text
    .match_indices(marker)
    .filter_map(|(at, _)| {
      let rest = &text[at + marker.len()..];

      rest.find('"').map(|end| &rest[..end])
    })
    .collect()
}

fn effect_ops<'a>(source: &'a str, effect: &str) -> Vec<&'a str> {
  let mut lines = source.lines().skip_while(|line| {
    let line = line.trim_end();

    !(line.starts_with(&format!("  {effect} ")) && line.ends_with('{'))
  });

  lines.next();
  lines
    .take_while(|line| line.trim_end() != "  }")
    .filter_map(|line| line.trim().split_once('(').map(|(name, _)| name))
    .collect()
}

#[test]
fn every_std_rs_path_has_a_native_function() {
  let mut missing = Vec::new();

  for module in MODULES {
    for path in quoted_after(module.source, "rs \"") {
      if path.contains("::") {
        if lookup(path).is_none() {
          missing.push(format!("{}: {path}", module.name));
        }

        continue;
      }

      for line in module.source.lines() {
        let Some(effect) = line
          .trim()
          .strip_suffix(&format!("in Native = rs \"{path}\""))
          .map(str::trim)
        else {
          continue;
        };
        let ops = effect_ops(module.source, effect);

        assert!(!ops.is_empty(), "{}: no operations for {effect}", module.name);

        for op in ops {
          let key = format!("{path}::{op}");

          if lookup(&key).is_none() {
            missing.push(format!("{}: {effect}.{op} -> {key}", module.name));
          }
        }
      }
    }
  }

  assert!(
    missing.is_empty(),
    "no native function for:\n{}",
    missing.join("\n")
  );
}
