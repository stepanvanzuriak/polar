use std::fmt::Write as _;

use crate::{
  check::{Types, hosts::show},
  core::ir::CBind,
  types::print::print_scheme,
};

#[must_use]
pub fn dump_types(types: &Types) -> String {
  let mut out = String::new();

  for (name, scheme) in types.decls.iter().filter(|(n, _)| !n.starts_with('$'))
  {
    let printed = print_scheme(scheme);
    let _ = writeln!(out, "{name} : {printed}");
  }

  for ((effect, op), scheme) in &types.op_schemes {
    let _ = writeln!(out, "{effect}.{op} : {}", print_scheme(scheme));
  }

  out
}

#[must_use]
pub fn dump_hosts(types: &Types, binds: &[CBind]) -> String {
  let width = types.hosts.iter().map(|(n, _)| n.len()).max().unwrap_or(0);
  let mut out = String::new();

  for (name, hosts) in &types.hosts {
    let _ = writeln!(out, "{name:width$} : {}", show(hosts));
  }

  for bind in binds {
    if let Some(target) = &bind.via {
      let _ = writeln!(out, "{} in {} from {target}", bind.effect, bind.host);
    }
  }

  out
}
