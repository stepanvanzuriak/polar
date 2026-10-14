use polar_compiler::{CompileOptions, Target, compile, compile_core, format};

fn program(extern_target: &str) -> String {
  format!(
    "externs\n  now() -> Int\n    = {extern_target}\n\nfunctions\n  main() {{\n    Log.info(Int.to_string(now()))\n  }}\n\nexports\n  main\n"
  )
}

fn codes(src: &str, target: Target) -> Vec<String> {
  let options = CompileOptions { target, ..CompileOptions::default() };

  compile(src, "t.px", &options)
    .diagnostics
    .iter()
    .map(|d| d.message.clone())
    .collect()
}

#[test]
fn legacy_form_means_js() {
  assert!(codes(&program("\"./t.js\" now"), Target::Js).is_empty());
}

#[test]
fn explicit_js_form() {
  assert!(codes(&program("js \"./t.js\" now"), Target::Js).is_empty());
}

#[test]
fn two_entry_form_builds_on_both_targets() {
  let src = program("js \"./t.js\" now, rs \"time::now\"");

  assert!(codes(&src, Target::Js).is_empty());
  assert!(codes(&src, Target::Native).is_empty());
}

#[test]
fn native_build_without_rs_entry_errors() {
  let messages = codes(&program("\"./t.js\" now"), Target::Native);

  assert_eq!(messages.len(), 1, "{messages:?}");
  assert!(messages[0].contains("`now`") && messages[0].contains("`rs`"));
}

#[test]
fn js_build_without_js_entry_errors() {
  let messages = codes(&program("rs \"time::now\""), Target::Js);

  assert_eq!(messages.len(), 1, "{messages:?}");
  assert!(messages[0].contains("`js`"));
}

#[test]
fn unknown_language_lists_the_valid_ones() {
  let out =
    compile(&program("py \"x\" now"), "t.px", &CompileOptions::default());

  assert!(
    out.diagnostics.iter().any(|d| {
      d.message.contains("unknown implementation language `py`")
        && d.help.iter().any(|h| h.contains("`js` and `rs`"))
    }),
    "{:?}",
    out.diagnostics
  );
}

#[test]
fn rs_entry_reaches_core() {
  let src = program("js \"./t.js\" now, rs \"time::now\"");
  let out = compile_core(
    &src,
    "t.px",
    &CompileOptions { target: Target::Native, ..CompileOptions::default() },
  );
  let module = out.module.unwrap();

  assert_eq!(module.externs[0].rs.as_deref(), Some("time::now"));
}

#[test]
fn formatter_round_trips_both_spellings() {
  for target in [
    "\"./t.js\" now",
    "js \"./t.js\" now, rs \"time::now\"",
    "rs \"time::now\"",
  ] {
    let once = format(&program(target), "t.px").output.unwrap();
    let twice = format(&once, "t.px").output.unwrap();

    assert_eq!(once, twice, "{target}");
    assert_eq!(
      once.contains("rs \"time::now\""),
      target.contains("rs "),
      "{once}"
    );
  }
}

#[test]
fn native_bind_attaches_rs_to_node_bind_ops() {
  let src = "hosts\n  Node\n\neffects\n  Clock in Node {\n    tick() -> Int\n  }\n\nbinds\n  Clock in Node = \"./clock.js\"\n\n  Clock in Native = rs \"clock\"\n\nfunctions\n  main() -> {} / {Clock} {\n    Log.info(Int.to_string(Clock.tick()))\n  }\n\nexports\n  main\n";
  let out = compile_core(
    src,
    "t.px",
    &CompileOptions { target: Target::Native, ..CompileOptions::default() },
  );

  assert!(out.diagnostics.is_empty(), "{:?}", out.diagnostics);

  let module = out.module.unwrap();

  assert!(
    module.externs.iter().any(|e| e.rs.as_deref() == Some("clock::tick"))
  );
}

#[test]
fn native_bind_with_a_body_is_rejected() {
  let src = "hosts\n  Node\n\neffects\n  Clock in Node {\n    tick() -> Int\n  }\n\nbinds\n  Clock in Native {\n    tick() { 1 }\n  }\n\nexports\n  Clock\n";
  let messages = codes(src, Target::Js);

  assert!(messages.iter().any(|m| m.contains("rs \"name\"")), "{messages:?}");
}
