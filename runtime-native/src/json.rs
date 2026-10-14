use crate::{
  interp::Control,
  value::{Value, json_quote, number_to_string},
};

pub fn parse(text: &str) -> Result<Value, String> {
  let mut p = Parser { s: text.as_bytes(), text, at: 0 };

  p.ws();

  let v = p.value()?;

  p.ws();

  if p.at < p.s.len() {
    return Err(p.unexpected());
  }

  Ok(v)
}

struct Parser<'a> {
  s: &'a [u8],
  text: &'a str,
  at: usize,
}

impl Parser<'_> {
  fn ws(&mut self) {
    while matches!(self.s.get(self.at), Some(b' ' | b'\t' | b'\n' | b'\r')) {
      self.at += 1;
    }
  }

  fn unexpected(&self) -> String {
    match self.text[self.at..].chars().next() {
      None => "Unexpected end of JSON input".to_string(),
      Some(c) => format!(
        "Unexpected token {c} in JSON at position {}",
        self.text[..self.at].encode_utf16().count()
      ),
    }
  }

  fn expect(&mut self, word: &str) -> Result<(), String> {
    if self.text[self.at..].starts_with(word) {
      self.at += word.len();

      Ok(())
    } else {
      Err(self.unexpected())
    }
  }

  fn value(&mut self) -> Result<Value, String> {
    match self.s.get(self.at) {
      Some(b'n') => {
        self.expect("null")?;

        Ok(Value::variant("JNull", Vec::new()))
      }
      Some(b't') => {
        self.expect("true")?;

        Ok(Value::variant("JBool", vec![Value::Bool(true)]))
      }
      Some(b'f') => {
        self.expect("false")?;

        Ok(Value::variant("JBool", vec![Value::Bool(false)]))
      }
      Some(b'"') => {
        let s = self.string()?;

        Ok(Value::variant("JString", vec![Value::str(s)]))
      }
      Some(b'[') => self.array(),
      Some(b'{') => self.object(),
      Some(b'-' | b'0'..=b'9') => self.number(),
      Some(_) | None => Err(self.unexpected()),
    }
  }

  fn number(&mut self) -> Result<Value, String> {
    let start = self.at;

    if self.s.get(self.at) == Some(&b'-') {
      self.at += 1;
    }

    match self.s.get(self.at) {
      Some(b'0') => self.at += 1,
      Some(b'1'..=b'9') => {
        while matches!(self.s.get(self.at), Some(b'0'..=b'9')) {
          self.at += 1;
        }
      }
      _ => return Err(self.unexpected()),
    }

    if self.s.get(self.at) == Some(&b'.') {
      self.at += 1;

      if !matches!(self.s.get(self.at), Some(b'0'..=b'9')) {
        return Err(self.unexpected());
      }

      while matches!(self.s.get(self.at), Some(b'0'..=b'9')) {
        self.at += 1;
      }
    }

    if matches!(self.s.get(self.at), Some(b'e' | b'E')) {
      self.at += 1;

      if matches!(self.s.get(self.at), Some(b'+' | b'-')) {
        self.at += 1;
      }

      if !matches!(self.s.get(self.at), Some(b'0'..=b'9')) {
        return Err(self.unexpected());
      }

      while matches!(self.s.get(self.at), Some(b'0'..=b'9')) {
        self.at += 1;
      }
    }

    let n: f64 =
      self.text[start..self.at].parse().map_err(|_| self.unexpected())?;

    Ok(Value::variant("JNumber", vec![Value::Num(n)]))
  }

  fn string(&mut self) -> Result<String, String> {
    self.at += 1;

    let mut out = String::new();

    loop {
      let Some(c) = self.text[self.at..].chars().next() else {
        return Err("Unterminated string in JSON".to_string());
      };

      self.at += c.len_utf8();

      match c {
        '"' => return Ok(out),
        '\\' => {
          let Some(esc) = self.text[self.at..].chars().next() else {
            return Err("Unterminated string in JSON".to_string());
          };

          self.at += esc.len_utf8();

          match esc {
            '"' | '\\' | '/' => out.push(esc),
            'b' => out.push('\u{8}'),
            'f' => out.push('\u{c}'),
            'n' => out.push('\n'),
            'r' => out.push('\r'),
            't' => out.push('\t'),
            'u' => out.push(self.unicode()?),
            _ => {
              self.at -= esc.len_utf8();

              return Err(self.unexpected());
            }
          }
        }
        c if (c as u32) < 0x20 => {
          self.at -= c.len_utf8();

          return Err(self.unexpected());
        }
        c => out.push(c),
      }
    }
  }

  fn hex4(&mut self) -> Result<u32, String> {
    let digits =
      self.text.get(self.at..self.at + 4).ok_or_else(|| self.unexpected())?;
    let n = u32::from_str_radix(digits, 16).map_err(|_| self.unexpected())?;

    self.at += 4;

    Ok(n)
  }

  fn unicode(&mut self) -> Result<char, String> {
    let first = self.hex4()?;

    if (0xD800..0xDC00).contains(&first)
      && self.text[self.at..].starts_with("\\u")
    {
      let save = self.at;

      self.at += 2;

      let second = self.hex4()?;

      if (0xDC00..0xE000).contains(&second) {
        let code = 0x10000 + ((first - 0xD800) << 10) + (second - 0xDC00);

        return Ok(char::from_u32(code).unwrap_or('\u{fffd}'));
      }

      self.at = save;
    }

    Ok(char::from_u32(first).unwrap_or('\u{fffd}'))
  }

  fn array(&mut self) -> Result<Value, String> {
    self.at += 1;
    self.ws();

    let mut items = Vec::new();

    if self.s.get(self.at) == Some(&b']') {
      self.at += 1;

      return Ok(Value::variant("JArray", vec![Value::list(items)]));
    }

    loop {
      self.ws();
      items.push(self.value()?);
      self.ws();

      match self.s.get(self.at) {
        Some(b',') => self.at += 1,
        Some(b']') => {
          self.at += 1;

          return Ok(Value::variant("JArray", vec![Value::list(items)]));
        }
        _ => return Err(self.unexpected()),
      }
    }
  }

  fn object(&mut self) -> Result<Value, String> {
    self.at += 1;
    self.ws();

    let mut entries: Vec<(String, Value)> = Vec::new();

    if self.s.get(self.at) == Some(&b'}') {
      self.at += 1;

      return Ok(Value::variant("JObject", vec![Value::list(Vec::new())]));
    }

    loop {
      self.ws();

      if self.s.get(self.at) != Some(&b'"') {
        return Err(self.unexpected());
      }

      let key = self.string()?;

      self.ws();

      if self.s.get(self.at) != Some(&b':') {
        return Err(self.unexpected());
      }

      self.at += 1;
      self.ws();

      let value = self.value()?;

      match entries.iter_mut().find(|(k, _)| *k == key) {
        Some(slot) => slot.1 = value,
        None => entries.push((key, value)),
      }

      self.ws();

      match self.s.get(self.at) {
        Some(b',') => self.at += 1,
        Some(b'}') => {
          self.at += 1;

          let items = entries
            .into_iter()
            .map(|(k, v)| {
              Value::record(vec![("key", Value::str(k)), ("value", v)])
            })
            .collect();

          return Ok(Value::variant("JObject", vec![Value::list(items)]));
        }
        _ => return Err(self.unexpected()),
      }
    }
  }
}

