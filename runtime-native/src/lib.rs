#![allow(
  clippy::unnecessary_wraps,
  reason = "every native function has the one `NativeFn` signature"
)]
#![allow(
  clippy::missing_errors_doc,
  reason = "errors are `Control` values, described on the type"
)]
#![allow(
  clippy::many_single_char_names,
  reason = "argument slices are named by position: a, b"
)]
#![allow(
  clippy::format_push_string,
  clippy::format_collect,
  reason = "short strings built once"
)]

pub mod bindings;
pub mod bridge;
pub mod builtins;
pub mod interp;
pub mod json;
pub mod value;

pub use interp::{Control, Interp, RunOutcome};
pub use value::Value;
