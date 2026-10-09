use std::collections::{BTreeMap, BTreeSet};

use crate::{
  check::{Checker, decl::groups, effects::FirstUses},
  core::effects::EffectTable,
  shared::codes::DiagnosticCode::{BindCycle, EffectNotOnHost, NoHost},
  shared::diagnostic::{Diagnostic, Label},
  shared::source::Span,
  shared::text::quoted_list,
  types::ty::{Effects, Label as EffectLabel},
};

pub type HostSet = Option<BTreeSet<String>>;

type Node = (String, String);

type UsesOf = BTreeMap<Node, Vec<(String, Span)>>;

type BindRow = ((String, String, String), Effects, Vec<(EffectLabel, Span)>);

#[must_use]
pub fn effect_hosts(table: &EffectTable, effect: &str) -> HostSet {
  let info = table.effects.get(effect)?;
  let mut set: BTreeSet<String> = info.host.iter().cloned().collect();

  for bind in &table.binds {
    if bind.effect == effect {
      set.insert(bind.host.clone());
    }
  }

  Some(set)
}

#[must_use]
pub fn label_hosts(table: &EffectTable, label: &EffectLabel) -> HostSet {
  if label.is_builtin() { None } else { effect_hosts(table, &label.name) }
}

#[must_use]
pub fn row_hosts(table: &EffectTable, row: &Effects) -> HostSet {
  row
    .labels
    .iter()
    .fold(None, |acc, label| intersect(&acc, &label_hosts(table, label)))
}

#[must_use]
pub fn intersect(a: &HostSet, b: &HostSet) -> HostSet {
  match (a, b) {
    (None, other) | (other, None) => other.clone(),
    (Some(a), Some(b)) => Some(a.intersection(b).cloned().collect()),
  }
}

#[must_use]
pub fn show(set: &HostSet) -> String {
  match set {
    None => "every host".to_string(),
    Some(hosts) => {
      let names: Vec<&str> = hosts.iter().map(String::as_str).collect();

      format!("{{{}}}", names.join(", "))
    }
  }
}

#[must_use]
pub fn runs_on(set: &HostSet, host: &str) -> bool {
  set.as_ref().is_none_or(|hosts| hosts.contains(host))
}

pub(crate) struct Blame<'u> {
  pub(crate) what: String,
  pub(crate) at: Span,
  pub(crate) row: Option<Span>,
  pub(crate) uses: &'u FirstUses,
}

