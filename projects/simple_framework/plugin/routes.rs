use polar_plugin::{
  Diagnostic, Entry, Expansion, Function, Module, Source, Span, Token, Zone,
  ZonePlugin, align,
};

pub struct Routes;

const METHODS: [&str; 5] = ["GET", "POST", "PUT", "PATCH", "DELETE"];

#[derive(Clone, PartialEq, Eq)]
enum Segment {
  Lit(String),
  Param(String, Span),
}

struct Route {
  entry: Span,
  method: String,
  segments: Vec<Segment>,
  handler: String,
  handler_span: Span,
}

impl ZonePlugin for Routes {
  fn zone(&self) -> Zone {
    Zone {
      keyword: "routes".to_string(),
      after: "binds".to_string(),
      blank_between_entries: false,
    }
  }

  fn expand(&self, zone: Span, entries: &[Entry], module: &Module) -> Expansion {
    let mut out = Expansion::default();
    let routes: Vec<Route> =
      entries.iter().filter_map(|e| route(e, &mut out)).collect();

    if module.import(&["Std", "Http"]).is_none() {
      out.error(
        Diagnostic::error("the `routes` zone needs `Std.Http`", zone)
          .with_help("add `Std.Http` to the `uses` zone"),
      );
    }

    for (i, route) in routes.iter().enumerate() {
      check(route, module, &mut out);
      duplicate(route, &routes[..i], &mut out);
    }

    if out.has_errors() {
      return out;
    }

    for (i, route) in routes.iter().enumerate() {
      let handler = module.function(&route.handler).expect("checked above");

      out.emit(try_route(i, route, handler));
    }

    out.emit(router(routes.len(), zone));

    let mut export = Source::new();

    export.push("router");
    out.emit(export.finish("exports", zone));

    out
  }

  fn print(&self, entries: &[Entry]) -> Vec<Vec<String>> {
    let rows: Vec<Vec<String>> = entries
      .iter()
      .map(|entry| {
        let tokens = &entry.tokens;
        let arrow = tokens.iter().position(|t| t.is("Arrow"));

        match (tokens.first(), arrow) {
          (Some(method), Some(arrow)) if arrow > 1 => {
            let path = entry.text(tokens[1].span.join(tokens[arrow - 1].span));
            let target: Vec<&str> =
              tokens[arrow..].iter().map(|t| t.text.as_str()).collect();

            vec![method.text.clone(), path.to_string(), target.join(" ")]
          }
          _ => vec![entry.source.clone()],
        }
      })
      .collect();

    align(&rows).into_iter().map(|line| vec![line]).collect()
  }
}

fn adjacent(a: &Token, b: &Token) -> bool {
  a.span.end == b.span.start
}

fn route(entry: &Entry, out: &mut Expansion) -> Option<Route> {
  let tokens: Vec<&Token> = entry.tokens.iter().collect();

  if let Some(next) = tokens.iter().skip(1).find(|t| t.newline_before) {
    out.error(
      Diagnostic::error("a route is one line", next.span)
        .with_help("write it like `GET  /posts/:id  -> show`"),
    );
    return None;
  }

  let method = tokens[0];

  if !METHODS.contains(&method.text.as_str()) {
    out.error(
      Diagnostic::error(format!("`{}` is not an HTTP method", method.text), method.span)
        .with_help("a route starts with GET, POST, PUT, PATCH or DELETE"),
    );
    return None;
  }

  let mut at = 1;
  let segments = path(&tokens, &mut at, out)?;

  match (tokens.get(at), tokens.get(at + 1), tokens.get(at + 2)) {
    (Some(arrow), Some(handler), None) if arrow.is("Arrow") && handler.is("Lower") => {
      Some(Route {
        entry: entry.span,
        method: method.text.clone(),
        segments,
        handler: handler.text.clone(),
        handler_span: handler.span,
      })
    }
    (found, ..) => {
      let span = found.map_or(entry.span, |t| t.span);

      out.error(
        Diagnostic::error("expected `-> handler` after the path", span)
          .with_help("write it like `GET  /posts/:id  -> show`"),
      );
      None
    }
  }
}

fn path(tokens: &[&Token], at: &mut usize, out: &mut Expansion) -> Option<Vec<Segment>> {
  let bad = |out: &mut Expansion, span: Span, message: &str| {
    out.error(
      Diagnostic::error(message.to_string(), span)
        .with_help("a path is `/` or segments like `/posts/:id`, with no spaces"),
    );
  };
  let mut segments = Vec::new();

  let Some(first) = tokens.get(*at).filter(|t| t.is("Slash")) else {
    let span = tokens.get(*at).map_or(tokens[0].span, |t| t.span);

    bad(out, span, "a path starts with `/`");
    return None;
  };
  let mut last = *first;

  *at += 1;

  loop {
    let Some(next) = tokens.get(*at).copied().filter(|t| adjacent(last, t)) else {
      if last.is("Slash") && !segments.is_empty() {
        bad(out, last.span, "a path doesn't end in `/`");
        return None;
      }

      if let Some(next) = tokens.get(*at).filter(|t| !t.is("Arrow")) {
        bad(out, next.span, "a path has no spaces in it");
        return None;
      }

      return Some(segments);
    };

    match (last.is("Slash"), next.kind.as_str()) {
      (true, "Lower" | "Upper" | "Int") => segments.push(Segment::Lit(next.text.clone())),
      (true, "Colon") => {
        let Some(name) = tokens.get(*at + 1).filter(|t| t.is("Lower") && adjacent(next, t))
        else {
          bad(out, next.span, "`:` names a parameter, like `:id`");
          return None;
        };

        segments.push(Segment::Param(name.text.clone(), next.span.join(name.span)));
        *at += 1;
        last = name;
        *at += 1;
        continue;
      }
      (false, "Slash") => {}
      _ => {
        bad(out, next.span, &format!("`{}` can't appear in a path", next.text));
        return None;
      }
    }

    last = next;
    *at += 1;
  }
}

