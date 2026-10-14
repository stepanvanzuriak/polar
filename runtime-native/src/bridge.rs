use crate::{
  interp::{Control, Interp, R},
  json,
  value::Value,
};
use polar_compiler::core::ir::{CBridge, CBridgeOp};
use std::{
  io::{BufRead, BufReader, Read, Write},
  net::{TcpListener, TcpStream, ToSocketAddrs},
  time::Duration,
};

const DEFAULT_BASE: &str = "http://127.0.0.1:3000/_polar";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const CALL_TIMEOUT: Duration = Duration::from_secs(300);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_BODY: usize = 64 * 1024 * 1024;

fn failed(effect: &str, op: &str, status: u16, detail: &str) -> Control {
  Control::throw(
    "BridgeFailed",
    Value::record(vec![
      ("effect", Value::str(effect)),
      ("op", Value::str(op)),
      ("status", Value::Num(f64::from(status))),
      ("detail", Value::str(detail)),
    ]),
  )
}

fn object_get(value: &Value, key: &str) -> Option<Value> {
  let Value::Variant(v) = value else { return None };

  if v.name != "JObject" {
    return None;
  }

  for field in v.fields.first()?.list_items()? {
    if field.field("key").and_then(Value::as_str) == Some(key) {
      return field.field("value").cloned();
    }
  }

  None
}

fn string_of(value: &Value) -> Option<String> {
  match value {
    Value::Variant(v) if v.name == "JString" => {
      v.fields.first()?.as_str().map(str::to_string)
    }
    _ => None,
  }
}

fn encode(
  it: &mut Interp,
  m: usize,
  codec: Option<&'static polar_compiler::core::ir::CExpr>,
  value: &Value,
) -> Result<String, Control> {
  let Some(expr) = codec else { return Ok("null".to_string()) };
  let dict = it.eval_in(expr, m)?;
  let to_json = dict
    .field("to_json")
    .cloned()
    .ok_or_else(|| Control::fail("a bridge codec without `to_json`"))?;
  let json = it.apply(&to_json, vec![value.clone()])?;

  json::print(&json)
}

enum Decoded {
  Value(Value),
  Reject(String),
}

fn decode(
  it: &mut Interp,
  m: usize,
  codec: Option<&'static polar_compiler::core::ir::CExpr>,
  json: Value,
  path: &str,
) -> Result<Decoded, Control> {
  let Some(expr) = codec else { return Ok(Decoded::Value(Value::unit())) };
  let dict = it.eval_in(expr, m)?;
  let from_json = dict
    .field("from_json")
    .cloned()
    .ok_or_else(|| Control::fail("a bridge codec without `from_json`"))?;
  let result = it.apply(&from_json, vec![json, Value::str(path)])?;
  let Value::Variant(v) = &result else {
    return Err(Control::fail("a codec returned a non-Result"));
  };
  let payload = v.fields.first().cloned().unwrap_or_else(Value::unit);

  if v.name == "Ok" {
    return Ok(Decoded::Value(payload));
  }

  let at = payload.field("path").and_then(Value::as_str).unwrap_or("");
  let expected =
    payload.field("expected").and_then(Value::as_str).unwrap_or("");
  let found = payload.field("found").and_then(Value::as_str).unwrap_or("");
  let where_ = if at.is_empty() { String::new() } else { format!("{at}: ") };

  Ok(Decoded::Reject(format!("{where_}expected {expected}, found {found}")))
}

