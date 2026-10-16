use crate::{
  builtins::{number, text},
  interp::{Control, Interp, R},
  value::Value,
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

fn to_hex(bytes: &[u8]) -> String {
  bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn digit(b: u8) -> Option<u8> {
  match b {
    b'0'..=b'9' => Some(b - b'0'),
    b'a'..=b'f' => Some(b - b'a' + 10),
    b'A'..=b'F' => Some(b - b'A' + 10),
    _ => None,
  }
}

fn from_hex(s: &str) -> Vec<u8> {
  s.as_bytes()
    .chunks_exact(2)
    .map_while(|pair| Some(digit(pair[0])? * 16 + digit(pair[1])?))
    .collect()
}

pub fn bytes(_: &mut Interp, a: &[Value]) -> R {
  #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
  let count = number(a, 0)?.max(0.0) as usize;
  let mut buf = vec![0u8; count];

  getrandom::getrandom(&mut buf)
    .map_err(|e| Control::fail(format!("no randomness available: {e}")))?;

  Ok(Value::str(to_hex(&buf)))
}

pub fn sha256(_: &mut Interp, a: &[Value]) -> R {
  Ok(Value::str(to_hex(&Sha256::digest(text(a, 0)?.as_bytes()))))
}

pub fn hmac_sha256(_: &mut Interp, a: &[Value]) -> R {
  let mut mac = Hmac::<Sha256>::new_from_slice(text(a, 0)?.as_bytes())
    .map_err(|e| Control::fail(e.to_string()))?;

  mac.update(text(a, 1)?.as_bytes());

  Ok(Value::str(to_hex(&mac.finalize().into_bytes())))
}

pub fn equal(_: &mut Interp, a: &[Value]) -> R {
  let (left, right) = (text(a, 0)?.as_bytes(), text(a, 1)?.as_bytes());

  Ok(Value::Bool(left.len() == right.len() && bool::from(left.ct_eq(right))))
}

pub fn base64url(_: &mut Interp, a: &[Value]) -> R {
  Ok(Value::str(URL_SAFE_NO_PAD.encode(text(a, 0)?.as_bytes())))
}

pub fn from_base64url(_: &mut Interp, a: &[Value]) -> R {
  let decoded = URL_SAFE_NO_PAD
    .decode(text(a, 0)?)
    .ok()
    .and_then(|raw| String::from_utf8(raw).ok());

  Ok(Value::maybe_str(
    decoded.as_deref().map(|d| d.strip_prefix('\u{feff}').unwrap_or(d)),
  ))
}

fn derive(a: &[Value]) -> Result<String, Control> {
  let (plain, salt) = (text(a, 0)?, from_hex(text(a, 1)?));
  #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
  let (ln, r, p) =
    (number(a, 2)? as u8, number(a, 3)? as u32, number(a, 4)? as u32);
  let params = scrypt::Params::new(ln, r, p, 32)
    .map_err(|e| Control::fail(format!("invalid scrypt parameters: {e}")))?;
  let mut out = [0u8; 32];

  scrypt::scrypt(plain.as_bytes(), &salt, &params, &mut out)
    .map_err(|e| Control::fail(format!("scrypt failed: {e}")))?;

  Ok(to_hex(&out))
}

pub fn scrypt(_: &mut Interp, a: &[Value]) -> R {
  derive(a).map(Value::str)
}

pub fn try_scrypt(_: &mut Interp, a: &[Value]) -> R {
  Ok(Value::maybe_str(derive(a).ok()))
}
