mod commands;
pub mod files;
mod node;
pub mod plugin;
pub mod project;
pub mod runtime;
pub mod watch;

use clap::{Parser, Subcommand, ValueEnum};
use polar_compiler::{
  CompileOptions, CompileOutput, DumpResult, FormatResult, Stage,
  shared::ice::InternalCompilerError, shared::render::should_use_color,
};
use std::{
  ffi::OsString,
  io::Write,
  panic::{AssertUnwindSafe, catch_unwind},
  path::PathBuf,
};

pub struct Io<'a> {
  pub out: &'a mut dyn Write,
  pub err: &'a mut dyn Write,
  pub stdout_is_tty: bool,
  pub stderr_is_tty: bool,
  pub cwd: PathBuf,
  pub env: &'a dyn Fn(&str) -> Option<String>,
}

#[derive(Clone, Copy)]
pub struct Compiler {
  pub compile: fn(&str, &str, &CompileOptions) -> CompileOutput,
  pub format: fn(&str, &str, &[String]) -> FormatResult,
  pub dump_stage: fn(&str, &str, Stage, &CompileOptions) -> DumpResult,
}

impl Default for Compiler {
  fn default() -> Self {
    Self {
      compile: polar_compiler::compile,
      format: polar_compiler::format_plugins,
      dump_stage: polar_compiler::dump_stage_with,
    }
  }
}

pub fn run(args: &[OsString], io: &mut Io<'_>) -> u8 {
  run_with(args, io, Compiler::default())
}

pub fn run_with(args: &[OsString], io: &mut Io<'_>, compiler: Compiler) -> u8 {
  let command_line =
    std::iter::once(OsString::from("polar")).chain(args.iter().cloned());

  let cli = match Cli::try_parse_from(command_line) {
    Ok(cli) => cli,
    Err(e) if !e.use_stderr() => {
      let _ = write!(io.out, "{}", e.render());
      return 0;
    }
    Err(e) => {
      let _ = write!(io.err, "{}", e.render());
      let _ = writeln!(io.err, "run `polar --help` for usage");
      return 1;
    }
  };

  let flag = if cli.no_color {
    Some(false)
  } else if cli.color {
    Some(true)
  } else {
    None
  };
  let color = should_use_color(io.env, io.stderr_is_tty, flag);
  let debug = (io.env)("POLAR_DEBUG").is_some_and(|v| v == "1");
  let mut ctx = Ctx { io, compiler, color, debug };

  let result = commands::dispatch(cli.command, &mut ctx);

  for notice in plugin::take_notices() {
    let _ = writeln!(ctx.io.err, "{notice}");
  }

  match result {
    Ok(code) => code,
    Err(e) => e.report(ctx.io.err, debug),
  }
}

pub(crate) struct Ctx<'a, 'io> {
  pub io: &'a mut Io<'io>,
  pub compiler: Compiler,
  pub color: bool,
  pub debug: bool,
}

#[derive(Parser)]
#[command(name = "polar", version, about = "The Polar compiler")]
struct Cli {
  #[command(subcommand)]
  command: Command,

  #[arg(long, global = true)]
  color: bool,

  #[arg(long, global = true, conflicts_with = "color")]
  no_color: bool,
}

#[derive(Subcommand)]
pub(crate) enum Command {
  Build {
    paths: Vec<PathBuf>,
    #[arg(long)]
    out: Option<PathBuf>,
    #[arg(long)]
    watch: bool,
    #[arg(long, value_enum)]
    emit: Option<EmitStage>,
    #[arg(long)]
    host: Option<String>,
  },
  Check {
    paths: Vec<PathBuf>,
    #[arg(long)]
    out: Option<PathBuf>,
    #[arg(long)]
    watch: bool,
  },
  Run {
    file: Option<PathBuf>,
    #[arg(long)]
    host: Option<String>,
    #[arg(last = true)]
    args: Vec<String>,
  },
  Fmt {
    paths: Vec<PathBuf>,
    #[arg(long)]
    check: bool,
  },
  Start {
    path: Option<PathBuf>,
    #[arg(long)]
    host: Option<String>,
    #[arg(last = true)]
    args: Vec<String>,
  },
  Test {
    path: Option<PathBuf>,
    #[arg(last = true)]
    args: Vec<String>,
  },
  Init {
    dir: Option<PathBuf>,
    #[arg(long)]
    name: Option<String>,
    #[arg(long = "zone")]
    zones: Vec<String>,
  },
}

