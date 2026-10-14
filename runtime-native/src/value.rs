use polar_compiler::core::ir::{CExpr, Sym};
use std::{cell::RefCell, fmt::Write, rc::Rc};

pub type NativeFn =
  fn(&mut crate::Interp, &[Value]) -> Result<Value, crate::Control>;

#[derive(Clone)]
pub enum Value {
  Num(f64),
  Str(Rc<str>),
  Bool(bool),
  Variant(Rc<VariantData>),
  Record(Rc<Vec<(&'static str, Value)>>),
  Closure(Rc<Closure>),
  Native(Rc<Native>),
  CtorFn(Rc<CtorFnData>),
  Ref(Rc<RefCell<Value>>),
  Bridge(Rc<BridgeCall>),
}

pub struct VariantData {
  pub name: &'static str,
  pub fields: Vec<Value>,
}

pub struct BridgeCall {
  pub module: usize,
  pub effect: String,
  pub op: String,
}

pub struct CtorFnData {
  pub name: &'static str,
  pub arity: usize,
}

pub struct Native {
  pub name: String,
  pub f: Option<NativeFn>,
}

pub struct Closure {
  pub params: &'static [Sym],
  pub body: &'static CExpr,
  pub env: Env,
  pub module: usize,
}

pub type Env = Option<Rc<Frame>>;

pub struct Frame {
  pub id: u32,
  pub value: Value,
  pub parent: Env,
}

impl std::fmt::Debug for Value {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.write_str(&show(self))
  }
}

impl Value {
  #[must_use]
  pub fn str(s: impl AsRef<str>) -> Self {
    Self::Str(Rc::from(s.as_ref()))
  }

  #[must_use]
  pub fn unit() -> Self {
    Self::Record(Rc::new(Vec::new()))
  }

  #[must_use]
  pub fn variant(name: &'static str, fields: Vec<Value>) -> Self {
    Self::Variant(Rc::new(VariantData { name, fields }))
  }

  #[must_use]
  pub fn some(value: Value) -> Self {
    Self::variant("Some", vec![value])
  }

  #[must_use]
  pub fn none() -> Self {
    Self::variant("None", Vec::new())
  }

  #[must_use]
  pub fn ok(value: Value) -> Self {
    Self::variant("Ok", vec![value])
  }

  #[must_use]
  pub fn err(value: Value) -> Self {
    Self::variant("Err", vec![value])
  }

