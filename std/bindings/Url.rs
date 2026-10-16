use crate::{
  builtins::text,
  interp::{Interp, R},
  value::Value,
};
use std::fmt::Write;

fn unreserved(b: u8) -> bool {
  b.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&b)
}

pub fn encode(_: &mut Interp, a: &[Value]) -> R {
  let mut out = String::new();

  for b in text(a, 0)?.bytes() {
    if unreserved(b) {
      out.push(char::from(b));
    } else {
      let _ = write!(out, "%{b:02X}");
    }
  }

  Ok(Value::str(out))
}

fn hex(b: u8) -> Option<u8> {
  match b {
    b'0'..=b'9' => Some(b - b'0'),
    b'a'..=b'f' => Some(b - b'a' + 10),
    b'A'..=b'F' => Some(b - b'A' + 10),
    _ => None,
  }
}

fn decoded(s: &str) -> Option<String> {
  let bytes = s.as_bytes();
  let mut out = Vec::with_capacity(bytes.len());
  let mut i = 0;

  while i < bytes.len() {
    if bytes[i] == b'%' {
      let high = hex(*bytes.get(i + 1)?)?;
      let low = hex(*bytes.get(i + 2)?)?;

      out.push(high * 16 + low);
      i += 3;
    } else {
      out.push(bytes[i]);
      i += 1;
    }
  }

  String::from_utf8(out).ok()
}

pub fn decode(_: &mut Interp, a: &[Value]) -> R {
  let s = text(a, 0)?;

  Ok(Value::str(decoded(s).unwrap_or_else(|| s.to_string())))
}
