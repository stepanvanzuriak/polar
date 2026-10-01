use polar_plugin::{
  Diagnostic, Entry, Expansion, Module, Source, Span, Zone, ZonePlugin, export,
};

struct Notes;

impl ZonePlugin for Notes {
  fn zone(&self) -> Zone {
    Zone {
      keyword: "notes".to_string(),
      after: "types".to_string(),
      blank_between_entries: false,
    }
  }

  fn expand(
    &self,
    _zone: Span,
    entries: &[Entry],
    _module: &Module,
  ) -> Expansion {
    let mut out = Expansion::default();

    for entry in entries {
      let name = &entry.tokens[0];
      let eq = entry.tokens.get(1);

      if !name.is("Lower")
        || !eq.is_some_and(|t| t.is("Eq"))
        || entry.tokens.len() < 3
      {
        out.error(
          Diagnostic::error("a note is `name = value`", entry.span)
            .with_help("write it like `greeting = \"hello\"`"),
        );
        continue;
      }

      let value =
        Span { start: entry.tokens[2].span.start, end: entry.span.end };
      let mut source = Source::new();

      source
        .from(name.span, &name.text)
        .push(": String = ")
        .from(value, entry.text(value));
      out.emit(source.finish("constants", entry.span));
    }

    out
  }

  fn print(&self, entries: &[Entry]) -> Vec<Vec<String>> {
    entries
      .iter()
      .map(|entry| {
        let mut line = String::new();
        let mut end = None;

        for token in &entry.tokens {
          if end.is_some_and(|e| e < token.span.start) {
            line.push(' ');
          }

          line.push_str(&token.text);
          end = Some(token.span.end);
        }

        vec![line]
      })
      .collect()
  }
}

export!(Notes);
