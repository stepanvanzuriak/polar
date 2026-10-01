use std::collections::HashMap;

use polar_plugin::{
  Diagnostic, Entry, Expansion, Module, Source, Span, Token, Zone, ZonePlugin,
  align,
};

use crate::types::{self, Type};

pub struct Schema;

struct Table {
  name: String,
  name_span: Span,
  alias: Option<(String, Span)>,
  host: Option<(String, Span)>,
  columns: Vec<Column>,
}

struct Column {
  span: Span,
  name: String,
  name_span: Span,
  ty: Type,
  primary: Option<Span>,
  references: Option<(String, Span, Span)>,
}

impl ZonePlugin for Schema {
  fn zone(&self) -> Zone {
    Zone {
      keyword: "schema".to_string(),
      after: "traits".to_string(),
      blank_between_entries: true,
    }
  }

  fn expand(&self, zone: Span, entries: &[Entry], module: &Module) -> Expansion {
    let mut out = Expansion::default();
    let tables: Vec<Table> =
      entries.iter().filter_map(|e| table(e, &mut out)).collect();

    if out.has_errors() || !imports(zone, &tables, module, &mut out) {
      return out;
    }

    let records = records(&tables, &mut out);

    for table in &tables {
      validate(table, &records, &mut out);
    }

    if out.has_errors() {
      return out;
    }

    for table in &tables {
      generate(table, &records[&table.name], &mut out);
    }

    out
  }

  fn print(&self, entries: &[Entry]) -> Vec<Vec<String>> {
    entries.iter().map(print).collect()
  }
}

fn table(entry: &Entry, out: &mut Expansion) -> Option<Table> {
  let lines = entry.lines();
  let header = &lines[0];
  let (named, host) = match header.as_slice() {
    [rest @ .., in_, host] if in_.is("KwIn") && host.is("Upper") => {
      (rest, Some((host.text.clone(), host.span)))
    }
    all => (all, None),
  };
  let alias = match named {
    [name] if name.is("Lower") => None,
    [name, as_, alias] if name.is("Lower") && as_.is("KwAs") && alias.is("Upper") => {
      Some((alias.text.clone(), alias.span))
    }
    _ => {
      out.error(
        Diagnostic::error("a table starts with its name alone on a line", header[0].span)
          .with_help(
            "write `posts`, `people as Person` to name its record type, or `posts in \
             Node` to keep it in memory on `Node`",
          ),
      );
      return None;
    }
  };
  let columns: Option<Vec<Column>> =
    lines[1..].iter().map(|line| column(line, out)).collect();

  Some(Table {
    name: header[0].text.clone(),
    name_span: header[0].span,
    alias,
    host,
    columns: columns?,
  })
}

fn column(line: &[&Token], out: &mut Expansion) -> Option<Column> {
  let name = line[0];
  let span = name.span.join(line[line.len() - 1].span);

  if !name.is("Lower") {
    out.error(
      Diagnostic::error("a column starts with its name", name.span)
        .with_help("write it like `title  String`"),
    );
    return None;
  }

  let mut at = 1;
  let Some(ty) = types::parse(line, &mut at) else {
    let found = line.get(at).map_or(span, |t| t.span);

    out.error(
      Diagnostic::error(format!("the column `{}` needs a type", name.text), found)
        .with_help("write it like `title  String`"),
    );
    return None;
  };
  let mut column = Column {
    span,
    name: name.text.clone(),
    name_span: name.span,
    ty,
    primary: None,
    references: None,
  };

  while let Some(token) = line.get(at) {
    match token.text.as_str() {
      "primary" if token.is("Lower") => {
        column.primary = Some(token.span);
        at += 1;
      }
      "references" if token.is("Lower") => match line.get(at + 1) {
        Some(table) if table.is("Lower") => {
          column.references =
            Some((table.text.clone(), table.span, token.span.join(table.span)));
          at += 2;
        }
        _ => {
          out.error(
            Diagnostic::error("`references` needs a table name", token.span)
              .with_help("write it like `references users`"),
          );
          return None;
        }
      },
      _ => {
        out.error(
          Diagnostic::error(format!("unknown column modifier `{}`", token.text), token.span)
            .with_help("a column can be `primary` or `references <table>`"),
        );
        return None;
      }
    }
  }

  Some(column)
}

fn imports(zone: Span, tables: &[Table], module: &Module, out: &mut Expansion) -> bool {
  let stored = tables.iter().any(|t| t.host.is_some());
  let missing: Vec<&str> = [
    ("Std.Id", ["Std", "Id"], true),
    ("Std.Json", ["Std", "Json"], true),
    ("Std.Table", ["Std", "Table"], stored),
  ]
  .into_iter()
  .filter(|(_, _, needed)| *needed)
  .map(|(name, path, _)| (name, path))
  .filter(|(_, path)| module.import(path).is_none())
  .map(|(name, _)| name)
  .collect();

  if missing.is_empty() {
    return true;
  }

  let names: Vec<String> = missing.iter().map(|m| format!("`{m}`")).collect();

  out.error(
    Diagnostic::error(format!("the `schema` zone needs {}", names.join(" and ")), zone)
      .with_help(format!("add {} to the `uses` zone", names.join(" and "))),
  );

  false
}

