use crate::{
  builtins::text,
  interp::{Interp, R},
  value::Value,
};

fn unreserved(b: u8) -> bool {
  b.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&b)
}

pub fn encode(_: &mut Interp, a: &[Value]) -> R {
  let mut out = String::new();

  for b in text(a, 0)?.bytes() {
    if unreserved(b) {
      out.push(char::from(b));
    } else {
      out.push_str(&format!("%{b:02X}"));
    }
  }

  Ok(Value::str(out))
}

fn hex(b: u8) -> Option<u8> {
  char::from(b).to_digit(16).map(|d| u8::try_from(d).unwrap_or(0))
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