fn find_op(bridge: &'static CBridge, op: &str) -> Option<&'static CBridgeOp> {
  bridge.ops.iter().find(|o| o.op == op)
}

pub fn call(
  it: &mut Interp,
  m: usize,
  effect: &str,
  op: &str,
  args: &[Value],
) -> R {
  let ir = it.module_ir(m);
  let host = ir.host.as_deref();
  let bridge = ir
    .bridges
    .iter()
    .find(|b| b.effect == effect && Some(b.host.as_str()) == host)
    .ok_or_else(|| Control::fail(format!("`{effect}` is not bridged")))?;
  let bop = find_op(bridge, op)
    .ok_or_else(|| Control::fail(format!("`{effect}.{op}` is not bridged")))?;
  let mut encoded = Vec::new();

  for (i, codec) in bop.params.iter().enumerate() {
    let value = args.get(i).cloned().unwrap_or_else(Value::unit);

    encoded.push(encode(it, m, codec.as_ref(), &value)?);
  }

  let body = format!("{{\"args\":[{}]}}", encoded.join(","));
  let base =
    it.env_var("POLAR_BRIDGE").unwrap_or_else(|| DEFAULT_BASE.to_string());
  let url = format!(
    "{}/{}/{}",
    base.trim_end_matches('/'),
    percent(effect),
    percent(op)
  );
  let (status, text) = post(&url, &body).map_err(|e| {
    failed(
      effect,
      op,
      0,
      &format!("{e}; start the bridge server or set POLAR_BRIDGE (now {base})"),
    )
  })?;
  let reject = |detail: String| failed(effect, op, status, &detail);
  let message = json::parse(&text).map_err(|_| reject(text.clone()))?;

  if status != 200 {
    let detail =
      object_get(&message, "error").and_then(|e| string_of(&e)).unwrap_or(text);

    return Err(reject(detail));
  }

  if let Some(thrown) = object_get(&message, "throw") {
    let tag = object_get(&thrown, "tag")
      .and_then(|t| string_of(&t))
      .unwrap_or_default();
    let Some((_, codec)) = bop.throws.iter().find(|(t, _)| *t == tag) else {
      return Err(reject(format!("undeclared error {tag}")));
    };
    let raw = object_get(&thrown, "value")
      .unwrap_or_else(|| Value::variant("JNull", Vec::new()));

    return match decode(it, m, Some(codec), raw, "")? {
      Decoded::Value(v) => Err(Control::throw(tag, v)),
      Decoded::Reject(detail) => Err(reject(detail)),
    };
  }

  let raw = object_get(&message, "ok")
    .unwrap_or_else(|| Value::variant("JNull", Vec::new()));

  match decode(it, m, bop.ret.as_ref(), raw, "")? {
    Decoded::Value(v) => Ok(v),
    Decoded::Reject(detail) => Err(reject(detail)),
  }
}

fn percent(s: &str) -> String {
  let mut out = String::new();

  for b in s.bytes() {
    if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
      out.push(char::from(b));
    } else {
      out.push_str(&format!("%{b:02X}"));
    }
  }

  out
}

fn unpercent(s: &str) -> String {
  let bytes = s.as_bytes();
  let mut out = Vec::new();
  let mut i = 0;

  let hex = |b: u8| char::from(b).to_digit(16);

  while i < bytes.len() {
    if bytes[i] == b'%'
      && i + 2 < bytes.len()
      && let (Some(high), Some(low)) = (hex(bytes[i + 1]), hex(bytes[i + 2]))
    {
      out.push(u8::try_from(high * 16 + low).unwrap_or(0));
      i += 3;
    } else {
      out.push(bytes[i]);
      i += 1;
    }
  }

  String::from_utf8_lossy(&out).into_owned()
}

fn post(url: &str, body: &str) -> Result<(u16, String), String> {
  let rest = url
    .strip_prefix("http://")
    .ok_or_else(|| "only http:// bridge URLs are supported".to_string())?;
  let (authority, path) = rest
    .split_once('/')
    .map_or((rest, String::new()), |(a, p)| (a, format!("/{p}")));
  let address = if authority.contains(':') {
    authority.to_string()
  } else {
    format!("{authority}:80")
  };
  let unreachable =
    |e: &dyn std::fmt::Display| format!("cannot reach {authority}: {e}");
  let socket = address
    .to_socket_addrs()
    .map_err(|e| unreachable(&e))?
    .next()
    .ok_or_else(|| unreachable(&"no address"))?;
  let mut stream = TcpStream::connect_timeout(&socket, CONNECT_TIMEOUT)
    .map_err(|e| unreachable(&e))?;

  stream.set_read_timeout(Some(CALL_TIMEOUT)).map_err(|e| e.to_string())?;
  stream.set_write_timeout(Some(CALL_TIMEOUT)).map_err(|e| e.to_string())?;

  let request = format!(
    "POST {path} HTTP/1.1\r\nHost: {authority}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
    body.len()
  );

  stream.write_all(request.as_bytes()).map_err(|e| e.to_string())?;

  let mut reader = BufReader::new(stream);
  let (status, headers) = read_head(&mut reader)?;
  let chunked = headers
    .iter()
    .any(|(k, v)| k == "transfer-encoding" && v.contains("chunked"));
  let length = headers
    .iter()
    .find(|(k, _)| k == "content-length")
    .and_then(|(_, v)| v.parse::<usize>().ok());
  let bytes = if chunked {
    read_chunked(&mut reader)?
  } else if let Some(n) = length {
    if n > MAX_BODY {
      return Err(format!("response body of {n} bytes is too large"));
    }

    let mut buf = vec![0u8; n];

    reader.read_exact(&mut buf).map_err(|e| e.to_string())?;
    buf
  } else {
    let mut buf = Vec::new();

    reader.read_to_end(&mut buf).map_err(|e| e.to_string())?;
    buf
  };

  Ok((status, String::from_utf8_lossy(&bytes).into_owned()))
}