fn records(tables: &[Table], out: &mut Expansion) -> HashMap<String, String> {
  let mut records = HashMap::new();

  for (i, table) in tables.iter().enumerate() {
    if let Some(first) = tables[..i].iter().find(|t| t.name == table.name) {
      out.error(
        Diagnostic::error(
          format!("the table `{}` is declared more than once", table.name),
          table.name_span,
        )
        .with_secondary(first.name_span, "first declared here"),
      );
      continue;
    }

    let record = match &table.alias {
      Some((alias, _)) => Some(alias.clone()),
      None => singular(&table.name).map(|s| pascal(&s)),
    };

    let Some(record) = record else {
      out.error(
        Diagnostic::error(
          format!("can't name the record type of `{}`: it doesn't end in `s`", table.name),
          table.name_span,
        )
        .with_help(format!("name it with `as`, like `{} as Person`", table.name)),
      );
      continue;
    };

    records.insert(table.name.clone(), record);
  }

  records
}

fn singular(table: &str) -> Option<String> {
  if let Some(stem) = table.strip_suffix("ies") {
    return (!stem.is_empty()).then(|| format!("{stem}y"));
  }

  table.strip_suffix('s').filter(|s| !s.is_empty()).map(ToString::to_string)
}

fn pascal(snake: &str) -> String {
  snake
    .split('_')
    .map(|part| {
      let mut chars = part.chars();

      chars
        .next()
        .map(|c| c.to_uppercase().chain(chars).collect::<String>())
        .unwrap_or_default()
    })
    .collect()
}

fn id_of(ty: &Type) -> Option<&str> {
  ty.arg_of("Id").filter(|a| a.args.is_empty()).map(|a| a.name.as_str())
}

fn allowed(ty: &Type, nested: bool) -> bool {
  ["Int", "Float", "String", "Bool"].iter().any(|p| ty.is(p))
    || id_of(ty).is_some()
    || (!nested && ty.arg_of("Option").is_some_and(|t| allowed(t, true)))
}

fn validate(table: &Table, records: &HashMap<String, String>, out: &mut Expansion) {
  let Some(record) = records.get(&table.name) else { return };
  let mut primaries: Vec<&Column> = Vec::new();

  for (i, column) in table.columns.iter().enumerate() {
    if let Some(first) = table.columns[..i].iter().find(|c| c.name == column.name) {
      out.error(
        Diagnostic::error(
          format!("the column `{}` appears more than once in `{}`", column.name, table.name),
          column.name_span,
        )
        .with_secondary(first.name_span, "first declared here"),
      );
    }

    if !allowed(&column.ty, false) {
      out.error(
        Diagnostic::error(format!("the column `{}` can't have this type", column.name), column.ty.span)
          .with_help(
            "a column is `Int`, `Float`, `String`, `Bool` or `Id<T>`, or an \
             `Option<…>` of one of those",
          ),
      );
    }

    if let Some(primary) = column.primary {
      primaries.push(column);

      if id_of(&column.ty) != Some(record.as_str()) {
        out.error(
          Diagnostic::error(
            format!("the primary key of `{}` must have type `Id<{record}>`", table.name),
            column.ty.span,
          )
          .with_secondary(primary, "the primary key"),
        );
      }
    }

    if let Some((target, target_span, modifier)) = &column.references {
      let elsewhere = singular(target).map(|s| pascal(&s));
      let Some(target_record) = records.get(target).cloned().or(elsewhere) else {
        out.error(
          Diagnostic::error(format!("there is no table `{target}` in this schema"), *target_span)
            .with_help(
              "a column references a table in this `schema` zone, or one from \
               another module whose record type it names, like `Id<User>` for `users`",
            ),
        );
        continue;
      };
      let ty = column.ty.arg_of("Option").unwrap_or(&column.ty);

      if id_of(ty) != Some(target_record.as_str()) {
        out.error(
          Diagnostic::error(
            format!(
              "the column `{}` references `{target}`, so its type must be `Id<{target_record}>`",
              column.name
            ),
            column.ty.span,
          )
          .with_secondary(*modifier, "the reference")
          .with_help(format!("write `Id<{target_record}>`, or `Option<Id<{target_record}>>`")),
        );
      }
    }
  }

  match primaries.as_slice() {
    [] => out.error(
      Diagnostic::error(format!("the table `{}` has no primary key", table.name), table.name_span)
        .with_help(format!("add a column like `id  Id<{record}>  primary`")),
    ),
    [_] => {}
    [first, rest @ ..] => {
      for column in rest {
        out.error(
          Diagnostic::error(
            format!("the table `{}` has more than one primary key", table.name),
            column.name_span,
          )
          .with_secondary(first.name_span, "the first primary key"),
        );
      }
    }
  }
}

fn record_type(name: &str, columns: &[&Column], origin: Span) -> polar_plugin::Generated {
  let mut source = Source::new();

  source.push(name).push(" = { ");

  for (i, column) in columns.iter().enumerate() {
    if i > 0 {
      source.push(", ");
    }

    source
      .from(column.name_span, &column.name)
      .push(": ")
      .from(column.ty.span, &column.ty.text());
  }

  source.push(" } derive(Json, Eq)");
  source.finish("types", origin)
}

