use std::{ffi::OsString, io::IsTerminal, process::ExitCode};

fn main() -> ExitCode {
  let debug = std::env::var("POLAR_DEBUG").is_ok_and(|v| v == "1");

  if !debug {
    std::panic::set_hook(Box::new(|_| {}));
  }

  let args: Vec<OsString> = std::env::args_os().skip(1).collect();
  let (mut stdout, mut stderr) =
    (std::io::stdout().lock(), std::io::stderr().lock());
  let env = |name: &str| std::env::var(name).ok();
  let mut io = polar_cli::Io {
    stdout_is_tty: stdout.is_terminal(),
    stderr_is_tty: stderr.is_terminal(),
    out: &mut stdout,
    err: &mut stderr,
    cwd: std::env::current_dir().unwrap_or_default(),
    env: &env,
  };

  let compiler = if std::env::var_os("POLAR_INTERNAL_TEST_PANIC").is_some() {
    polar_cli::Compiler {
      compile: |_, _, _| panic!("test panic"),
      ..Default::default()
    }
  } else {
    polar_cli::Compiler::default()
  };

  let code = polar_cli::run_with(&args, &mut io, compiler);
  let _ = io.out.flush();

  ExitCode::from(code)
}