fn read_head<R: BufRead>(
  reader: &mut R,
) -> Result<(u16, Vec<(String, String)>), String> {
  let mut line = String::new();

  reader.read_line(&mut line).map_err(|e| e.to_string())?;

  let status = line
    .split_whitespace()
    .nth(1)
    .and_then(|s| s.parse().ok())
    .ok_or_else(|| format!("bad HTTP response: {}", line.trim()))?;
  let mut headers = Vec::new();

  loop {
    line.clear();

    if reader.read_line(&mut line).map_err(|e| e.to_string())? == 0 {
      break;
    }

    let trimmed = line.trim_end();

    if trimmed.is_empty() {
      break;
    }

    if let Some((k, v)) = trimmed.split_once(':') {
      headers.push((k.trim().to_ascii_lowercase(), v.trim().to_string()));
    }
  }

  Ok((status, headers))
}

fn read_chunked<R: BufRead>(reader: &mut R) -> Result<Vec<u8>, String> {
  let mut out = Vec::new();

  loop {
    let mut line = String::new();

    reader.read_line(&mut line).map_err(|e| e.to_string())?;

    let size =
      usize::from_str_radix(line.trim().split(';').next().unwrap_or("0"), 16)
        .map_err(|e| e.to_string())?;

    if size == 0 {
      break;
    }

    if out.len().saturating_add(size) > MAX_BODY {
      return Err("response body is too large".to_string());
    }

    let mut chunk = vec![0u8; size];

    reader.read_exact(&mut chunk).map_err(|e| e.to_string())?;
    out.extend(chunk);

    let mut crlf = [0u8; 2];

    reader.read_exact(&mut crlf).map_err(|e| e.to_string())?;
  }

  Ok(out)
}

pub fn serve(it: &mut Interp, port: u16) -> Result<(), String> {
  let listener = TcpListener::bind(("127.0.0.1", port))
    .map_err(|e| format!("cannot listen on port {port}: {e}"))?;
  let bound = listener.local_addr().map_err(|e| e.to_string())?;

  eprintln!("polar: serving bridges on http://{bound}/_polar");

  for stream in listener.incoming() {
    let Ok(stream) = stream else { continue };

    if let Err(e) = handle(it, stream) {
      eprintln!("polar: {e}");
    }
  }

  Ok(())
}

fn respond(
  stream: &mut TcpStream,
  status: u16,
  body: &str,
) -> Result<(), String> {
  let reason = match status {
    200 => "OK",
    400 => "Bad Request",
    404 => "Not Found",
    405 => "Method Not Allowed",
    413 => "Payload Too Large",
    _ => "Internal Server Error",
  };
  let text = format!(
    "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
    body.len()
  );

  stream.write_all(text.as_bytes()).map_err(|e| e.to_string())
}

fn error_body(message: &str) -> String {
  format!("{{\"error\":{}}}", crate::value::json_quote(message))
}

