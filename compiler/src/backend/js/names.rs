use std::borrow::Cow;

pub const RESERVED: &[&str] = &[
  "Infinity",
  "NaN",
  "arguments",
  "await",
  "break",
  "case",
  "catch",
  "class",
  "const",
  "continue",
  "debugger",
  "default",
  "delete",
  "do",
  "else",
  "enum",
  "eval",
  "export",
  "extends",
  "false",
  "finally",
  "for",
  "function",
  "if",
  "implements",
  "import",
  "in",
  "instanceof",
  "interface",
  "let",
  "new",
  "null",
  "package",
  "private",
  "protected",
  "public",
  "return",
  "static",
  "super",
  "switch",
  "this",
  "throw",
  "true",
  "try",
  "typeof",
  "undefined",
  "var",
  "void",
  "while",
  "with",
  "yield",
];

#[must_use]
pub fn js_ident(name: &str) -> Cow<'_, str> {
  if !name.contains('$') && RESERVED.binary_search(&name).is_ok() {
    Cow::Owned(format!("{name}$"))
  } else {
    Cow::Borrowed(name)
  }
}
