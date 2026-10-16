use crate::{
  builtins::{arg, number},
  interp::{Control, Interp, R},
  value::Value,
};
use std::time::{SystemTime, UNIX_EPOCH};

const DAY: i64 = 86_400_000;
const MAX_MS: f64 = 8.64e15;
const PARTS: [&str; 7] =
  ["year", "month", "day", "hour", "minute", "second", "milli"];

fn parts(values: [f64; 7]) -> Value {
  Value::record(PARTS.into_iter().zip(values.map(Value::Num)).collect())
}

pub fn now(_: &mut Interp, _: &[Value]) -> R {
  let ms = SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .map_or(0.0, |d| d.as_secs_f64() * 1000.0)
    .floor();

  Ok(Value::variant("Time", vec![Value::Num(ms)]))
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
  let z = z + 719_468;
  let era = z.div_euclid(146_097);
  let doe = z.rem_euclid(146_097);
  let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
  let y = yoe + era * 400;
  let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
  let mp = (5 * doy + 2) / 153;
  let d = doy - (153 * mp + 2) / 5 + 1;
  let m = if mp < 10 { mp + 3 } else { mp - 9 };

  (if m <= 2 { y + 1 } else { y }, m, d)
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
  let y = if m <= 2 { y - 1 } else { y };
  let era = y.div_euclid(400);
  let yoe = y.rem_euclid(400);
  let mp = (m + 9) % 12;
  let doy = (153 * mp + 2) / 5 + d - 1;
  let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;

  era * 146_097 + doe - 719_468
}

#[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
pub fn to_parts(_: &mut Interp, a: &[Value]) -> R {
  let ms = number(a, 0)?;

  if !ms.is_finite() || ms.abs() > MAX_MS {
    return Ok(parts([f64::NAN; 7]));
  }

  let ms = ms.trunc() as i64;
  let rest = ms.rem_euclid(DAY);
  let (y, m, d) = civil_from_days(ms.div_euclid(DAY));

  Ok(parts(
    [
      y,
      m,
      d,
      rest / 3_600_000,
      rest / 60_000 % 60,
      rest / 1000 % 60,
      rest % 1000,
    ]
    .map(|v| v as f64),
  ))
}

#[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
pub fn from_parts(_: &mut Interp, a: &[Value]) -> R {
  let parts = arg(a, 0)?;
  let get = |name: &str| {
    parts
      .field(name)
      .and_then(Value::as_num)
      .ok_or_else(|| Control::fail(format!("parts.{name} should be a number")))
  };
  let (year, month, day) = (get("year")?, get("month")?, get("day")?);
  let (hour, minute, second, milli) =
    (get("hour")?, get("minute")?, get("second")?, get("milli")?);

  if ![year, month, day, hour, minute, second, milli]
    .iter()
    .all(|v| v.is_finite())
  {
    return Ok(Value::Num(f64::NAN));
  }

  let month0 = month.trunc() as i64 - 1;
  let y = year.trunc() as i64 + month0.div_euclid(12);
  let m = month0.rem_euclid(12) + 1;
  let days = days_from_civil(y, m, 1) as f64 + day.trunc() - 1.0;
  let ms = days * DAY as f64
    + hour.trunc() * 3_600_000.0
    + minute.trunc() * 60_000.0
    + second.trunc() * 1000.0
    + milli.trunc();

  Ok(Value::Num(if ms.abs() > MAX_MS { f64::NAN } else { ms + 0.0 }))
}
