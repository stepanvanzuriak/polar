use crate::{
  CliError, Ctx,
  files::{discover, read_source},
  guard,
};
use polar_compiler::{
  CompileOptions, HostOption,
  core::serial::{bundle_to_bytes, pack},
  shared::{diagnostic::Severity, source::SourceFile},
};
use std::{
  fs,
  path::{Path, PathBuf},
  process::Command,
};

pub(crate) fn runtime_binary(
  ctx: &Ctx<'_, '_>,
  triple: Option<&str>,
) -> Result<PathBuf, CliError> {
  if let Some(explicit) = (ctx.io.env)("POLAR_RT") {
    return Ok(PathBuf::from(explicit));
  }

  let exe = std::env::current_exe().ok();
  let beside = exe.as_deref().and_then(Path::parent).map(Path::to_path_buf);
  let suffix = if cfg!(windows) { ".exe" } else { "" };

  let (dir, name) = match triple {
    Some(triple) => (
      (ctx.io.env)("POLAR_RT_DIR").map(PathBuf::from).or(beside),
      format!("polar-rt-{triple}{suffix}"),
    ),
    None => (beside, format!("polar-rt{suffix}")),
  };
  let path = dir.map(|d| d.join(&name)).filter(|p| p.is_file());

  path.ok_or_else(|| {
    CliError::Message(match triple {
      Some(triple) => format!(
        "no native runtime for `{triple}` (looked for `{name}`; set POLAR_RT_DIR)"
      ),
      None => format!(
        "cannot find the native runtime `{name}` next to `polar` (set POLAR_RT)"
      ),
    })
  })
}

fn entry(ctx: &Ctx<'_, '_>, file: Option<&Path>) -> Result<PathBuf, CliError> {
  Ok(match crate::project::run_target(file, &ctx.io.cwd)? {
    crate::project::RunTarget::Launch(project) => project.main,
    crate::project::RunTarget::File { path, .. } => path,
  })
}

fn compile(
  ctx: &mut Ctx<'_, '_>,
  file: &Path,
  host: Option<&str>,
) -> Result<Option<polar_compiler::core::serial::Bundle>, CliError> {
  let inputs = discover(&[file.to_path_buf()], &ctx.io.cwd, None)?;
  let [input] = inputs.as_slice() else {
    return Err(CliError::Message(
      "`--target native` takes a single `.px` file".to_string(),
    ));
  };
  let source = read_source(input)?;
  let host = host.map_or(HostOption::Auto, |h| HostOption::Flag(h.to_string()));

  compile_source(ctx, input, &source.text, host)
}

pub(crate) fn compile_source(
  ctx: &mut Ctx<'_, '_>,
  input: &crate::files::Input,
  text: &str,
  host: HostOption,
) -> Result<Option<polar_compiler::core::serial::Bundle>, CliError> {
  let (modules, _) = super::read_imports(input, text)?;
  let options = CompileOptions {
    modules,
    host,
    plugins: input.plugins(),
    ..CompileOptions::default()
  };
  let out = guard(&input.display, || {
    polar_compiler::compile_native(text, &input.display, &options)
  })
  .map_err(CliError::Ice)?;

  if let Some(message) = out.host_error {
    return Err(CliError::Message(message));
  }

  let mut files =
    vec![SourceFile::new(input.display.clone(), text.to_string())];

  for module in &options.modules {
    files.push(SourceFile::new(module.filename(), module.source.clone()));
  }

  for std in polar_compiler::stdlib::MODULES {
    let name = polar_compiler::stdlib::filename(std.name);

    if out.diagnostics.iter().any(|d| *d.primary.span.file == *name) {
      files.push(SourceFile::new(name, std.source.to_string()));
    }
  }

  let grouped: Vec<Vec<_>> = files
    .iter()
    .map(|f| {
      out
        .diagnostics
        .iter()
        .filter(|d| &*d.primary.span.file == f.name())
        .cloned()
        .collect()
    })
    .collect();
  let listed: Vec<_> =
    files.iter().zip(&grouped).map(|(f, d)| (f, d.as_slice())).collect();

  super::render(ctx.io.err, ctx.color, &listed);

  for d in &out.diagnostics {
    if !files.iter().any(|f| *d.primary.span.file == *f.name()) {
      let severity = match d.severity {
        Severity::Error => "error",
        Severity::Warning => "warning",
        Severity::Info => "info",
      };

      let _ = writeln!(ctx.io.err, "{severity}: {}", d.message);
    }
  }

  if super::has_errors(&out.diagnostics) {
    return Ok(None);
  }

  Ok(out.bundle)
}

