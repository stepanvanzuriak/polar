mod library;

use std::{
  cell::RefCell,
  sync::{Mutex, OnceLock},
};

pub use polar_plugin::{Entry, Expansion, Module, Zone};

use crate::{
  shared::ice::ice,
  shared::source::SourceFile,
  syntax::ast::{Builtin, Decl, PluginId, Zone as AstZone, ZoneKind},
  syntax::lexer::{LexResult, token::TokenKind},
};
use library::Library;

struct Registered {
  library: usize,
  keyword: &'static str,
  after: Builtin,
  blank: bool,
}

#[derive(Default)]
struct Registry {
  libraries: Vec<Library>,
  zones: Vec<Registered>,
}

fn with_registry<T>(f: impl FnOnce(&mut Registry) -> T) -> T {
  static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();

  let mut guard = REGISTRY
    .get_or_init(|| Mutex::new(Registry::default()))
    .lock()
    .unwrap_or_else(std::sync::PoisonError::into_inner);

  f(&mut guard)
}

/// Loads the plugin library at `path`, once per process, and returns the
/// zones it defines.
///
/// # Errors
///
/// Fails if the library can't be loaded, speaks another plugin ABI, or
/// declares a zone the compiler can't place.
pub fn load(path: &str) -> Result<Vec<Zone>, String> {
  let ids = load_ids(path)?;

  Ok(with_registry(|r| {
    ids
      .iter()
      .map(|id| {
        let zone = &r.zones[usize::from(id.0)];

        Zone {
          keyword: zone.keyword.to_string(),
          after: zone.after.as_str().to_string(),
          blank_between_entries: zone.blank,
        }
      })
      .collect()
  }))
}

fn load_ids(path: &str) -> Result<Vec<PluginId>, String> {
  let loaded = with_registry(|r| {
    let index = r.libraries.iter().position(|l| l.path == path)?;

    Some(ids_of(r, index))
  });

  if let Some(ids) = loaded {
    return Ok(ids);
  }

  let library = Library::open(path)?;
  let zones = match library.call(&polar_plugin::Request::Zones)? {
    polar_plugin::Response::Zones(zones) => zones,
    polar_plugin::Response::Failed(message) => return Err(message),
    other => return Err(format!("`{path}` answered {other:?} to a zone list")),
  };

  with_registry(|r| {
    if let Some(index) = r.libraries.iter().position(|l| l.path == path) {
      return Ok(ids_of(r, index));
    }

    let index = r.libraries.len();
    let mut added = Vec::new();

    for zone in &zones {
      let after = Builtin::ALL
        .into_iter()
        .find(|b| b.as_str() == zone.after)
        .ok_or_else(|| {
        format!(
          "the zone `{}` in `{path}` goes after `{}`, which isn't a zone",
          zone.keyword, zone.after
        )
      })?;

      if Builtin::ALL.iter().any(|b| b.as_str() == zone.keyword) {
        return Err(format!(
          "`{path}` defines the zone `{}`, which is built in",
          zone.keyword
        ));
      }

      added.push(Registered {
        library: index,
        keyword: Box::leak(zone.keyword.clone().into_boxed_str()),
        after,
        blank: zone.blank_between_entries,
      });
    }

    r.zones.extend(added);
    r.libraries.push(library);
    Ok(ids_of(r, index))
  })
}

fn ids_of(r: &Registry, library: usize) -> Vec<PluginId> {
  r.zones
    .iter()
    .enumerate()
    .filter(|(_, z)| z.library == library)
    .map(|(i, _)| PluginId(u16::try_from(i).unwrap_or(u16::MAX)))
    .collect()
}

fn zone(r: &Registry, id: PluginId) -> &Registered {
  r.zones
    .get(usize::from(id.0))
    .unwrap_or_else(|| ice(format!("no zone plugin #{}", id.0), None))
}

#[must_use]
pub fn keyword(id: PluginId) -> &'static str {
  with_registry(|r| zone(r, id).keyword)
}

#[must_use]
pub fn after(id: PluginId) -> Builtin {
  with_registry(|r| zone(r, id).after)
}

