use super::{Compiled, Plan, compile_imports, compile_one, render_compiled};
use crate::{CliError, Ctx, files::discover, project::Target};

pub(crate) fn check(
  ctx: &mut Ctx<'_, '_>,
  targets: &[Target],
) -> Result<u8, CliError> {
  let mut compiled = Vec::new();
  let plan = Plan::checking();

  for target in targets {
    for input in discover(&target.paths, &ctx.io.cwd, Some(&target.out))? {
      compiled.push(compile_one(
        ctx,
        &input,
        "./_polar/runtime.js".to_string(),
        &plan,
      )?);
    }
  }

  compile_imports(
    ctx.compiler,
    &mut compiled,
    |_| "./_polar/runtime.js".to_string(),
    &plan,
  )?;

  render_compiled(ctx, &compiled);

  Ok(u8::from(compiled.iter().any(Compiled::has_errors)))
}
