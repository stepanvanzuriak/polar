mod common;

use common::{
  effectful,
  node::runtime_url,
  typed::{arguments, program},
};
use polar_compiler::{CompileOptions, HostOption, compile};
use proptest::{prelude::*, test_runner::Config};

fn cases() -> u32 {
  std::env::var("PROPTEST_CASES")
    .ok()
    .and_then(|n| n.parse().ok())
    .unwrap_or(256)
}

fn js(src: &str, host: Option<&str>) -> String {
  let out = compile(
    src,
    "gen.px",
    &CompileOptions {
      runtime: runtime_url(),
      host: host
        .map_or(HostOption::Auto, |h| HostOption::Fixed(Some(h.to_string()))),
      ..CompileOptions::default()
    },
  );

  assert!(out.diagnostics.is_empty(), "{src}\n{:#?}", out.diagnostics);
  out.js
}

fn function<'a>(js: &'a str, name: &str) -> &'a str {
  let start = js
    .find(&format!("function {name}("))
    .unwrap_or_else(|| panic!("no function {name}\n{js}"));
  let header = js[..start].rfind('\n').map_or(0, |i| i + 1);
  let end = js[start..].find("\n}\n").map_or(js.len(), |i| start + i + 3);

  &js[header..end]
}

proptest! {
  #![proptest_config(Config { cases: cases(), ..Config::default() })]

  #[test]
  fn a1_pure_programs_never_await(
    (program, args) in program().prop_flat_map(|p| {
      let args = arguments(&p);

      (Just(p), args)
    })
  ) {
    let src = program.with_main(&args);
    let js = js(&src, None);

    prop_assert!(!js.contains("await"), "{src}\n{js}");
    prop_assert!(!js.contains("async"), "{src}\n{js}");
  }

  #[test]
  fn a2_async_exactly_where_the_row_suspends(program in effectful::program()) {
    let src = program.source();

    for host in program.hosts() {
      let js = js(&src, Some(host));

      for (i, f) in program.fns.iter().enumerate() {
        let name = format!("g{i}");

        if !js.contains(&format!("function {name}(")) {
          continue;
        }

        let text = function(&js, &name);
        let suspends = !f.row.is_empty();

        prop_assert_eq!(
          text.starts_with("async function") || text.starts_with("export async function"),
          suspends,
          "{} on {}\n{}", src, host, js
        );
        prop_assert_eq!(text.contains("await"), suspends, "{} on {}\n{}", src, host, js);
      }
    }
  }

  #[test]
  fn a3_calls_to_suspending_functions_are_awaited(program in effectful::program()) {
    let src = program.source();

    for host in program.hosts() {
      let js = js(&src, Some(host));

      for (i, f) in program.fns.iter().enumerate() {
        let call = format!("g{i}(");

        for (at, _) in js.match_indices(&call) {
          let before = &js[..at];

          if before.ends_with("function ") {
            continue;
          }

          prop_assert_eq!(
            before.ends_with("await "),
            !f.row.is_empty(),
            "call to g{} in {} on {}\n{}", i, src, host, js
          );
        }
      }
    }
  }
}
