use crate::{CliError, Ctx};
use polar_compiler::syntax::lexer::token;
use std::collections::BTreeMap;
use std::fmt::Write;
use std::path::Path;

fn is_identifier(text: &str) -> bool {
  let mut chars = text.chars();

  match chars.next() {
    Some(first) if first.is_ascii_lowercase() => {
      chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    }
    _ => false,
  }
}

fn project_name(name: Option<&str>, root: &Path) -> Result<String, CliError> {
  let resolved_name = if let Some(explicit_name) = name {
    explicit_name.to_string()
  } else {
    let folder_str =
      root.file_name().and_then(|n| n.to_str()).ok_or_else(|| {
        CliError::Message(
          "could not infer project name from directory path; pass `--name`"
            .to_string(),
        )
      })?;

    folder_str.to_lowercase().replace(['-', ' '], "_")
  };

  if !is_identifier(&resolved_name) {
    return Err(CliError::Message(format!(
      "`{resolved_name}` isn't a valid project name; use lowercase letters, digits and `_`, or pass `--name`"
    )));
  }

  Ok(resolved_name)
}

const MAIN: &str = "module Main\n\nfunctions\n  main() {\n    Log.info(\"hello, world\")\n  }\n\nexports\n  main\n";
const GITIGNORE: &str = "dist/\n.polar/\n";

fn validate_zones(zones: &[String]) -> Result<(), CliError> {
  for (index, zone) in zones.iter().enumerate() {
    if !is_identifier(zone) {
      return Err(CliError::Message(format!(
        "`{zone}` isn't a valid zone keyword; use lowercase letters, digits and `_`"
      )));
    }

    if token::keyword(zone).is_some() {
      return Err(CliError::Message(format!(
        "`{zone}` is already a Polar keyword; choose another zone keyword"
      )));
    }

    if zones[..index].contains(zone) {
      return Err(CliError::Message(format!(
        "the zone `{zone}` is given twice"
      )));
    }
  }

  Ok(())
}

pub(crate) fn init(
  ctx: &mut Ctx<'_, '_>,
  dir: Option<&Path>,
  name: Option<&str>,
  zones: &[String],
) -> Result<u8, CliError> {
  let root = match dir {
    Some(dir) => ctx.io.cwd.join(dir),
    None => ctx.io.cwd.clone(),
  };

  let config = root.join(crate::project::CONFIG);

  if config.is_file() {
    return Err(CliError::Message(format!(
      "{} already exists; `polar init` doesn't overwrite a project",
      config.display()
    )));
  }

  let resolved_name = project_name(name, &root)?;

  validate_zones(zones)?;

  let mut toml = format!("[project]\nname = \"{resolved_name}\"\n");

  if !zones.is_empty() {
    let listed: Vec<String> = zones.iter().map(|z| format!("{z:?}")).collect();

    let _ = write!(toml, "\n[plugin]\nzones = [{}]\n", listed.join(", "));
  }

  let files = [
    ("polar.toml", toml.as_str()),
    ("src/main.px", MAIN),
    (".gitignore", GITIGNORE),
  ];

  let mut kept: Vec<&str> = Vec::new();
  let mut created: Vec<&str> = Vec::new();

  for (rel_path, content) in files {
    let target = root.join(rel_path);

    if target.is_file() {
      kept.push(rel_path);
    } else {
      crate::files::write_atomic(&target, content.as_bytes())
        .map_err(|e| CliError::write(target.display().to_string(), &e))?;
      created.push(rel_path);
    }
  }

  if !zones.is_empty() {
    if crate::plugin::write_files(
      &root,
      &resolved_name,
      zones,
      &BTreeMap::new(),
    )? {
      created.push("plugin/lib.rs");
    } else {
      kept.push("plugin/lib.rs");
    }

    created.push(".polar/");
  }

  let header_dir = match dir {
    Some(d) => d.display().to_string(),
    None => ".".to_string(),
  };

  let _ = writeln!(ctx.io.out, "created {header_dir}/");
  for file in &created {
    let _ = writeln!(ctx.io.out, "  {file}");
  }

  for file in &kept {
    let _ = writeln!(ctx.io.out, "  kept {file}");
  }

  let cd_prefix = match dir {
    Some(d) => format!("cd {} && ", d.display()),
    None => String::new(),
  };

  let _ = writeln!(ctx.io.out, "next: {cd_prefix}polar run");

  Ok(0)
}
