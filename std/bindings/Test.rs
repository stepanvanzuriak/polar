use crate::{
  builtins::{arg, number, text},
  interp::{Control, Interp, R},
  value::{Value, show},
};
use std::{
  cell::RefCell,
  io::Write,
  path::PathBuf,
  rc::Rc,
  sync::OnceLock,
  time::{Duration, Instant},
};

#[derive(Clone, Default)]
struct Buffer(Rc<RefCell<Vec<u8>>>);

impl Write for Buffer {
  fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
    self.0.borrow_mut().extend_from_slice(bytes);
    Ok(bytes.len())
  }

  fn flush(&mut self) -> std::io::Result<()> {
    Ok(())
  }
}

pub fn capture(it: &mut Interp, a: &[Value]) -> R {
  let run = arg(a, 0)?.clone();
  let buffer = Buffer::default();
  let _ = it.out.flush();
  let out = std::mem::replace(&mut it.out, Box::new(buffer.clone()));
  let err = std::mem::replace(&mut it.err, Box::new(buffer.clone()));
  let result = it.apply(&run, Vec::new());

  it.out = out;
  it.err = err;

  let value = result?;
  let output = String::from_utf8_lossy(&buffer.0.borrow()).into_owned();

  Ok(Value::record(vec![("output", Value::str(output)), ("value", value)]))
}

fn problem(kind: &str, message: String) -> Value {
  Value::record(vec![
    ("kind", Value::str(kind)),
    ("message", Value::str(message)),
    ("at", Value::str("")),
  ])
}

fn payload(value: &Value) -> String {
  match value {
    Value::Variant(v) => {
      v.fields.first().map(Value::coerce_string).unwrap_or_default()
    }
    other => other.coerce_string(),
  }
}

pub fn guard(it: &mut Interp, a: &[Value]) -> R {
  let run = arg(a, 0)?.clone();
  let limit = Duration::from_secs_f64(number(a, 1)?.max(0.0) / 1000.0);
  let started = Instant::now();
  let saved = it.deadline.replace(started + limit);
  let _ = it.out.flush();
  let err = std::mem::replace(&mut it.err, Box::new(std::io::stdout()));
  let result = it.apply(&run, Vec::new());

  it.err = err;
  it.deadline = saved;

  Ok(match result {
    Ok(_) if started.elapsed() > limit => Value::variant("Timeout", Vec::new()),
    Ok(_) => Value::variant("Pass", Vec::new()),
    Err(Control::Timeout) => Value::variant("Timeout", Vec::new()),
    Err(Control::Throw { tag, value }) if tag == "Assert.Failed" => {
      Value::variant("Fail", vec![problem("", payload(&value))])
    }
    Err(Control::Throw { value, .. }) => {
      Value::variant("Error", vec![problem("throws", show(&value))])
    }
    Err(other) => {
      Value::variant("Error", vec![problem("host", other.describe())])
    }
  })
}

fn shown(path: &std::path::Path, cwd: &std::path::Path) -> String {
  path.strip_prefix(cwd).unwrap_or(path).to_string_lossy().into_owned()
}

pub fn compare_snapshot(it: &mut Interp, a: &[Value]) -> R {
  let (name, actual, dir) = (text(a, 0)?, text(a, 1)?, text(a, 2)?);
  let update =
    it.env_var("POLAR_UPDATE").is_some_and(|v| !v.is_empty() && v != "0");
  let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
  let shared = cwd.join(dir).join(format!("{name}.txt"));
  let own = it
    .env_var("POLAR_TEST_HOST")
    .map(|host| cwd.join(dir).join(format!("{name}.{host}.txt")))
    .filter(|p| p.is_file());
  let path = own.unwrap_or(shared);
  let expected = std::fs::read_to_string(&path).ok();

  if expected.as_deref() == Some(actual) {
    return Ok(Value::variant("Match", Vec::new()));
  }

  if update {
    if let Some(parent) = path.parent() {
      let _ = std::fs::create_dir_all(parent);
    }

    std::fs::write(&path, actual).map_err(|e| {
      Control::fail(format!("cannot write {}: {e}", path.display()))
    })?;

    return Ok(Value::variant("Created", Vec::new()));
  }

  let path = Value::str(shown(&path, &cwd));

  Ok(match expected {
    None => Value::variant("Missing", vec![path]),
    Some(expected) => Value::variant(
      "Differ",
      vec![Value::record(vec![
        ("path", path),
        ("expected", Value::str(expected)),
      ])],
    ),
  })
}

pub fn clock(_: &mut Interp, _: &[Value]) -> R {
  static START: OnceLock<Instant> = OnceLock::new();

  let start = START.get_or_init(Instant::now);

  Ok(Value::Num(start.elapsed().as_secs_f64() * 1000.0))
}

pub fn finish(it: &mut Interp, _: &[Value]) -> R {
  let _ = it.out.flush();

  Ok(Value::unit())
}

pub fn mock(it: &mut Interp, a: &[Value]) -> R {
  let effect = text(a, 0)?.to_string();
  let ops = arg(a, 1)?.clone();
  let body = arg(a, 2)?.clone();

  it.mocks.push((effect, ops));

  let result = it.apply(&body, Vec::new());

  it.mocks.pop();

  result
}