pub(crate) fn write_executable(
  runtime: &Path,
  bundle: &polar_compiler::core::serial::Bundle,
  output: &Path,
) -> Result<(), CliError> {
  let rt = fs::read(runtime)
    .map_err(|e| CliError::write(runtime.display().to_string(), &e))?;
  let packed = pack(&rt, &bundle_to_bytes(bundle));

  if let Some(parent) = output.parent().filter(|p| !p.as_os_str().is_empty()) {
    fs::create_dir_all(parent)
      .map_err(|e| CliError::write(parent.display().to_string(), &e))?;
  }

  fs::write(output, packed)
    .map_err(|e| CliError::write(output.display().to_string(), &e))?;

  #[cfg(unix)]
  {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(output, fs::Permissions::from_mode(0o755))
      .map_err(|e| CliError::write(output.display().to_string(), &e))?;
  }

  Ok(())
}

pub(crate) struct BuildOptions<'a> {
  pub host: Option<&'a str>,
  pub triple: Option<&'a str>,
  pub out: Option<&'a Path>,
  pub output: Option<&'a Path>,
}

pub(crate) fn build(
  ctx: &mut Ctx<'_, '_>,
  paths: &[PathBuf],
  options: &BuildOptions<'_>,
) -> Result<u8, CliError> {
  let BuildOptions { host, triple, out, output } = *options;

  let file = match paths {
    [] => entry(ctx, None)?,
    [file] => file.clone(),
    _ => {
      return Err(CliError::Message(
        "`--target native` takes a single `.px` file".to_string(),
      ));
    }
  };
  let file = &file;
  let runtime = runtime_binary(ctx, triple)?;
  let Some(bundle) = compile(ctx, file, host)? else {
    return Ok(1);
  };
  let suffix = if triple.is_some_and(|t| t.contains("windows"))
    || (triple.is_none() && cfg!(windows))
  {
    ".exe"
  } else {
    ""
  };
  let output = if let Some(output) = output {
    ctx.io.cwd.join(output)
  } else {
    let target =
      crate::project::targets(paths, &ctx.io.cwd, out)?.into_iter().next();
    let out_dir =
      target.as_ref().map_or_else(|| PathBuf::from("dist"), |t| t.out.clone());
    let stem = target
      .and_then(|t| t.project)
      .or_else(|| file.file_stem().map(|s| s.to_string_lossy().into_owned()));

    ctx
      .io
      .cwd
      .join(out_dir)
      .join(format!("{}{suffix}", stem.unwrap_or_default()))
  };

  write_executable(&runtime, &bundle, &output)?;

  let _ = writeln!(ctx.io.out, "{}", output.display());

  if node_bridges(&bundle) {
    let name = output.file_name().map_or_else(String::new, |n| {
      n.to_string_lossy().trim_end_matches(".exe").to_string()
    });
    let companion = output.with_file_name(format!("{name}.server"));
    let paths: Vec<PathBuf> = vec![file.clone()];

    if let Some(script) = write_node_server(ctx, &paths, &companion)? {
      let _ = writeln!(
        ctx.io.err,
        "note: `{}` calls a Node bridge server; start it first with `node {} <port>` \
         (default 3000), or set POLAR_BRIDGE",
        output.display(),
        script.display()
      );
    }
  }

  Ok(0)
}