  #[must_use]
  pub fn record(fields: Vec<(&'static str, Value)>) -> Self {
    Self::Record(Rc::new(fields))
  }

  #[must_use]
  pub fn list(items: Vec<Value>) -> Self {
    items
      .into_iter()
      .rev()
      .fold(Self::variant("Nil", Vec::new()), |tail, head| {
        Self::variant("Cons", vec![head, tail])
      })
  }

  #[must_use]
  pub fn as_str(&self) -> Option<&str> {
    match self {
      Self::Str(s) => Some(s),
      _ => None,
    }
  }

  #[must_use]
  pub fn as_num(&self) -> Option<f64> {
    match self {
      Self::Num(n) => Some(*n),
      _ => None,
    }
  }

  #[must_use]
  pub fn as_bool(&self) -> Option<bool> {
    match self {
      Self::Bool(b) => Some(*b),
      _ => None,
    }
  }

  #[must_use]
  pub fn field(&self, name: &str) -> Option<&Value> {
    match self {
      Self::Record(fields) => {
        fields.iter().find(|(k, _)| *k == name).map(|(_, v)| v)
      }
      _ => None,
    }
  }

  #[must_use]
  pub fn list_items(&self) -> Option<Vec<Value>> {
    let mut out = Vec::new();
    let mut rest = self.clone();

    loop {
      let Self::Variant(v) = &rest else { return None };

      match (v.name, v.fields.as_slice()) {
        ("Nil", []) => return Some(out),
        ("Cons", [head, tail]) => {
          out.push(head.clone());
          rest = tail.clone();
        }
        _ => return None,
      }
    }
  }

  #[must_use]
  pub fn strict_eq(&self, other: &Value) -> bool {
    match (self, other) {
      (Self::Num(a), Self::Num(b)) => a == b,
      (Self::Str(a), Self::Str(b)) => a == b,
      (Self::Bool(a), Self::Bool(b)) => a == b,
      (Self::Variant(a), Self::Variant(b)) => Rc::ptr_eq(a, b),
      (Self::Record(a), Self::Record(b)) => Rc::ptr_eq(a, b),
      (Self::Closure(a), Self::Closure(b)) => Rc::ptr_eq(a, b),
      (Self::Native(a), Self::Native(b)) => Rc::ptr_eq(a, b),
      (Self::Ref(a), Self::Ref(b)) => Rc::ptr_eq(a, b),
      (Self::Bridge(a), Self::Bridge(b)) => Rc::ptr_eq(a, b),
      _ => false,
    }
  }

  #[must_use]
  pub fn coerce_string(&self) -> String {
    match self {
      Self::Str(s) => s.to_string(),
      Self::Num(n) => number_to_string(*n),
      Self::Bool(b) => b.to_string(),
      other => show(other),
    }
  }
}

#[must_use]
pub fn number_to_string(x: f64) -> String {
  if x.is_nan() {
    return "NaN".to_string();
  }

  if x == 0.0 {
    return "0".to_string();
  }

  if x.is_infinite() {
    return if x > 0.0 { "Infinity" } else { "-Infinity" }.to_string();
  }

  let sign = if x < 0.0 { "-" } else { "" };
  let sci = format!("{:e}", x.abs());
  let (mantissa, exp) = sci.split_once('e').unwrap_or((&sci, "0"));
  let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
  let exp: i32 = exp.parse().unwrap_or(0);
  let k = i32::try_from(digits.len()).unwrap_or(i32::MAX);
  let n = exp + 1;

  let body = if k <= n && n <= 21 {
    format!("{digits}{}", "0".repeat(usize::try_from(n - k).unwrap_or(0)))
  } else if 0 < n && n <= 21 {
    let at = usize::try_from(n).unwrap_or(0);

    format!("{}.{}", &digits[..at], &digits[at..])
  } else if -6 < n && n <= 0 {
    format!("0.{}{digits}", "0".repeat(usize::try_from(-n).unwrap_or(0)))
  } else {
    let e = n - 1;
    let sign_e = if e < 0 { "-" } else { "+" };

    if k == 1 {
      format!("{digits}e{sign_e}{}", e.abs())
    } else {
      format!("{}.{}e{sign_e}{}", &digits[..1], &digits[1..], e.abs())
    }
  };

  format!("{sign}{body}")
}

#[must_use]
pub fn float_to_string(x: f64) -> String {
  let text = number_to_string(x);

  if text.trim_start_matches('-').bytes().all(|b| b.is_ascii_digit()) {
    format!("{text}.0")
  } else {
    text
  }
}

#[must_use]
pub fn json_quote(s: &str) -> String {
  let mut out = String::with_capacity(s.len() + 2);

  out.push('"');

  for c in s.chars() {
    match c {
      '"' => out.push_str("\\\""),
      '\\' => out.push_str("\\\\"),
      '\u{8}' => out.push_str("\\b"),
      '\u{c}' => out.push_str("\\f"),
      '\n' => out.push_str("\\n"),
      '\r' => out.push_str("\\r"),
      '\t' => out.push_str("\\t"),
      c if (c as u32) < 0x20 => {
        let _ = write!(out, "\\u{:04x}", c as u32);
      }
      c => out.push(c),
    }
  }

  out.push('"');
  out
}

#[must_use]
pub fn show(value: &Value) -> String {
  match value {
    Value::Str(s) => json_quote(s),
    Value::Num(n) => number_to_string(*n),
    Value::Bool(b) => b.to_string(),
    Value::Closure(_)
    | Value::Native(_)
    | Value::CtorFn(_)
    | Value::Bridge(_) => "<function>".to_string(),
    Value::Variant(v) => {
      if v.fields.is_empty() {
        v.name.to_string()
      } else {
        let shown: Vec<String> = v.fields.iter().map(show).collect();

        format!("{}({})", v.name, shown.join(", "))
      }
    }
    Value::Record(fields) => record_show(
      &fields.iter().map(|(k, v)| ((*k).to_string(), v)).collect::<Vec<_>>(),
    ),
    Value::Ref(cell) => {
      record_show(&[("value".to_string(), &cell.borrow().clone())])
    }
  }
}

fn record_show(fields: &[(String, &Value)]) -> String {
  if fields.is_empty() {
    return "{}".to_string();
  }

  let shown: Vec<String> =
    fields.iter().map(|(k, v)| format!("{k}: {}", show(v))).collect();

  format!("{{ {} }}", shown.join(", "))
}
