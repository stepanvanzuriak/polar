use polar_compiler::core::serial::{bundle_from_bytes, unpack};
use polar_rt::Interp;
use std::process::ExitCode;

const STACK: usize = 1 << 30;

fn main() -> ExitCode {
  let mut args: Vec<String> = std::env::args().skip(1).collect();
  let exe = match std::env::current_exe().and_then(std::fs::read) {
    Ok(bytes) => bytes,
    Err(e) => {
      eprintln!("polar-rt: cannot read its own executable: {e}");

      return ExitCode::from(2);
    }
  };
  let payload = match unpack(&exe) {
    Some(payload) => payload.to_vec(),
    None => match args.first() {
      Some(path) if !path.starts_with('-') => {
        let path = args.remove(0);

        match std::fs::read(&path) {
          Ok(bytes) => bytes,
          Err(e) => {
            eprintln!("polar-rt: cannot read {path}: {e}");

            return ExitCode::from(2);
          }
        }
      }
      _ => {
        eprintln!("usage: polar-rt <program.bundle> [args...]");
        eprintln!(
          "  or run an executable built by `polar build --target native`"
        );

        return ExitCode::from(2);
      }
    },
  };
  drop(exe);

  let bundle = match bundle_from_bytes(&payload) {
    Ok(bundle) => bundle,
    Err(message) => {
      eprintln!("polar-rt: {message}");

      return ExitCode::from(2);
    }
  };
  let handle = std::thread::Builder::new().stack_size(STACK).spawn(move || {
    let serve = (args.first().map(String::as_str) == Some("--serve"))
      .then(|| args.get(1).and_then(|p| p.parse::<u16>().ok()));
    let mut interp = Interp::new(bundle, args);

    match serve {
      Some(Some(port)) => match polar_rt::bridge::serve(&mut interp, port) {
        Ok(()) => 0,
        Err(message) => {
          eprintln!("polar-rt: {message}");
          2
        }
      },
      Some(None) => {
        eprintln!("usage: <program> --serve <port>");
        2
      }
      None => interp.run_main().exit_code,
    }
  });
  let code = match handle.map(std::thread::JoinHandle::join) {
    Ok(Ok(code)) => code,
    _ => 1,
  };

  ExitCode::from(u8::try_from(code).unwrap_or(1))
}
