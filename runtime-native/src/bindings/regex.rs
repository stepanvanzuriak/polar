use crate::{
  builtins::text,
  interp::{Interp, R},
  value::Value,
};
use regex::{NoExpand, Regex};
use std::{cell::RefCell, collections::HashMap};

thread_local! {
  static CACHE: RefCell<HashMap<String, Option<Regex>>> =
    RefCell::new(HashMap::new());
}

const NEWLINES: &str = "\\n\\r\\u{2028}\\u{2029}";
const SPACE: &str = " \\t\\n\\u{b}\\u{c}\\r\\u{a0}\\u{1680}\\u{2000}-\\u{200a}\\u{2028}\\u{2029}\\u{202f}\\u{205f}\\u{3000}\\u{feff}";

fn portable(pattern: &str) -> Result<(), String> {
  let mut chars = pattern.chars().peekable();
  let mut in_class = false;

  while let Some(c) = chars.next() {
    match c {
      '\\' => {
        let Some(next) = chars.next() else { continue };

        if !in_class && (next.is_ascii_digit() && next != '0' || next == 'k') {
          return Err("back-references are not supported".to_string());
        }
      }
      '[' => in_class = true,
      ']' => in_class = false,
      '(' if !in_class && chars.peek() == Some(&'?') => {
        let rest: String = chars.clone().skip(1).take(2).collect();

        if rest.starts_with('=')
          || rest.starts_with('!')
          || rest == "<="
          || rest == "<!"
        {
          return Err("look-around is not supported".to_string());
        }
      }
      _ => {}
    }
  }

  Ok(())
}

fn translate(pattern: &str) -> String {
  let mut out = String::new();
  let mut chars = pattern.chars().peekable();
  let mut in_class = false;

  while let Some(c) = chars.next() {
    match c {
      '\\' => {
        let Some(next) = chars.next() else {
          out.push('\\');
          continue;
        };

        match (next, in_class) {
          ('d', false) => out.push_str("[0-9]"),
          ('D', false) => out.push_str("[^0-9]"),
          ('w', false) => out.push_str("[0-9A-Za-z_]"),
          ('W', false) => out.push_str("[^0-9A-Za-z_]"),
          ('s', false) => out.push_str(&format!("[{SPACE}]")),
          ('S', false) => out.push_str(&format!("[^{SPACE}]")),
          ('b', false) => out.push_str("(?-u:\\b)"),
          ('B', false) => out.push_str("(?-u:\\B)"),
          ('d', true) => out.push_str("0-9"),
          ('w', true) => out.push_str("0-9A-Za-z_"),
          ('s', true) => out.push_str(SPACE),
          _ => {
            out.push('\\');
            out.push(next);
          }
        }
      }
      '[' if !in_class => {
        in_class = true;
        out.push('[');
      }
      ']' if in_class => {
        in_class = false;
        out.push(']');
      }
      '.' if !in_class => out.push_str(&format!("[^{NEWLINES}]")),
      c => out.push(c),
    }
  }

  out
}

fn build(pattern: &str) -> Result<Regex, String> {
  portable(pattern)?;

  Regex::new(&translate(pattern)).map_err(|e| {
    e.to_string().lines().last().unwrap_or("invalid pattern").trim().to_string()
  })
}

fn compiled(pattern: &str) -> Option<Regex> {
  CACHE.with(|cache| {
    cache
      .borrow_mut()
      .entry(pattern.to_string())
      .or_insert_with(|| build(pattern).ok())
      .clone()
  })
}

pub fn syntax_error(_: &mut Interp, a: &[Value]) -> R {
  Ok(match build(text(a, 0)?) {
    Ok(_) => Value::none(),
    Err(message) => Value::some(Value::str(if message.is_empty() {
      "invalid pattern".to_string()
    } else {
      message
    })),
  })
}

pub fn matches(_: &mut Interp, a: &[Value]) -> R {
  let (pattern, input) = (text(a, 0)?, text(a, 1)?);

  Ok(Value::Bool(compiled(pattern).is_some_and(|re| re.is_match(input))))
}

pub fn first_match(_: &mut Interp, a: &[Value]) -> R {
  let (pattern, input) = (text(a, 0)?, text(a, 1)?);
  let found = compiled(pattern)
    .and_then(|re| re.find(input).map(|m| m.as_str().to_string()));

  Ok(found.map_or_else(Value::none, |m| Value::some(Value::str(m))))
}

pub fn all_matches(_: &mut Interp, a: &[Value]) -> R {
  let (pattern, input) = (text(a, 0)?, text(a, 1)?);
  let items = compiled(pattern).map_or_else(Vec::new, |re| {
    re.find_iter(input).map(|m| Value::str(m.as_str())).collect()
  });

  Ok(Value::list(items))
}

pub fn capture_groups(_: &mut Interp, a: &[Value]) -> R {
  let Some(re) = compiled(text(a, 0)?) else {
    return Ok(Value::none());
  };

  Ok(match re.captures(text(a, 1)?) {
    None => Value::none(),
    Some(caps) => Value::some(Value::list(
      caps
        .iter()
        .skip(1)
        .map(|g| Value::str(g.map_or("", |m| m.as_str())))
        .collect(),
    )),
  })
}

pub fn replace_all(_: &mut Interp, a: &[Value]) -> R {
  let (pattern, input, with) = (text(a, 0)?, text(a, 1)?, text(a, 2)?);

  Ok(Value::str(match compiled(pattern) {
    None => input.to_string(),
    Some(re) => re.replace_all(input, NoExpand(with)).into_owned(),
  }))
}
