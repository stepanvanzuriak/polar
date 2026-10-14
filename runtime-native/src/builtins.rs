use crate::{
  interp::{Control, Interp, R},
  json,
  value::{NativeFn, Value, number_to_string},
};
use std::{cell::RefCell, io::Write, rc::Rc};

#[must_use]
pub fn lookup(module: &str, member: &str) -> Option<NativeFn> {
  Some(match (module, member) {
    ("Log", "info") => log_info,
    ("Log", "error") => log_error,
    ("String", "replace") => string_replace,
    ("String", "concat") => string_concat,
    ("String" | "Int" | "Float" | "Bool", "eq") => eq,
    ("String", "contains") => string_contains,
    ("String", "ends_with") => string_ends_with,
    ("String", "length") => string_length,
    ("String", "lowercase") => string_lowercase,
    ("String", "repeat") => string_repeat,
    ("String", "slice") => string_slice,
    ("String", "starts_with") => string_starts_with,
    ("String", "trim") => string_trim,
    ("String", "uppercase") => string_uppercase,
    ("String", "charCodeAt") => string_char_code_at,
    ("String", "split") => string_split,
    ("String", "split_once") => string_split_once,
    ("String", "chars") => string_chars,
    ("Int" | "Bool", "to_string") => to_string,
    ("Float", "to_string") => float_to_string,
    ("Int", "to_float") => identity,
    ("Int", "to_base") => int_to_base,
    ("Int", "parse") => int_parse,
    ("Float", "to_int") => float_to_int,
    ("Float", "sqrt") => |_, a| num1(a, f64::sqrt),
    ("Float", "abs") => |_, a| num1(a, f64::abs),
    ("Float", "floor") => |_, a| num1(a, |n| n - (n % 1.0)),
    ("Float", "sin") => |_, a| num1(a, f64::sin),
    ("Float", "cos") => |_, a| num1(a, f64::cos),
    ("Float", "atan2") => float_atan2,
    ("RefRaw", "new") => ref_new,
    ("RefRaw", "get") => ref_get,
    ("RefRaw", "set") => ref_set,
    ("JsonRaw", "parse") => json_parse,
    ("JsonRaw", "print") => json_print,
    ("JsonRaw", "lookup") => json_lookup,
    _ => return None,
  })
}

pub fn arg(args: &[Value], i: usize) -> Result<&Value, Control> {
  args.get(i).ok_or_else(|| Control::fail(format!("missing argument {i}")))
}

pub fn text(args: &[Value], i: usize) -> Result<&str, Control> {
  args
    .get(i)
    .and_then(Value::as_str)
    .ok_or_else(|| Control::fail(format!("argument {i} should be a String")))
}

pub fn number(args: &[Value], i: usize) -> Result<f64, Control> {
  args
    .get(i)
    .and_then(Value::as_num)
    .ok_or_else(|| Control::fail(format!("argument {i} should be a number")))
}

fn num1(args: &[Value], f: fn(f64) -> f64) -> R {
  Ok(Value::Num(f(number(args, 0)?)))
}

fn log_info(it: &mut Interp, args: &[Value]) -> R {
  let s = args.first().map(Value::coerce_string).unwrap_or_default();

  let _ = writeln!(it.out, "{s}");

  Ok(Value::unit())
}

fn log_error(it: &mut Interp, args: &[Value]) -> R {
  let s = args.first().map(Value::coerce_string).unwrap_or_default();

  let _ = it.out.flush();
  let _ = writeln!(it.err, "{s}");

  Ok(Value::unit())
}

fn string_replace(_: &mut Interp, a: &[Value]) -> R {
  let (s, from, to) = (text(a, 0)?, text(a, 1)?, text(a, 2)?);

  Ok(Value::str(if from.is_empty() {
    let mut out = String::from(to);

    for c in s.chars() {
      out.push(c);
      out.push_str(to);
    }

    out
  } else {
    s.replace(from, to)
  }))
}