pub(crate) fn run(
  ctx: &mut Ctx<'_, '_>,
  file: Option<&Path>,
  host: Option<&str>,
  args: &[String],
) -> Result<u8, CliError> {
  let file_arg = file.map(Path::to_path_buf);
  let file = entry(ctx, file)?;
  let runtime = runtime_binary(ctx, None)?;
  let Some(bundle) = compile(ctx, &file, host)? else {
    return Ok(1);
  };
  let dir = tempfile::Builder::new()
    .prefix("polar-native-")
    .tempdir()
    .map_err(|e| CliError::write("a temporary directory", &e))?;
  let exe = dir.path().join(if cfg!(windows) { "app.exe" } else { "app" });

  write_executable(&runtime, &bundle, &exe)?;

  let main = &bundle.main;
  let mut servers: Vec<String> = main
    .bridges
    .iter()
    .filter(|b| {
      Some(b.host.as_str()) == main.host.as_deref() && b.via != b.host
    })
    .map(|b| b.via.clone())
    .collect();

  servers.sort();
  servers.dedup();

  let mut command = Command::new(&exe);

  command.args(args).current_dir(&ctx.io.cwd);

  let serving = args.first().map(String::as_str) == Some("--serve");
  let _server = if servers.is_empty() || serving {
    None
  } else if servers.iter().all(|s| s == "Node") {
    let paths: Vec<PathBuf> = file_arg.into_iter().collect();
    let Some(sub) = node_server(ctx, &paths, dir.path())? else {
      return Ok(1);
    };

    command
      .env("POLAR_BRIDGE", format!("http://127.0.0.1:{}/_polar", sub.port));

    Some(sub)
  } else {
    return Err(CliError::Message(format!(
      "`polar run --target native` can start a Node bridge server, not {}",
      servers.join(", ")
    )));
  };
  command.stdout(std::process::Stdio::piped());

  let mut child = command.spawn().map_err(|e| {
    CliError::Message(format!("cannot run {}: {e}", exe.display()))
  })?;

  if let Some(mut stdout) = child.stdout.take() {
    let mut chunk = [0u8; 8192];

    while let Ok(n) = std::io::Read::read(&mut stdout, &mut chunk) {
      if n == 0 {
        break;
      }

      let _ = ctx.io.out.write_all(&chunk[..n]);
      let _ = ctx.io.out.flush();
    }
  }

  let status = child.wait().map_err(|e| {
    CliError::Message(format!("cannot run {}: {e}", exe.display()))
  })?;

  Ok(status.code().map_or(1, |code| u8::try_from(code).unwrap_or(1)))
}

struct SubService {
  child: std::process::Child,
  port: u16,
}

impl Drop for SubService {
  fn drop(&mut self) {
    let _ = self.child.kill();
    let _ = self.child.wait();
  }
}

const SERVE_MJS: &str = r#"import { createServer } from "node:http";
import { bridges } from "./_polar/bridges.js";
import { bridge } from "./_polar/runtime.js";

createServer(bridge.nodeListener(bridges)).listen(
  Number(process.argv[2] ?? 3000),
  "127.0.0.1",
);
"#;

fn write_node_server(
  ctx: &mut Ctx<'_, '_>,
  paths: &[PathBuf],
  out: &Path,
) -> Result<Option<PathBuf>, CliError> {
  let targets = crate::project::targets(paths, &ctx.io.cwd, Some(out))?;

  if super::build::build_into(ctx, &targets, Some("Node"), false, None)?
    .is_none()
  {
    return Ok(None);
  }

  let script = out.join("serve.mjs");

  fs::write(&script, SERVE_MJS)
    .map_err(|e| CliError::write(script.display().to_string(), &e))?;

  Ok(Some(script))
}

fn node_bridges(bundle: &polar_compiler::core::serial::Bundle) -> bool {
  let main = &bundle.main;

  main.bridges.iter().any(|b| {
    Some(b.host.as_str()) == main.host.as_deref()
      && b.via != b.host
      && b.via == "Node"
  })
}

fn node_server(
  ctx: &mut Ctx<'_, '_>,
  paths: &[PathBuf],
  scratch: &Path,
) -> Result<Option<SubService>, CliError> {
  let node = super::run::needs_node(ctx)?;
  let out = scratch.join("server");
  let Some(script) = write_node_server(ctx, paths, &out)? else {
    return Ok(None);
  };

  let port = std::net::TcpListener::bind("127.0.0.1:0")
    .and_then(|l| l.local_addr())
    .map(|a| a.port())
    .map_err(|e| {
      CliError::Message(format!("no free port for the bridge server: {e}"))
    })?;
  let child = Command::new(node)
    .arg(&script)
    .arg(port.to_string())
    .stdout(std::process::Stdio::null())
    .spawn()
    .map_err(|e| {
      CliError::Message(format!("cannot start the bridge server: {e}"))
    })?;
  let sub = SubService { child, port };

  for _ in 0..100 {
    if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
      return Ok(Some(sub));
    }

    std::thread::sleep(std::time::Duration::from_millis(50));
  }

  Err(CliError::Message("the bridge server did not start".to_string()))
}