#[must_use]
pub fn blank_between_entries(id: PluginId) -> bool {
  with_registry(|r| zone(r, id).blank)
}

#[must_use]
pub fn is_known(keyword: &str) -> bool {
  with_registry(|r| r.zones.iter().any(|z| z.keyword == keyword))
}

fn call(
  id: PluginId,
  request: &polar_plugin::Request,
) -> Result<polar_plugin::Response, String> {
  with_registry(|r| {
    let library = zone(r, id).library;

    r.libraries[library].call(request)
  })
}

pub(crate) fn expand(
  id: PluginId,
  zone: polar_plugin::Span,
  entries: Vec<Entry>,
  module: Module,
) -> Result<Expansion, String> {
  let keyword = keyword(id).to_string();
  let request =
    polar_plugin::Request::Expand { keyword, zone, entries, module };

  match call(id, &request)? {
    polar_plugin::Response::Expanded(expansion) => Ok(expansion),
    polar_plugin::Response::Failed(message) => Err(message),
    other => Err(format!("answered {other:?} to an expansion")),
  }
}

pub(crate) fn print(
  id: PluginId,
  entries: Vec<Entry>,
) -> Result<Vec<Vec<String>>, String> {
  let keyword = keyword(id).to_string();
  let count = entries.len();

  match call(id, &polar_plugin::Request::Print { keyword, entries })? {
    polar_plugin::Response::Printed(lines) if lines.len() == count => Ok(lines),
    polar_plugin::Response::Failed(message) => Err(message),
    other => Err(format!("answered {other:?} to printing {count} entries")),
  }
}

thread_local! {
  static ENABLED: RefCell<Vec<PluginId>> = const { RefCell::new(Vec::new()) };
}

struct Restore(Vec<PluginId>);

impl Drop for Restore {
  fn drop(&mut self) {
    let saved = std::mem::take(&mut self.0);

    ENABLED.with(|enabled| *enabled.borrow_mut() = saved);
  }
}

/// Runs `f` with the zones of the plugin libraries at `paths` enabled. A
/// library that fails to load enables nothing: call [`load`] first to report
/// why.
pub fn within<T>(paths: &[String], f: impl FnOnce() -> T) -> T {
  let ids: Vec<PluginId> =
    paths.iter().filter_map(|p| load_ids(p).ok()).flatten().collect();
  let saved = ENABLED.with(|enabled| enabled.replace(ids));
  let _restore = Restore(saved);

  f()
}

#[must_use]
pub fn enabled() -> Vec<PluginId> {
  ENABLED.with(|enabled| enabled.borrow().clone())
}

#[must_use]
pub fn generates_below(zone: ZoneKind, target: Builtin) -> bool {
  ZoneKind::Builtin(target) > zone
}

#[must_use]
pub(crate) fn entries(
  file: &SourceFile,
  lexed: &LexResult,
  zone: &AstZone,
) -> Vec<Entry> {
  let token = |kind: String, span: &crate::shared::source::Span, newline| {
    let position = file.position_at(span.start);

    polar_plugin::Token {
      kind,
      text: file.slice(span).to_string(),
      span: polar_plugin::Span { start: span.start, end: span.end },
      line: position.line,
      column: position.column,
      newline_before: newline,
    }
  };

  zone
    .decls
    .iter()
    .filter_map(|decl| match decl {
      Decl::Plugin(entry) => Some(entry),
      _ => None,
    })
    .map(|entry| {
      let span =
        polar_plugin::Span { start: entry.span.start, end: entry.span.end };
      let inside = |s: &crate::shared::source::Span| {
        s.start >= entry.span.start && s.end <= entry.span.end
      };
      let tokens = lexed
        .tokens
        .iter()
        .filter(|t| t.kind != TokenKind::Eof && inside(&t.span))
        .map(|t| token(format!("{:?}", t.kind), &t.span, t.newline_before))
        .collect();
      let comments = lexed
        .comments
        .iter()
        .filter(|c| inside(&c.span))
        .map(|c| token("Comment".to_string(), &c.span, true))
        .collect();

      Entry {
        span,
        source: file.slice(&entry.span).to_string(),
        tokens,
        comments,
      }
    })
    .collect()
}