fn handle(it: &mut Interp, mut stream: TcpStream) -> Result<(), String> {
  stream.set_read_timeout(Some(REQUEST_TIMEOUT)).map_err(|e| e.to_string())?;
  stream.set_write_timeout(Some(REQUEST_TIMEOUT)).map_err(|e| e.to_string())?;

  let mut reader =
    BufReader::new(stream.try_clone().map_err(|e| e.to_string())?);
  let mut line = String::new();

  reader.read_line(&mut line).map_err(|e| e.to_string())?;

  let mut parts = line.split_whitespace();
  let (method, target) =
    (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
  let (method, target) = (method.to_string(), target.to_string());
  let mut length = 0usize;

  loop {
    line.clear();

    if reader.read_line(&mut line).map_err(|e| e.to_string())? == 0 {
      break;
    }

    let header = line.trim_end();

    if header.is_empty() {
      break;
    }

    if let Some((k, v)) = header.split_once(':')
      && k.trim().eq_ignore_ascii_case("content-length")
    {
      length = v.trim().parse().unwrap_or(0);
    }
  }

  if length > MAX_BODY {
    return respond(&mut stream, 413, &error_body("request body too large"));
  }

  let mut body = vec![0u8; length];

  reader.read_exact(&mut body).map_err(|e| e.to_string())?;

  let path = target.split('?').next().unwrap_or("");
  let Some(rest) = path.strip_prefix("/_polar/") else {
    return respond(&mut stream, 404, &error_body("not a bridged operation"));
  };
  let segments: Vec<&str> = rest.split('/').collect();
  let [effect, op] = segments.as_slice() else {
    return respond(&mut stream, 404, &error_body("not a bridged operation"));
  };

  if method != "POST" {
    return respond(
      &mut stream,
      405,
      &error_body("bridged operations take POST"),
    );
  }

  let (status, out) = dispatch(
    it,
    &unpercent(effect),
    &unpercent(op),
    &String::from_utf8_lossy(&body),
  );

  respond(&mut stream, status, &out)
}

fn dispatch(
  it: &mut Interp,
  effect: &str,
  op: &str,
  body: &str,
) -> (u16, String) {
  let m = it.main_module();
  let ir = it.module_ir(m);
  let host = ir.host.as_deref();
  let Some(bop) = ir
    .bridges
    .iter()
    .filter(|b| b.effect == effect && Some(b.via.as_str()) == host)
    .find_map(|b| find_op(b, op))
  else {
    return (404, error_body(&format!("no bridged operation {effect}.{op}")));
  };
  let Ok(request) = json::parse(body) else {
    return (400, error_body("malformed request"));
  };
  let raw: Vec<Value> = match object_get(&request, "args") {
    Some(Value::Variant(v)) if v.name == "JArray" => {
      v.fields.first().and_then(Value::list_items).unwrap_or_default()
    }
    _ => return (400, error_body("malformed request")),
  };

  if raw.len() != bop.params.len() {
    return (
      400,
      error_body(&format!("expected {} arguments", bop.params.len())),
    );
  }

  let mut args = Vec::new();

  for (i, (codec, json)) in bop.params.iter().zip(raw).enumerate() {
    match decode(it, m, codec.as_ref(), json, &format!("args[{i}]")) {
      Ok(Decoded::Value(v)) => args.push(v),
      Ok(Decoded::Reject(detail)) => return (400, error_body(&detail)),
      Err(c) => return internal(it, &c),
    }
  }

  let result = it.local_op(m, effect, op).and_then(|f| it.apply(&f, args));

  match result {
    Ok(value) => match encode(it, m, bop.ret.as_ref(), &value) {
      Ok(text) => (200, format!("{{\"ok\":{text}}}")),
      Err(c) => internal(it, &c),
    },
    Err(Control::Throw { tag, value }) => {
      match bop.throws.iter().find(|(t, _)| *t == tag) {
        Some((_, codec)) => match encode(it, m, Some(codec), &value) {
          Ok(text) => (
            200,
            format!(
              "{{\"throw\":{{\"tag\":{},\"value\":{text}}}}}",
              crate::value::json_quote(&tag)
            ),
          ),
          Err(c) => internal(it, &c),
        },
        None => internal(it, &Control::Throw { tag, value }),
      }
    }
    Err(c) => internal(it, &c),
  }
}

fn internal(it: &mut Interp, control: &Control) -> (u16, String) {
  it.report_control(control);

  (500, error_body("internal error"))
}

#[cfg(test)]
mod tests {
  use super::unpercent;

  #[test]
  fn unpercent_decodes_escapes_and_keeps_the_rest() {
    assert_eq!(unpercent("a%2Fb%41"), "a/bA");
    assert_eq!(unpercent("%a\u{e9}/%"), "%a\u{e9}/%");
    assert_eq!(unpercent("%zz%4"), "%zz%4");
  }
}
