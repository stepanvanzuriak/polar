use super::render;
use crate::{
  CliError, Ctx,
  files::{Input, decode, discover, normalize, write_atomic_with},
  guard, plural,
  project::Target,
};
use polar_compiler::shared::source::SourceFile;
use std::{
  fmt::Write as _,
  fs,
  path::{Component, Path},
};

pub(crate) struct Options<'a> {
  pub check: bool,
  pub idempotent: bool,
  pub expect: Option<&'a Path>,
}

fn wild(pattern: &str, text: &str) -> bool {
  match pattern.split_once('*') {
    None => pattern == text,
    Some((head, tail)) => text.strip_prefix(head).is_some_and(|rest| {
      (0..=rest.len())
        .filter(|i| rest.is_char_boundary(*i))
        .any(|i| wild(tail, &rest[i..]))
    }),
  }
}

fn under(pattern: &Path, path: &Path) -> bool {
  let mut parts = path.components();

  pattern.components().all(|want| match (want, parts.next()) {
    (Component::Normal(want), Some(Component::Normal(have))) => {
      wild(&want.to_string_lossy(), &have.to_string_lossy())
    }
    (want, Some(have)) => want == have,
    (_, None) => false,
  })
}

fn skipped(input: &Input, target: &Target, cwd: &Path) -> bool {
  let named =
    target.paths.iter().any(|path| normalize(&cwd.join(path)) == input.path);

  if named {
    return false;
  }

  let name = input.path.file_name().map(|n| n.to_string_lossy().into_owned());
  let expected = name.as_deref().is_some_and(|n| n.ends_with(".expected.px"));
  let has_expected = input.path.extension().is_some()
    && input.path.with_extension("expected.px").is_file();

  expected
    || has_expected
    || target
      .fmt_exclude
      .iter()
      .any(|pattern| under(&normalize(&cwd.join(pattern)), &input.path))
}

fn line_diff(expected: &str, actual: &str) -> String {
  let a: Vec<&str> = expected.split('\n').collect();
  let b: Vec<&str> = actual.split('\n').collect();
  let mut table = vec![vec![0usize; b.len() + 1]; a.len() + 1];

  for i in (0..a.len()).rev() {
    for j in (0..b.len()).rev() {
      table[i][j] = if a[i] == b[j] {
        table[i + 1][j + 1] + 1
      } else {
        table[i + 1][j].max(table[i][j + 1])
      };
    }
  }

  let (mut i, mut j) = (0, 0);
  let mut out = String::new();

  while i < a.len() || j < b.len() {
    if i < a.len() && j < b.len() && a[i] == b[j] {
      i += 1;
      j += 1;
    } else if j >= b.len()
      || (i < a.len() && table[i + 1][j] >= table[i][j + 1])
    {
      let _ = writeln!(out, "- {}: {}", i + 1, a[i]);
      i += 1;
    } else {
      let _ = writeln!(out, "+ {}: {}", j + 1, b[j]);
      j += 1;
    }
  }

  out
}

pub(crate) fn fmt(
  ctx: &mut Ctx<'_, '_>,
  targets: &[Target],
  options: &Options<'_>,
) -> Result<u8, CliError> {
  let cwd = ctx.io.cwd.clone();
  let (check, idempotent) = (options.check, options.idempotent);
  let mut inputs = Vec::new();

  for target in targets {
    inputs.extend(
      discover(&target.paths, &cwd, Some(&target.out))?
        .into_iter()
        .filter(|input| !skipped(input, target, &cwd)),
    );
  }

  if let Some(expect) = options.expect {
    return expect_one(ctx, &inputs, expect);
  }

  let (mut changed, mut unchanged, mut failed) = (0, 0, 0);

  for input in &inputs {
    let bytes =
      fs::read(&input.path).map_err(|e| CliError::read(&input.display, &e))?;
    let plugins = input.plugins();
    let source = decode(&input.display, &bytes);
    let file = SourceFile::new(input.display.clone(), source.text.clone());

    if let Some(invalid) = source.invalid {
      render(ctx.io.err, ctx.color, &[(&file, &[invalid])]);
      failed += 1;
      continue;
    }

    let format = ctx.compiler.format;
    let result =
      guard(&input.display, || format(&source.text, &input.display, &plugins))
        .map_err(CliError::Ice)?;

    render(ctx.io.err, ctx.color, &[(&file, &result.diagnostics)]);

    let Some(output) = result.output else {
      failed += 1;
      continue;
    };

    if output.as_bytes() == bytes.as_slice() {
      unchanged += 1;
      continue;
    }

    if idempotent {
      let again =
        guard(&input.display, || format(&output, &input.display, &plugins))
          .map_err(CliError::Ice)?;

      if again.output.as_deref().is_some_and(|text| *text != output) {
        let _ = writeln!(
          ctx.io.err,
          "error: formatting {} is not idempotent: a second pass changes it",
          input.display
        );
        failed += 1;
        continue;
      }
    }

    changed += 1;

    if check {
      let _ = writeln!(ctx.io.out, "would reformat: {}", input.display);
      continue;
    }

    let permissions = fs::metadata(&input.path).map(|m| m.permissions()).ok();

    write_atomic_with(
      &input.path,
      output.as_bytes(),
      |temp| match permissions {
        Some(permissions) => fs::set_permissions(temp, permissions),
        None => Ok(()),
      },
    )
    .map_err(|e| CliError::write(&input.display, &e))?;
  }

  let verb = if check { "would be reformatted" } else { "reformatted" };

  let _ = writeln!(
    ctx.io.err,
    "{} {verb}, {unchanged} unchanged, {failed} failed to parse",
    plural(changed, "file")
  );

  Ok(u8::from(failed > 0 || (check && changed > 0)))
}

fn expect_one(
  ctx: &mut Ctx<'_, '_>,
  inputs: &[Input],
  expect: &Path,
) -> Result<u8, CliError> {
  let [input] = inputs else {
    return Err(CliError::Message(
      "`polar fmt --expect FILE` formats exactly one file: `polar fmt --expect \
       FILE PATH`"
        .to_string(),
    ));
  };
  let bytes =
    fs::read(&input.path).map_err(|e| CliError::read(&input.display, &e))?;
  let source = decode(&input.display, &bytes);
  let file = SourceFile::new(input.display.clone(), source.text.clone());

  if let Some(invalid) = source.invalid {
    render(ctx.io.err, ctx.color, &[(&file, &[invalid])]);
    return Ok(1);
  }

  let plugins = input.plugins();
  let format = ctx.compiler.format;
  let result =
    guard(&input.display, || format(&source.text, &input.display, &plugins))
      .map_err(CliError::Ice)?;

  render(ctx.io.err, ctx.color, &[(&file, &result.diagnostics)]);

  let Some(output) = result.output else { return Ok(1) };
  let shown = expect.display().to_string();
  let wanted = fs::read_to_string(ctx.io.cwd.join(expect))
    .map_err(|e| CliError::read(&shown, &e))?;

  if wanted == output {
    return Ok(0);
  }

  let _ = write!(
    ctx.io.err,
    "error: formatting {} differs from {shown} (- expected, + actual)\n{}",
    input.display,
    line_diff(&wanted, &output)
  );

  Ok(1)
}
