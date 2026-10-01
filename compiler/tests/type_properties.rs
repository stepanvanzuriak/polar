mod common;

use common::{
  effectful,
  node::{run_main, run_program_with, runtime_url},
  typed::{MUTATIONS, Program, arguments, program},
};
use polar_compiler::{CompileOptions, Stage, compile, dump_stage};
use proptest::{
  prelude::*,
  strategy::ValueTree,
  test_runner::{Config, FileFailurePersistence, TestRunner},
};
use std::collections::BTreeSet;

fn cases() -> u32 {
  std::env::var("PROPTEST_CASES")
    .ok()
    .and_then(|n| n.parse().ok())
    .unwrap_or(256)
}

fn node_cases() -> u32 {
  std::env::var("PROPTEST_CASES")
    .ok()
    .and_then(|n| n.parse::<u32>().ok())
    .map_or(32, |n| (n / 32).max(1))
}

fn config(cases: u32) -> Config {
  Config {
    cases,
    failure_persistence: Some(Box::new(FileFailurePersistence::WithSource(
      "proptest-regressions",
    ))),
    ..Config::default()
  }
}

fn codes(src: &str) -> Vec<String> {
  dump_stage(src, "gen.px", Stage::Types)
    .diagnostics
    .iter()
    .map(|d| d.code.to_string())
    .collect()
}

fn samples() -> Vec<Program> {
  let mut runner = TestRunner::deterministic();
  let strategy = program();

  (0..100)
    .map(|_| strategy.new_tree(&mut runner).expect("generate").current())
    .collect()
}

#[test]
fn generator_covers_all_kinds() {
  let seen: BTreeSet<&str> =
    samples().iter().flat_map(Program::kinds).collect();
  let expected = [
    "int",
    "float",
    "bool",
    "string",
    "var",
    "arith_int",
    "arith_float",
    "compare",
    "equality",
    "logic",
    "if",
    "let",
    "record",
    "field",
    "update",
    "lambda",
    "call",
    "list",
    "builtin",
    "interp",
  ];
  let missing: Vec<&str> =
    expected.iter().copied().filter(|k| !seen.contains(k)).collect();

  assert!(missing.is_empty(), "never generated: {missing:?}");
}

#[test]
fn generator_finds_mutation_sites() {
  let samples = samples();

  for mutation in MUTATIONS {
    assert!(
      samples.iter().any(|p| p.sites(mutation) > 0),
      "no sample has a site for {mutation:?}"
    );
  }
}

proptest! {
  #![proptest_config(config(cases()))]

  #[test]
  fn p1_well_typed_programs_check(program in program()) {
    let src = program.source();
    let result = dump_stage(&src, "gen.px", Stage::Types);

    prop_assert!(
      result.diagnostics.is_empty(),
      "{src}\n{:#?}",
      result.diagnostics
    );

    let expected = format!("gen : {}", program.expected_type());
    let output = result.output.unwrap_or_default();

    prop_assert!(output.lines().any(|l| l == expected), "{src}\n{output}\nwanted {expected}");
  }

  #[test]
  fn p2_mutations_are_rejected(program in program(), seed in any::<usize>()) {
    for mutation in MUTATIONS {
      let sites = program.sites(mutation);

      if sites == 0 {
        continue;
      }

      let src = program.mutated(mutation, seed % sites);
      let found = codes(&src);

      prop_assert!(
        mutation.expected().iter().any(|c| found.iter().any(|f| f == c)),
        "{mutation:?} should give {:?}, got {found:?}\n{src}",
        mutation.expected()
      );
    }
  }
}

proptest! {
  #![proptest_config(config(node_cases()))]

  #[test]
  fn p3_well_typed_programs_run(
    (program, args) in program().prop_flat_map(|p| {
      let args = arguments(&p);

      (Just(p), args)
    })
  ) {
    let src = program.with_main(&args);
    let out = compile(
      &src,
      "gen.px",
      &CompileOptions { runtime: runtime_url(), ..CompileOptions::default() },
    );

    prop_assert!(out.diagnostics.is_empty(), "{src}\n{:#?}", out.diagnostics);

    match run_main(&out.js) {
      Ok(_) => {}
      Err(stderr) => prop_assert!(false, "{src}\n{}\n{stderr}", out.js),
    }
  }
}

fn types_of(src: &str) -> (String, Vec<String>) {
  let result = dump_stage(src, "gen.px", Stage::Types);
  let codes = result.diagnostics.iter().map(|d| d.code.to_string()).collect();

  (result.output.unwrap_or_default(), codes)
}

proptest! {
  #![proptest_config(config(cases()))]

  #[test]
  fn p4_effectful_programs_check(program in effectful::program()) {
    let src = program.source();
    let (output, codes) = types_of(&src);

    prop_assert!(codes.is_empty(), "{src}\n{codes:?}");

    for i in 0..program.fns.len() {
      let expected = format!("g{i} : {}", program.expected_type(i));

      prop_assert!(output.lines().any(|l| l == expected), "{src}\n{output}\nwanted {expected}");
    }
  }

  #[test]
  fn p5_dropped_label_is_rejected(program in effectful::program(), seed in any::<usize>()) {
    let sites = program.drop_label_sites();

    if !sites.is_empty() {
      let src = program.without_label(sites[seed % sites.len()]);
      let (_, codes) = types_of(&src);

      prop_assert!(codes.iter().any(|c| c == "POLAR0808"), "{src}\n{codes:?}");
    }
  }

  #[test]
  fn p6_disjoint_hosts_are_rejected(program in effectful::program(), seed in any::<usize>()) {
    let src = program.with_disjoint_call(seed % program.fns.len());
    let (_, codes) = types_of(&src);

    prop_assert!(codes.iter().any(|c| c == "POLAR0811"), "{src}\n{codes:?}");
  }
}

proptest! {
  #![proptest_config(config(node_cases()))]

  #[test]
  fn p7_effectful_programs_run(program in effectful::program()) {
    let src = program.source();
    let expected = program.expected_output();
    let hosts = program.hosts();

    prop_assert!(!hosts.is_empty(), "{src}");

    for host in hosts {
      match run_program_with(&src, "gen.px", &[], Some(host), &[]) {
        Ok(out) => prop_assert_eq!(&out, &expected, "{} on {}", src, host),
        Err(err) => prop_assert!(false, "{src}\non {host}: {err}"),
      }
    }
  }
}

#[test]
fn effectful_generator_covers() {
  let mut runner = TestRunner::deterministic();
  let strategy = effectful::program();
  let samples: Vec<effectful::Program> = (0..100)
    .map(|_| strategy.new_tree(&mut runner).expect("generate").current())
    .collect();
  let fns = || samples.iter().flat_map(|p| &p.fns);

  for label in 0..3 {
    assert!(
      fns().any(|f| f.row.contains(&label)),
      "no function uses effect {label}"
    );
  }

  assert!(fns().any(|f| f.declared && !f.row.is_empty()));
  assert!(fns().any(|f| !f.declared && !f.row.is_empty()));
  assert!(fns().any(|f| f.row.is_empty()));

  for hosts in [vec!["H1"], vec!["H2"], vec!["H1", "H2"]] {
    assert!(
      samples.iter().any(|p| p.hosts() == hosts),
      "no program runs on {hosts:?}"
    );
  }
}
