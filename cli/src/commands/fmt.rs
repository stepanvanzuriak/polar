use super::render;
use crate::{
  CliError, Ctx,
  files::{decode, discover, write_atomic_with},
  guard, plural,
  project::Target,
};
use polar_compiler::shared::source::SourceFile;
use std::fs;

pub(crate) fn fmt(
  ctx: &mut Ctx<'_, '_>,
  targets: &[Target],
  check: bool,
) -> Result<u8, CliError> {
  let mut inputs = Vec::new();

  for target in targets {
    inputs.extend(discover(&target.paths, &ctx.io.cwd, Some(&target.out))?);
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