fn generate(table: &Table, record: &str, out: &mut Expansion) {
  let origin = table.alias.as_ref().map_or(table.name_span, |(_, s)| *s);
  let all: Vec<&Column> = table.columns.iter().collect();
  let insert: Vec<&Column> = table.columns.iter().filter(|c| c.primary.is_none()).collect();
  let effect = pascal(&table.name);
  let mut ops = Source::new();

  ops.push(&format!(
    "{effect} {{\n  find(id: Id<{record}>) -> Option<{record}>\n  all() -> List<{record}>\n  \
     insert(row: New{record}) -> {record}\n  update(row: {record}) -> Bool\n  \
     delete(id: Id<{record}>) -> Bool\n}}"
  ));

  let mut metadata = Source::new();

  metadata.push(&format!(
    "schema_{}: String = \"{}\"",
    table.name,
    describe(table, record).replace('\\', "\\\\").replace('"', "\\\"")
  ));

  out.emit(record_type(record, &all, origin));
  out.emit(record_type(&format!("New{record}"), &insert, origin));
  out.emit(metadata.finish("constants", table.name_span));
  out.emit(ops.finish("effects", table.name_span));

  if let Some((host, host_span)) = &table.host {
    in_memory(table, record, &effect, host, *host_span, out);
  }
}

fn in_memory(table: &Table, record: &str, effect: &str, host: &str, span: Span, out: &mut Expansion) {
  let cell = format!("{}_table", table.name);
  let Some(primary) = table.columns.iter().find(|c| c.primary.is_some()) else { return };
  let key = &primary.name;
  let fields: Vec<String> = table
    .columns
    .iter()
    .map(|c| {
      if c.primary.is_some() {
        format!("{}: Id(n)", c.name)
      } else {
        format!("{}: row.{}", c.name, c.name)
      }
    })
    .collect();
  let mut constant = Source::new();
  let mut binding = Source::new();

  constant.push(&format!("{cell}: Table<{record}> = Table.new()"));
  binding.push(&format!(
    "{effect} in {host} {{\n  \
     find(id) {{\n    Table.find({cell}, Id.value(id))\n  }}\n\n  \
     all() {{\n    Table.all({cell})\n  }}\n\n  \
     insert(row) {{\n    Table.insert({cell}, function(n) {{ {{ {} }} }})\n  }}\n\n  \
     update(row) {{\n    Table.update({cell}, Id.value(row.{key}), row)\n  }}\n\n  \
     delete(id) {{\n    Table.delete({cell}, Id.value(id))\n  }}\n}}",
    fields.join(", ")
  ));

  out.emit(constant.finish("constants", span));
  out.emit(binding.finish("binds", span));
}

fn describe(table: &Table, record: &str) -> String {
  let columns: Vec<String> = table
    .columns
    .iter()
    .map(|c| {
      let mut fields = vec![
        format!("\"name\":\"{}\"", c.name),
        format!("\"type\":\"{}\"", c.ty.text()),
      ];

      if c.primary.is_some() {
        fields.push("\"primary\":true".to_string());
      }

      if let Some((target, ..)) = &c.references {
        fields.push(format!("\"references\":\"{target}\""));
      }

      format!("{{{}}}", fields.join(","))
    })
    .collect();

  format!(
    "{{\"table\":\"{}\",\"record\":\"{record}\",\"columns\":[{}]}}",
    table.name,
    columns.join(",")
  )
}

fn print(entry: &Entry) -> Vec<String> {
  let lines = entry.lines();
  let header: Vec<&str> = lines[0].iter().map(|t| t.text.as_str()).collect();
  let rows: Vec<Vec<String>> = lines[1..]
    .iter()
    .map(|line| {
      let mut at = 1;
      let ty = types::parse(line, &mut at).map_or_else(String::new, |t| t.text());
      let rest: Vec<&str> = line[at.min(line.len())..].iter().map(|t| t.text.as_str()).collect();
      let mut row = vec![line[0].text.clone(), ty];

      if !rest.is_empty() {
        row.push(rest.join(" "));
      }

      row
    })
    .collect();
  let aligned = align(&rows);
  let header_line = lines[0][0].line;
  let mut out = vec![match entry.comments.iter().find(|c| c.line == header_line) {
    Some(comment) => format!("{} {}", header.join(" "), comment.text),
    None => header.join(" "),
  }];
  let line_of: Vec<usize> = lines[1..].iter().map(|l| l[0].line).collect();

  for (i, row) in aligned.into_iter().enumerate() {
    for comment in entry.comments.iter().filter(|c| {
      c.line < line_of[i] && (i == 0 || c.line > line_of[i - 1])
    }) {
      out.push(format!("  {}", comment.text));
    }

    let trailing = entry.comments.iter().find(|c| c.line == line_of[i]);

    match trailing {
      Some(comment) => out.push(format!("  {row} {}", comment.text)),
      None => out.push(format!("  {row}")),
    }
  }

  out
}