fn string_concat(_: &mut Interp, a: &[Value]) -> R {
  Ok(Value::str(format!("{}{}", text(a, 0)?, text(a, 1)?)))
}

fn eq(_: &mut Interp, a: &[Value]) -> R {
  Ok(Value::Bool(arg(a, 0)?.strict_eq(arg(a, 1)?)))
}

fn string_contains(_: &mut Interp, a: &[Value]) -> R {
  Ok(Value::Bool(text(a, 0)?.contains(text(a, 1)?)))
}

fn string_ends_with(_: &mut Interp, a: &[Value]) -> R {
  Ok(Value::Bool(text(a, 0)?.ends_with(text(a, 1)?)))
}

fn string_starts_with(_: &mut Interp, a: &[Value]) -> R {
  Ok(Value::Bool(text(a, 0)?.starts_with(text(a, 1)?)))
}

#[allow(clippy::cast_precision_loss)]
fn string_length(_: &mut Interp, a: &[Value]) -> R {
  Ok(Value::Num(text(a, 0)?.chars().count() as f64))
}

fn string_lowercase(_: &mut Interp, a: &[Value]) -> R {
  Ok(Value::str(text(a, 0)?.to_lowercase()))
}

fn string_uppercase(_: &mut Interp, a: &[Value]) -> R {
  Ok(Value::str(text(a, 0)?.to_uppercase()))
}

fn js_trim(c: char) -> bool {
  c.is_whitespace() || c == '\u{feff}'
}