fn check(route: &Route, module: &Module, out: &mut Expansion) {
  let Some(handler) = module.function(&route.handler) else {
    out.error(
      Diagnostic::error(
        format!("there is no function `{}` in this module", route.handler),
        route.handler_span,
      )
      .with_help("a route's handler is a function in this module's `functions` zone"),
    );
    return;
  };

  for segment in &route.segments {
    let Segment::Param(name, span) = segment else { continue };

    if !handler.params.iter().any(|p| p.name == *name) {
      out.error(
        Diagnostic::error(
          format!("`{}` has no parameter `{name}` for `:{name}`", handler.name),
          *span,
        )
        .with_secondary(handler.span, "the handler"),
      );
    }
  }

  for param in &handler.params {
    let in_path = route
      .segments
      .iter()
      .any(|s| matches!(s, Segment::Param(n, _) if *n == param.name));
    let ty = param.ty.as_deref().unwrap_or("");
    let fine = if in_path { ty == "Int" || ty == "String" } else { ty == "Request" };

    if fine {
      continue;
    }

    let message = if in_path {
      format!(
        "`:{}` comes from the path, so `{}`'s parameter `{}` must be `Int` or `String`",
        param.name, handler.name, param.name
      )
    } else {
      format!(
        "`{}`'s parameter `{}` isn't in the path, so it must be a `Request`",
        handler.name, param.name
      )
    };

    out.error(
      Diagnostic::error(message, route.entry).with_secondary(param.span, "this parameter"),
    );
  }
}

fn shape(route: &Route) -> Vec<Option<&str>> {
  route
    .segments
    .iter()
    .map(|s| match s {
      Segment::Lit(text) => Some(text.as_str()),
      Segment::Param(..) => None,
    })
    .collect()
}

fn duplicate(route: &Route, before: &[Route], out: &mut Expansion) {
  if let Some(first) =
    before.iter().find(|r| r.method == route.method && shape(r) == shape(route))
  {
    out.error(
      Diagnostic::error(
        format!("another `{}` route already matches this path", route.method),
        route.entry,
      )
      .with_secondary(first.entry, "the first route"),
    );
  }
}

fn try_route(index: usize, route: &Route, handler: &Function) -> polar_plugin::Generated {
  let root = route.segments.is_empty().then(|| "\"\"".to_string());
  let patterns: Vec<String> = std::iter::once("\"\"".to_string())
    .chain(root)
    .chain(route.segments.iter().enumerate().map(|(i, s)| match s {
      Segment::Lit(text) => format!("\"{text}\""),
      Segment::Param(..) => format!("segment_{i}"),
    }))
    .collect();
  let param_of = |name: &str| {
    route.segments.iter().enumerate().find_map(|(i, s)| match s {
      Segment::Param(n, _) if n == name => Some(i),
      _ => None,
    })
  };
  let args: Vec<String> = handler
    .params
    .iter()
    .map(|p| match param_of(&p.name) {
      Some(i) if p.ty.as_deref() == Some("String") => format!("segment_{i}"),
      Some(_) => p.name.clone(),
      None => "request".to_string(),
    })
    .collect();
  let mut source = Source::new();

  source.push(&format!(
    "router_route_{index}(request: Request, segments: List<String>) {{\n  \
     if request.method == \"{}\" {{\n    match segments {{\n      [{}] -> ",
    route.method,
    patterns.join(", ")
  ));

  let ints: Vec<(usize, &str)> = handler
    .params
    .iter()
    .filter(|p| p.ty.as_deref() == Some("Int"))
    .filter_map(|p| param_of(&p.name).map(|i| (i, p.name.as_str())))
    .collect();

  for (i, name) in &ints {
    source.push(&format!("match Int.parse(segment_{i}) {{\n        Some({name}) -> "));
  }

  source
    .push("{\n          let response: Response = ")
    .from(route.handler_span, &route.handler)
    .push(&format!("({})\n\n          Some(response)\n        }}", args.join(", ")));

  for _ in &ints {
    source.push(",\n        None -> None,\n      }");
  }

  source.push(",\n      _ -> None,\n    }\n  } else {\n    None\n  }\n}");
  source.finish("functions", route.entry)
}

fn router(count: usize, origin: Span) -> polar_plugin::Generated {
  let mut body = String::from("None");

  for i in (0..count).rev() {
    body = format!(
      "match router_route_{i}(request, segments) {{\n    Some(response) -> Some(response),\n    None -> {body},\n  }}"
    );
  }

  let mut source = Source::new();

  source.push(&format!(
    "router(request: Request) {{\n  let segments = String.split(request.path, \"/\")\n\n  {body}\n}}"
  ));
  source.finish("functions", origin)
}
