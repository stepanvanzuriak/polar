macro_rules! sexp {
  ($($tree:tt)+) => {
    $crate::common::sexp::render(&format!("({})", stringify!($($tree)+)))
  };
}

pub(crate) use sexp;

use std::fmt::Write as _;

enum Tree {
  Atom(String),
  Node(Vec<Tree>),
}

pub fn render(src: &str) -> String {
  let tokens = tokenize(src);
  let mut pos = 0;
  let tree = parse(&tokens, &mut pos);

  assert_eq!(pos, tokens.len(), "sexp: trailing input after the root node");

  let mut out = String::new();

  write(&mut out, &tree, 0);
  out.push('\n');

  out
}

fn tokenize(src: &str) -> Vec<String> {
  let mut tokens: Vec<String> = Vec::new();
  let mut chars = src.chars().peekable();

  while let Some(c) = chars.next() {
    match c {
      '(' | ')' | '=' => tokens.push(c.to_string()),
      c if c.is_whitespace() => {}
      '"' => {
        let mut text = String::from('"');

        while let Some(c) = chars.next() {
          text.push(c);

          match c {
            '\\' => text.extend(chars.next()),
            '"' => break,
            _ => {}
          }
        }

        tokens.push(text);
      }
      _ => {
        let mut word = String::from(c);

        while let Some(&c) = chars.peek() {
          if c.is_whitespace() || matches!(c, '(' | ')' | '=' | '"') {
            break;
          }

          word.push(c);
          chars.next();
        }

        tokens.push(word);
      }
    }
  }

  let mut merged: Vec<String> = Vec::new();
  let mut iter = tokens.into_iter();

  while let Some(token) = iter.next() {
    if token == "=" {
      let key = merged.pop().expect("sexp: `=` with no key before it");
      let value = iter.next().expect("sexp: `=` with no value after it");

      merged.push(format!("{key}={value}"));
    } else {
      merged.push(token);
    }
  }

  merged
}

fn parse(tokens: &[String], pos: &mut usize) -> Tree {
  let token = tokens.get(*pos).expect("sexp: unexpected end of input");

  *pos += 1;

  if token != "(" {
    return Tree::Atom(token.clone());
  }

  let mut items = Vec::new();

  loop {
    match tokens.get(*pos).map(String::as_str) {
      Some(")") => {
        *pos += 1;

        return Tree::Node(items);
      }
      Some(_) => items.push(parse(tokens, pos)),
      None => panic!("sexp: unclosed `(`"),
    }
  }
}

fn write(out: &mut String, tree: &Tree, indent: usize) {
  let Tree::Node(items) = tree else {
    panic!("sexp: the root must be a `(...)` node");
  };

  let _ = write!(out, "{:indent$}(", "");

  let mut first = true;

  for item in items {
    if let Tree::Atom(word) = item {
      if !first {
        out.push(' ');
      }

      out.push_str(word);
      first = false;
    }
  }

  for item in items {
    if let Tree::Node(_) = item {
      out.push('\n');
      write(out, item, indent + 2);
    }
  }

  out.push(')');
}