fn string_trim(_: &mut Interp, a: &[Value]) -> R {
  Ok(Value::str(text(a, 0)?.trim_matches(js_trim)))
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn string_repeat(_: &mut Interp, a: &[Value]) -> R {
  let n = number(a, 1)?.trunc().max(0.0);

  if n > 100_000_000.0 {
    return Err(Control::fail("Invalid string length"));
  }

  Ok(Value::str(text(a, 0)?.repeat(n as usize)))
}

fn clamp_index(position: f64, len: usize) -> usize {
  #[allow(clippy::cast_precision_loss)]
  let len_f = len as f64;
  let relative = position.trunc();
  let relative = if relative.is_nan() { 0.0 } else { relative };
  let at = if relative < 0.0 {
    (len_f + relative).max(0.0)
  } else {
    relative.min(len_f)
  };

  #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
  {
    at as usize
  }
}

fn string_slice(_: &mut Interp, a: &[Value]) -> R {
  let chars: Vec<char> = text(a, 0)?.chars().collect();
  let from = clamp_index(number(a, 1)?, chars.len());
  let to = clamp_index(number(a, 2)?, chars.len());

  Ok(Value::str(if from < to {
    chars[from..to].iter().collect::<String>()
  } else {
    String::new()
  }))
}

fn string_char_code_at(_: &mut Interp, a: &[Value]) -> R {
  let units: Vec<u16> = text(a, 0)?.encode_utf16().collect();
  let n = number(a, 1)?;

  #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
  let unit = if n >= 0.0 { units.get(n as usize) } else { None };

  Ok(Value::Num(unit.map_or(f64::NAN, |u| f64::from(*u))))
}

fn string_split(_: &mut Interp, a: &[Value]) -> R {
  let (s, sep) = (text(a, 0)?, text(a, 1)?);
  let parts: Vec<Value> = if sep.is_empty() {
    s.encode_utf16()
      .map(|u| Value::str(String::from_utf16_lossy(&[u])))
      .collect()
  } else {
    s.split(sep).map(Value::str).collect()
  };

  Ok(Value::list(parts))
}

fn string_split_once(_: &mut Interp, a: &[Value]) -> R {
  let (s, sep) = (text(a, 0)?, text(a, 1)?);

  Ok(match s.find(sep) {
    None => Value::none(),
    Some(at) => Value::some(Value::record(vec![
      ("before", Value::str(&s[..at])),
      ("after", Value::str(&s[at + sep.len()..])),
    ])),
  })
}

fn string_chars(_: &mut Interp, a: &[Value]) -> R {
  Ok(Value::list(
    text(a, 0)?.chars().map(|c| Value::str(c.to_string())).collect(),
  ))
}

fn float_to_string(_: &mut Interp, a: &[Value]) -> R {
  Ok(Value::str(crate::value::float_to_string(number(a, 0)?)))
}

fn to_string(_: &mut Interp, a: &[Value]) -> R {
  Ok(Value::str(arg(a, 0)?.coerce_string()))
}

fn identity(_: &mut Interp, a: &[Value]) -> R {
  Ok(arg(a, 0)?.clone())
}

fn int_to_base(_: &mut Interp, a: &[Value]) -> R {
  let n = number(a, 0)?;
  let radix = number(a, 1)?;

  #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
  let radix = radix as u32;

  if !(2..=36).contains(&radix) {
    return Err(Control::fail("toString() radix must be between 2 and 36"));
  }

  if !n.is_finite() || n.fract() != 0.0 {
    return Ok(Value::str(number_to_string(n)));
  }

  let negative = n < 0.0;
  let mut rest = n.abs();
  let mut digits = Vec::new();

  if rest == 0.0 {
    digits.push('0');
  }

  while rest >= 1.0 {
    let d = rest % f64::from(radix);

    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    digits.push(char::from_digit(d as u32, radix).unwrap_or('0'));
    rest = (rest / f64::from(radix)).floor();
  }

  if negative {
    digits.push('-');
  }

  Ok(Value::str(digits.iter().rev().collect::<String>()))
}

fn int_parse(_: &mut Interp, a: &[Value]) -> R {
  let s = text(a, 0)?;
  let body = s.strip_prefix('-').unwrap_or(s);

  if body.is_empty() || !body.bytes().all(|b| b.is_ascii_digit()) {
    return Ok(Value::none());
  }

  let n: f64 = s.parse().unwrap_or(f64::NAN);

  Ok(if n.abs() <= 9_007_199_254_740_991.0 {
    Value::some(Value::Num(if n == 0.0 { 0.0 } else { n }))
  } else {
    Value::none()
  })
}

fn float_to_int(_: &mut Interp, a: &[Value]) -> R {
  num1(a, f64::trunc)
}

fn float_atan2(_: &mut Interp, a: &[Value]) -> R {
  Ok(Value::Num(number(a, 0)?.atan2(number(a, 1)?)))
}

fn ref_new(_: &mut Interp, a: &[Value]) -> R {
  Ok(Value::Ref(Rc::new(RefCell::new(arg(a, 0)?.clone()))))
}

fn ref_get(_: &mut Interp, a: &[Value]) -> R {
  match arg(a, 0)? {
    Value::Ref(cell) => Ok(cell.borrow().clone()),
    _ => Err(Control::fail("not a Ref")),
  }
}

fn ref_set(_: &mut Interp, a: &[Value]) -> R {
  match arg(a, 0)? {
    Value::Ref(cell) => {
      *cell.borrow_mut() = arg(a, 1)?.clone();

      Ok(Value::unit())
    }
    _ => Err(Control::fail("not a Ref")),
  }
}

fn json_parse(_: &mut Interp, a: &[Value]) -> R {
  Ok(match json::parse(text(a, 0)?) {
    Ok(v) => Value::ok(v),
    Err(message) => Value::err(Value::str(message)),
  })
}

fn json_print(_: &mut Interp, a: &[Value]) -> R {
  Ok(Value::str(json::print(arg(a, 0)?)?))
}

fn json_lookup(_: &mut Interp, a: &[Value]) -> R {
  let key = text(a, 1)?;

  for field in arg(a, 0)?.list_items().unwrap_or_default() {
    if field.field("key").and_then(Value::as_str) == Some(key) {
      return Ok(
        field.field("value").cloned().map_or_else(Value::none, Value::some),
      );
    }
  }

  Ok(Value::none())
}
