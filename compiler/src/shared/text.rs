use std::fmt::Display;

#[must_use]
pub fn quoted_list<T: Display>(names: &[T]) -> String {
  let quoted: Vec<String> = names.iter().map(|n| format!("`{n}`")).collect();

  match quoted.as_slice() {
    [] => String::new(),
    [one] => one.clone(),
    [init @ .., last] => format!("{} and {last}", init.join(", ")),
  }
}

#[must_use]
pub fn string_literal(text: &str) -> String {
  let mut out = String::from("\"");

  for c in text.chars() {
    match c {
      '\\' | '"' | '#' => {
        out.push('\\');
        out.push(c);
      }
      '\n' => out.push_str("\\n"),
      '\r' => out.push_str("\\r"),
      '\t' => out.push_str("\\t"),
      _ => out.push(c),
    }
  }

  out.push('"');
  out
}