#[derive(Clone, Copy, ValueEnum)]
pub(crate) enum EmitStage {
  Tokens,
  Ast,
  Core,
  Types,
  Hosts,
  Dictionaries,
  Js,
}

impl From<EmitStage> for Stage {
  fn from(stage: EmitStage) -> Self {
    match stage {
      EmitStage::Tokens => Self::Tokens,
      EmitStage::Ast => Self::Ast,
      EmitStage::Core => Self::Core,
      EmitStage::Types => Self::Types,
      EmitStage::Hosts => Self::Hosts,
      EmitStage::Dictionaries => Self::Dictionaries,
      EmitStage::Js => Self::Js,
    }
  }
}

#[derive(Debug)]
pub enum CliError {
  Read { path: String, reason: String },
  Write { path: String, reason: String },
  Message(String),
  Ice(Ice),
}

impl CliError {
  pub(crate) fn read(path: impl Into<String>, e: &std::io::Error) -> Self {
    Self::Read { path: path.into(), reason: io_reason(e) }
  }

  pub(crate) fn write(path: impl Into<String>, e: &std::io::Error) -> Self {
    Self::Write { path: path.into(), reason: io_reason(e) }
  }

  pub(crate) fn report(&self, err: &mut dyn Write, debug: bool) -> u8 {
    match self {
      Self::Read { path, reason } => {
        let _ = writeln!(err, "error: cannot read {path}: {reason}");
        1
      }
      Self::Write { path, reason } => {
        let _ = writeln!(err, "error: cannot write {path}: {reason}");
        1
      }
      Self::Message(message) => {
        let _ = writeln!(err, "error: {message}");
        1
      }
      Self::Ice(ice) => {
        ice.report(err, debug);
        2
      }
    }
  }
}

fn io_reason(e: &std::io::Error) -> String {
  let message = e.to_string();

  match message.find(" (os error") {
    Some(i) => message[..i].to_string(),
    None => message,
  }
}

#[derive(Debug)]
pub struct Ice {
  pub message: String,
  pub file: String,
  pub backtrace: Option<String>,
}

impl Ice {
  pub(crate) fn report(&self, err: &mut dyn Write, debug: bool) {
    let _ = writeln!(
      err,
      "error[POLAR0001]: internal compiler error: {}\n  --> {}\n   = note: this is a bug in the \
       Polar compiler, please report it",
      self.message, self.file
    );

    match &self.backtrace {
      Some(backtrace) if debug => {
        let _ = writeln!(err, "\nbacktrace:\n{backtrace}");
      }
      _ => {}
    }
  }
}

pub(crate) fn guard<T>(file: &str, f: impl FnOnce() -> T) -> Result<T, Ice> {
  catch_unwind(AssertUnwindSafe(f)).map_err(|payload| {
    let (message, backtrace) = match payload.downcast::<InternalCompilerError>()
    {
      Ok(ice) => (ice.message.clone(), Some(ice.backtrace.to_string())),
      Err(payload) => match payload.downcast::<String>() {
        Ok(s) => (*s, None),
        Err(payload) => match payload.downcast::<&str>() {
          Ok(s) => ((*s).to_string(), None),
          Err(_) => ("a panic with no message".to_string(), None),
        },
      },
    };

    Ice { message, file: file.to_string(), backtrace }
  })
}

pub(crate) fn plural(n: usize, thing: &str) -> String {
  if n == 1 { format!("1 {thing}") } else { format!("{n} {thing}s") }
}