pub fn print(value: &Value) -> Result<String, Control> {
  let mut out = String::new();

  write(value, &mut out)?;

  Ok(out)
}

fn write(value: &Value, out: &mut String) -> Result<(), Control> {
  let Value::Variant(v) = value else {
    return Err(Control::fail("not a JsonValue"));
  };

  match (v.name, v.fields.as_slice()) {
    ("JNull", []) => out.push_str("null"),
    ("JBool", [Value::Bool(b)]) => out.push_str(&b.to_string()),
    ("JNumber", [Value::Num(n)]) => {
      out.push_str(&if n.is_finite() {
        number_to_string(*n)
      } else {
        "null".to_string()
      });
    }
    ("JString", [Value::Str(s)]) => out.push_str(&json_quote(s)),
    ("JArray", [items]) => {
      out.push('[');

      for (i, item) in items.list_items().unwrap_or_default().iter().enumerate()
      {
        if i > 0 {
          out.push(',');
        }

        write(item, out)?;
      }

      out.push(']');
    }
    ("JObject", [fields]) => {
      let mut entries: Vec<(String, Value)> = Vec::new();

      for field in fields.list_items().unwrap_or_default() {
        let key =
          field.field("key").and_then(Value::as_str).unwrap_or("").to_string();
        let value = field.field("value").cloned().unwrap_or_else(Value::unit);

        match entries.iter_mut().find(|(k, _)| *k == key) {
          Some(slot) => slot.1 = value,
          None => entries.push((key, value)),
        }
      }

      out.push('{');

      for (i, (k, v)) in entries.iter().enumerate() {
        if i > 0 {
          out.push(',');
        }

        out.push_str(&json_quote(k));
        out.push(':');
        write(v, out)?;
      }

      out.push('}');
    }
    _ => return Err(Control::fail("not a JsonValue")),
  }

  Ok(())
}
