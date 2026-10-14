use super::ir::{CExpr, CExprKind, CModule, PrimOp, TypeSlot};
use crate::types::ty::Type;
use serde::{Deserialize, Serialize, de::DeserializeOwned};

const MAGIC: &[u8; 7] = b"POLARIR";
const VERSION: u32 = 1;
const HEADER_LEN: usize = MAGIC.len() + 4;

const FOOTER_MAGIC: &[u8; 8] = b"POLARPAY";
const FOOTER_LEN: usize = 8 + FOOTER_MAGIC.len();

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Bundle {
  pub main: CModule,
  pub modules: Vec<(String, CModule)>,
}

impl Bundle {
  #[must_use]
  pub fn module(&self, path: &str) -> Option<&CModule> {
    self.modules.iter().find(|(p, _)| p == path).map(|(_, m)| m)
  }
}

fn encode<T: Serialize>(value: &T) -> Vec<u8> {
  let body = serde_json::to_vec(value).expect("IR serializes to JSON");
  let mut out = Vec::with_capacity(HEADER_LEN + body.len());

  out.extend_from_slice(MAGIC);
  out.extend_from_slice(&VERSION.to_le_bytes());
  out.extend_from_slice(&body);

  out
}

fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, String> {
  if bytes.len() < HEADER_LEN || &bytes[..MAGIC.len()] != MAGIC {
    return Err("not a polar program (bad magic)".to_string());
  }

  let version = u32::from_le_bytes(
    bytes[MAGIC.len()..HEADER_LEN].try_into().expect("4 bytes"),
  );

  if version != VERSION {
    return Err(format!(
      "this file was built by {} polar (format {version}, supported {VERSION})",
      if version > VERSION { "a newer" } else { "an older" }
    ));
  }

  serde_json::from_slice(&bytes[HEADER_LEN..])
    .map_err(|e| format!("corrupt polar program: {e}"))
}

#[must_use]
pub fn to_bytes(module: &CModule) -> Vec<u8> {
  encode(module)
}

pub fn from_bytes(bytes: &[u8]) -> Result<CModule, String> {
  decode(bytes)
}

#[must_use]
pub fn bundle_to_bytes(bundle: &Bundle) -> Vec<u8> {
  encode(bundle)
}

pub fn bundle_from_bytes(bytes: &[u8]) -> Result<Bundle, String> {
  decode(bytes)
}

/// Appends `payload`, its length and a magic footer to `runtime`.
#[must_use]
pub fn pack(runtime: &[u8], payload: &[u8]) -> Vec<u8> {
  let mut out = Vec::with_capacity(runtime.len() + payload.len() + FOOTER_LEN);

  out.extend_from_slice(runtime);
  out.extend_from_slice(payload);
  out.extend_from_slice(&(payload.len() as u64).to_le_bytes());
  out.extend_from_slice(FOOTER_MAGIC);

  out
}

/// The payload `pack` appended, or `None` when `exe` has no footer.
#[must_use]
pub fn unpack(exe: &[u8]) -> Option<&[u8]> {
  let footer_at = exe.len().checked_sub(FOOTER_LEN)?;
  let (rest, footer) = exe.split_at(footer_at);

  if &footer[8..] != FOOTER_MAGIC {
    return None;
  }

  let len = u64::from_le_bytes(footer[..8].try_into().ok()?);
  let start = rest.len().checked_sub(usize::try_from(len).ok()?)?;

  Some(&rest[start..])
}

fn is_int(expr: &CExpr) -> bool {
  matches!(
    &expr.ty,
    Some(TypeSlot(Type::Con { name, args })) if args.is_empty() && &**name == "Int"
  )
}

fn check_int_ops(e: &mut CExpr) {
  if let CExprKind::Prim { op, args } = &mut e.kind
    && args.first().is_some_and(is_int)
  {
    *op = match *op {
      PrimOp::Add => PrimOp::IntAdd,
      PrimOp::Sub => PrimOp::IntSub,
      PrimOp::Mul => PrimOp::IntMul,
      PrimOp::Div => PrimOp::IntDiv,
      PrimOp::Mod => PrimOp::IntMod,
      other => other,
    };
  }

  e.for_each_child_mut(&mut check_int_ops);
}

/// Turns the type-dependent parts of the IR into explicit operations, so the
/// types can be dropped from the payload: `Int` arithmetic is checked.
pub fn lower_typed_ops(module: &mut CModule) {
  let bodies = module
    .decls
    .iter_mut()
    .map(|d| &mut d.body)
    .chain(
      module
        .impls
        .iter_mut()
        .flat_map(|i| i.methods.iter_mut().map(|m| &mut m.body)),
    )
    .chain(
      module
        .binds
        .iter_mut()
        .flat_map(|b| b.ops.iter_mut().map(|o| &mut o.body)),
    );

  for body in bodies {
    check_int_ops(body);
  }
}
