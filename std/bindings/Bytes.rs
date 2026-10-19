use crate::{
  builtins::{bytes, number, text},
  interp::{Interp, R},
  value::Value,
};

#[allow(clippy::cast_precision_loss)]
fn clamp(position: f64, len: usize) -> usize {
  let position = if position.is_nan() { 0.0 } else { position.trunc() };

  #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
  {
    position.clamp(0.0, len as f64) as usize
  }
}

#[allow(clippy::cast_precision_loss)]
pub fn length(_: &mut Interp, a: &[Value]) -> R {
  Ok(Value::Num(bytes(a, 0)?.len() as f64))
}

pub fn slice(_: &mut Interp, a: &[Value]) -> R {
  let data = bytes(a, 0)?;
  let from = clamp(number(a, 1)?, data.len());
  let to = clamp(number(a, 2)?, data.len());

  Ok(Value::bytes(if from < to { &data[from..to] } else { &[] }))
}

#[allow(clippy::cast_precision_loss)]
pub fn index_of(_: &mut Interp, a: &[Value]) -> R {
  let (data, needle) = (bytes(a, 0)?, bytes(a, 1)?);
  let from = clamp(number(a, 2)?, data.len());
  let found = if needle.is_empty() {
    Some(from)
  } else {
    data[from..]
      .windows(needle.len())
      .position(|window| window == needle)
      .map(|at| at + from)
  };

  Ok(found.map_or_else(Value::none, |at| Value::some(Value::Num(at as f64))))
}

pub fn to_string(_: &mut Interp, a: &[Value]) -> R {
  Ok(match std::str::from_utf8(bytes(a, 0)?) {
    Ok(s) => Value::ok(Value::str(s)),
    Err(_) => Value::err(Value::str("bytes are not valid UTF-8")),
  })
}

pub fn of_string(_: &mut Interp, a: &[Value]) -> R {
  Ok(Value::bytes(text(a, 0)?.as_bytes()))
}

pub fn concat(_: &mut Interp, a: &[Value]) -> R {
  Ok(Value::bytes([bytes(a, 0)?, bytes(a, 1)?].concat()))
}

pub fn from_list(_: &mut Interp, a: &[Value]) -> R {
  let items = a.first().and_then(Value::list_items).unwrap_or_default();

  #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
  let data: Vec<u8> = items
    .iter()
    .map(|v| (v.as_num().unwrap_or(0.0) as i64 & 0xff) as u8)
    .collect();

  Ok(Value::bytes(data))
}

pub fn to_list(_: &mut Interp, a: &[Value]) -> R {
  Ok(Value::list(
    bytes(a, 0)?.iter().map(|b| Value::Num(f64::from(*b))).collect(),
  ))
}

pub fn equal(_: &mut Interp, a: &[Value]) -> R {
  Ok(Value::Bool(bytes(a, 0)? == bytes(a, 1)?))
}