impl Checker<'_> {
  pub(crate) fn check_hosts(&mut self, row: &Effects, blame: &Blame<'_>) {
    let table = self.effects;

    if row_hosts(table, row) != Some(BTreeSet::new()) {
      return;
    }

    let mut labels: Vec<&EffectLabel> =
      row.labels.iter().filter(|l| !l.is_builtin()).collect();

    labels.sort_by_key(|l| blame.uses.position(l).unwrap_or(usize::MAX));
    let pair = labels.iter().enumerate().find_map(|(i, a)| {
      labels[i + 1..].iter().find_map(|b| {
        let both = intersect(&label_hosts(table, a), &label_hosts(table, b));

        (both == Some(BTreeSet::new())).then_some((*a, *b))
      })
    });
    let involved: Vec<&EffectLabel> = match pair {
      Some((a, b)) => vec![a, b],
      None => labels.clone(),
    };
    let key = |l: &EffectLabel| l.name.to_string();

    if let Some((a, b)) = pair {
      let reported = (key(a), key(b));

      if self.reported_hosts.contains(&reported) {
        return;
      }

      self.reported_hosts.push(reported);
    }

    let names: Vec<&str> = involved.iter().map(|l| &*l.name).collect();
    let message = if names.len() == 2 {
      format!("no host provides both {}", quoted_list(&names))
    } else {
      format!("no host provides all of {}", quoted_list(&names))
    };
    let mut diagnostic = Diagnostic::error(
      NoHost,
      format!("{} can't run anywhere", blame.what),
      Label::new(blame.at.clone()).with_message(message),
    );

    for label in &involved {
      let hosts = show(&label_hosts(table, label));
      let text = format!("`{}` runs on {hosts}", label.name);
      let at = blame.uses.get(label).cloned().or_else(|| blame.row.clone());

      if let Some(at) = at {
        diagnostic =
          diagnostic.with_secondary(Label::new(at).with_message(text));
      } else {
        diagnostic = diagnostic.with_note(text);
      }
    }

    let help = match names.as_slice() {
      [a, b] => format!(
        "split the work: call a function that uses `{a}` from one host and one \
         that uses `{b}` from the other"
      ),
      _ => "split the work so that each function uses effects one host \
            provides"
        .to_string(),
    };

    self.push(diagnostic.with_help(help));
  }

  pub(crate) fn check_bind_hosts(&mut self) {
    let (nodes, uses_of) = self.bind_uses();

    self.bind_cycles(&nodes, &uses_of);
  }

  fn bind_uses(&mut self) -> (Vec<Node>, UsesOf) {
    let table = self.effects;
    let rows: Vec<BindRow> = self
      .bind_rows
      .iter()
      .map(|(k, (row, uses))| (k.clone(), row.clone(), uses.clone()))
      .collect();
    let mut nodes: Vec<Node> = Vec::new();
    let mut uses_of: BTreeMap<Node, Vec<(String, Span)>> = BTreeMap::new();

    for ((effect, host, _), row, uses) in &rows {
      let node = (effect.clone(), host.clone());

      if !nodes.contains(&node) {
        nodes.push(node.clone());
      }

      for label in row.labels.iter().filter(|l| !l.is_builtin()) {
        let at = uses
          .iter()
          .find(|(l, _)| l == label)
          .map_or_else(|| self.bind_span(effect, host), |(_, s)| s.clone());

        if *label.name == **effect {
          self.push(
            Diagnostic::error(
              BindCycle,
              format!(
                "the binding of `{effect}` in `{host}` calls `{effect}` itself"
              ),
              Label::new(at),
            )
            .with_help(format!(
              "a binding must reach `{host}` some other way: an extern, or \
               another effect"
            )),
          );
        } else if runs_on(&label_hosts(table, label), host) {
          let list = uses_of.entry(node.clone()).or_default();

          if !list.iter().any(|(e, _)| *e == *label.name) {
            list.push((label.name.to_string(), at));
          }
        } else {
          let hosts = show(&label_hosts(table, label));

          self.push(
            Diagnostic::error(
              EffectNotOnHost,
              format!(
                "the binding of `{effect}` in `{host}` uses `{}`, which isn't \
                 available on `{host}`",
                label.name
              ),
              Label::new(at)
                .with_message(format!("`{}` runs on {hosts}", label.name)),
            )
            .with_help(format!(
              "bind `{}` in `{host}` too, or implement this without it",
              label.name
            )),
          );
        }
      }
    }

    (nodes, uses_of)
  }

  fn bind_cycles(
    &mut self,
    nodes: &[Node],
    uses_of: &BTreeMap<Node, Vec<(String, Span)>>,
  ) {
    let index = |n: &Node| nodes.iter().position(|m| m == n);
    let edges: Vec<Vec<usize>> = nodes
      .iter()
      .map(|node| {
        uses_of
          .get(node)
          .into_iter()
          .flatten()
          .filter_map(|(effect, _)| index(&(effect.clone(), node.1.clone())))
          .collect()
      })
      .collect();

    for group in groups(&edges) {
      if group.len() < 2 {
        continue;
      }

      let first = &nodes[group[0]];
      let mut path: Vec<String> = Vec::new();
      let mut at = first.clone();

      for _ in 0..=group.len() {
        path.push(format!("`{}`", at.0));

        let next = edges[index(&at).unwrap_or(0)]
          .iter()
          .find(|k| group.contains(k))
          .map(|&k| nodes[k].clone());

        match next {
          Some(next) if next == *first => {
            path.push(format!("`{}`", first.0));
            break;
          }
          Some(next) => at = next,
          None => break,
        }
      }

      let span = uses_of
        .get(first)
        .and_then(|u| u.first())
        .map_or_else(|| self.bind_span(&first.0, &first.1), |(_, s)| s.clone());

      self.push(
        Diagnostic::error(
          BindCycle,
          format!(
            "the bindings on `{}` call each other in a cycle: {}",
            first.1,
            path.join(" → ")
          ),
          Label::new(span),
        )
        .with_help(
          "break the cycle: one of them must reach the host another way",
        ),
      );
    }
  }

  fn bind_span(&self, effect: &str, host: &str) -> Span {
    self
      .effects
      .binds
      .iter()
      .find(|b| b.effect == effect && b.host == host)
      .and_then(|b| b.span.clone())
      .unwrap_or_else(|| Span::empty(std::sync::Arc::from("<bind>"), 0))
  }
}
