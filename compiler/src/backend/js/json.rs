use std::fmt::Write as _;

pub fn write_string(out: &mut String, s: &str) {
  out.push('"');

  for c in s.chars() {
    match c {
      '"' => out.push_str("\\\""),
      '\\' => out.push_str("\\\\"),
      c => write_char(out, c),
    }
  }

  out.push('"');
}

pub fn write_template_text(out: &mut String, s: &str) {
  let mut chars = s.chars().peekable();

  while let Some(c) = chars.next() {
    match c {
      '`' => out.push_str("\\`"),
      '\\' => out.push_str("\\\\"),
      '$' if chars.peek() == Some(&'{') => out.push_str("\\$"),
      c => write_char(out, c),
    }
  }
}

fn write_char(out: &mut String, c: char) {
  match c {
    '\u{8}' => out.push_str("\\b"),
    '\u{c}' => out.push_str("\\f"),
    '\n' => out.push_str("\\n"),
    '\r' => out.push_str("\\r"),
    '\t' => out.push_str("\\t"),
    c if u32::from(c) < 0x20 => {
      let _ = write!(out, "\\u{:04x}", u32::from(c));
    }
    c => out.push(c),
  }
}

#[must_use]
pub fn string(s: &str) -> String {
  let mut out = String::with_capacity(s.len() + 2);

  write_string(&mut out, s);
  out
}
