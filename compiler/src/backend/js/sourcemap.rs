use crate::{
  backend::js::{json, print::Mapping},
  shared::source::SourceFile,
};

const BASE64: &[u8; 64] =
  b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn encode_vlq(value: i64, out: &mut String) {
  let mut rest = if value < 0 {
    (value.unsigned_abs() << 1) | 1
  } else {
    value.unsigned_abs() << 1
  };

  loop {
    let mut digit = rest & 0b1_1111;

    rest >>= 5;

    if rest > 0 {
      digit |= 0b10_0000;
    }

    out.push(char::from(BASE64[usize::try_from(digit).unwrap_or(0)]));

    if rest == 0 {
      break;
    }
  }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SourceMapV3 {
  pub file: String,
  pub sources: Vec<String>,
  pub sources_content: Vec<String>,
  pub names: Vec<String>,
  pub mappings: String,
}

impl SourceMapV3 {
  #[must_use]
  pub fn to_json(&self) -> String {
    let list = |items: &[String]| {
      let items: Vec<String> = items.iter().map(|s| json::string(s)).collect();

      format!("[{}]", items.join(","))
    };

    let content = if self.sources_content.is_empty() {
      format!("[{}]", vec!["null"; self.sources.len()].join(","))
    } else {
      list(&self.sources_content)
    };

    format!(
      "{{\"version\":3,\"file\":{},\"sources\":{},\"sourcesContent\":{},\"names\":{},\"mappings\":{}}}",
      json::string(&self.file),
      list(&self.sources),
      content,
      list(&self.names),
      json::string(&self.mappings),
    )
  }
}

pub struct OriginalLocation<'a> {
  pub file: &'a SourceFile,
  pub offset: usize,
  pub name: Option<&'a str>,
}

struct Segment {
  gen_column: usize,
  source: Option<(usize, usize, usize, Option<usize>)>,
}

#[derive(Default)]
pub struct SourceMapBuilder {
  sources: Vec<String>,
  sources_content: Vec<String>,
  names: Vec<String>,
  lines: Vec<Vec<Segment>>,
}

impl SourceMapBuilder {
  pub fn add_mapping(
    &mut self,
    gen_line: usize,
    gen_column: usize,
    source: Option<OriginalLocation<'_>>,
  ) {
    if self.lines.len() <= gen_line {
      self.lines.resize_with(gen_line + 1, Vec::new);
    }

    if self.lines[gen_line].iter().any(|s| s.gen_column == gen_column) {
      return;
    }

    let source = source.map(|loc| {
      let index = self.source_index(loc.file);
      let position = loc.file.position_at(loc.offset);
      let line_text = loc.file.line_text(position.line);
      let column = line_text[..position.column].encode_utf16().count();
      let name = loc.name.map(|name| self.name_index(name));

      (index, position.line, column, name)
    });

    self.lines[gen_line].push(Segment { gen_column, source });
  }

  fn source_index(&mut self, file: &SourceFile) -> usize {
    if let Some(i) = self.sources.iter().position(|s| s == file.name()) {
      return i;
    }

    self.sources.push(file.name().to_string());
    self.sources_content.push(file.text().to_string());
    self.sources.len() - 1
  }

  fn name_index(&mut self, name: &str) -> usize {
    if let Some(i) = self.names.iter().position(|n| n == name) {
      return i;
    }

    self.names.push(name.to_string());
    self.names.len() - 1
  }

  #[must_use]
  pub fn build(mut self) -> SourceMapV3 {
    let mut mappings = String::new();
    let (mut source, mut line, mut column, mut name) = (0i64, 0i64, 0i64, 0i64);

    for (i, segments) in self.lines.iter_mut().enumerate() {
      if i > 0 {
        mappings.push(';');
      }

      segments.sort_by_key(|s| s.gen_column);

      let mut gen_column = 0i64;

      for (j, segment) in segments.iter().enumerate() {
        if j > 0 {
          mappings.push(',');
        }

        let at = to_i64(segment.gen_column);

        encode_vlq(at - gen_column, &mut mappings);
        gen_column = at;

        if let Some((s, l, c, n)) = segment.source {
          let (s, l, c) = (to_i64(s), to_i64(l), to_i64(c));

          encode_vlq(s - source, &mut mappings);
          encode_vlq(l - line, &mut mappings);
          encode_vlq(c - column, &mut mappings);
          (source, line, column) = (s, l, c);

          if let Some(n) = n {
            let n = to_i64(n);

            encode_vlq(n - name, &mut mappings);
            name = n;
          }
        }
      }
    }

    SourceMapV3 {
      file: String::new(),
      sources: self.sources,
      sources_content: self.sources_content,
      names: self.names,
      mappings,
    }
  }
}

fn to_i64(n: usize) -> i64 {
  i64::try_from(n).unwrap_or(i64::MAX)
}

#[must_use]
pub fn from_mappings(mappings: &[Mapping], file: &SourceFile) -> SourceMapV3 {
  let mut builder = SourceMapBuilder::default();

  builder.source_index(file);

  for m in mappings {
    let source = m.origin.as_ref().map(|span| OriginalLocation {
      file,
      offset: span.start,
      name: m.name.as_deref(),
    });

    builder.add_mapping(m.gen_line, m.gen_column, source);
  }

  builder.build()
}
