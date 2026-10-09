use super::{
  comments::{
    Attached, Attachments, NodeKey, Placement, attach, has_line_break,
    line_breaks,
  },
  doc::{
    Doc, anchor, breaking_suffix, closer, concat, fresh_line, group, hardline,
    if_break, join, line, line_suffix, nest, nil, render, softline,
    tail_anchor, text,
  },
  parens::{
    Side, binary_token, entry, exposes_record, operand_needs_parens,
    postfix_target_needs_parens, starts_with_minus, unary_operand_needs_parens,
  },
};
use crate::{
  shared::ice::ice,
  shared::{diagnostic::DiagnosticBag, source::SourceFile},
  syntax::ast::{
    Binary, BinaryOp, BindDecl, Block, Bound, Builtin, Call, ConstDecl,
    CtorDecl, Decl, Derive, EffectDecl, EffectRow, Else, ExportDecl, Expr,
    ExternDecl, FieldAccess, FieldInit, FieldType, FnDecl, FnType, HostDecl,
    If, ImplDecl, Import, Lambda, LetStmt, ListLit, Match, MatchArm, MethodSig,
    Module, Name, PCtor, PField, PList, PLit, PRecord, Param, PatLit, Pattern,
    Pipe, PluginId, Recipe, RecordLit, RecordType, Return, Stmt, StringLit,
    StringPart, Throw, TraitDecl, Try, TypeBody, TypeDecl, TypeExpr, TypeRef,
    Unary, VariantBody, Zone, ZoneKind,
    fields::{AsNode, NodeRef},
  },
  syntax::lexer::lex,
  syntax::lexer::token::{Comment, TokenKind},
  syntax::parser::precedence::{Assoc, PrecedenceTable},
  syntax::plugins,
};

pub const WIDTH: usize = 90;
const INDENT: u16 = 2;

#[must_use]
pub fn print_module(
  module: &Module,
  source: &str,
  comments: &[Comment],
  table: &PrecedenceTable,
) -> String {
  let attachments = attach(module, comments, source);
  let printer =
    Printer { source, table, comments: &attachments, flat: false, bare: None };
  let mut out = render(&printer.module(module), WIDTH);
  let trimmed = out.trim_end_matches('\n').len();

  out.truncate(trimmed);
  out.push('\n');

  out
}

#[derive(Clone, Copy)]
pub(crate) struct Printer<'a> {
  source: &'a str,
  table: &'a PrecedenceTable,
  comments: &'a Attachments,
  flat: bool,
  bare: Option<NodeKey>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BlockStyle {
  Broken,
  Group,
  Inline,
}

impl Printer<'_> {
  pub(crate) fn wrap(&self, node: NodeRef<'_>, doc: Doc) -> Doc {
    self.wrap_with(node, doc, false)
  }

  fn wrap_with(
    &self,
    node: NodeRef<'_>,
    doc: Doc,
    leading_on_the_line: bool,
  ) -> Doc {
    let attached = self.comments.get(node);

    if self.bare == Some(NodeKey::of(node)) {
      return doc;
    }

    if attached.is_empty() {
      return concat([anchor(), doc]);
    }

    let mut before = Vec::new();
    let mut after = Vec::new();
    let mut prev_end = node.span().end;

    for (i, comment) in attached.iter().enumerate() {
      let body = text(self.comment_text(comment));

      match comment.placement {
        Placement::Leading if leading_on_the_line => {
          before.push(line_suffix(concat([text(" "), body])));
        }
        Placement::Leading => {
          let next = attached[i + 1..]
            .iter()
            .find(|c| c.placement == Placement::Leading)
            .map_or_else(|| Self::anchor(node), |c| c.span.start);

          before.extend([fresh_line(), anchor(), body, hardline()]);

          if self.blank_between(comment.span.end, next) {
            before.push(hardline());
          }
        }
        Placement::Trailing => {
          after.push(breaking_suffix(concat([text(" "), body])));
          prev_end = comment.span.end;
        }
        Placement::TrailingOwnLine => {
          let blank = self.blank_between(prev_end, comment.span.start);

          let own_line =
            concat([hardline(), if blank { hardline() } else { nil() }, body]);

          after.push(match node {
            NodeRef::Zone(_) => own_line,
            _ => breaking_suffix(own_line),
          });
          prev_end = comment.span.end;
        }
        Placement::Dangling if prints_dangling(node) => {}
        Placement::AfterBracket => {}
        Placement::Before => {
          before.extend([fresh_line(), anchor(), body, hardline()]);
        }
        Placement::AfterOpen => {
          before.push(breaking_suffix(concat([text(" "), body])));
        }
        Placement::Dangling => {
          after.push(line_suffix(concat([text(" "), body])));
        }
      }
    }

    let after = concat(after);
    let after = match node {
      NodeRef::Zone(_) => nest(INDENT, after),
      _ => after,
    };

    concat([anchor()].into_iter().chain(before).chain([doc, after]))
  }

  fn only_before(&self, node: NodeRef<'_>) -> bool {
    self.comments.get(node).iter().all(|c| {
      matches!(
        c.placement,
        Placement::Before | Placement::Leading | Placement::AfterOpen
      )
    })
  }

  fn chainable(&self, node: NodeRef<'_>) -> bool {
    self.comments.get(node).iter().all(|c| {
      matches!(
        c.placement,
        Placement::Before
          | Placement::Leading
          | Placement::AfterOpen
          | Placement::Trailing
      )
    })
  }

  fn trailing(&self, node: NodeRef<'_>) -> Doc {
    concat(
      self
        .comments
        .get(node)
        .iter()
        .filter(|c| c.placement == Placement::Trailing)
        .map(|c| {
          breaking_suffix(concat([text(" "), text(self.comment_text(c))]))
        }),
    )
  }

  fn before<'n>(&self, nodes: impl IntoIterator<Item = NodeRef<'n>>) -> Doc {
    let mut comments: Vec<(usize, &Attached)> = nodes
      .into_iter()
      .flat_map(|node| {
        let start = node.span().start;

        self
          .comments
          .get(node)
          .iter()
          .filter(|c| c.placement != Placement::Trailing)
          .map(move |c| (start, c))
      })
      .collect();

    comments.sort_by_key(|(_, c)| c.span.start);

    let mut parts = Vec::new();

    for (i, (start, c)) in comments.iter().enumerate() {
      if c.placement == Placement::AfterOpen {
        let body = text(self.comment_text(c));

        parts.push(breaking_suffix(concat([text(" "), body])));
        continue;
      }

      let next = comments.get(i + 1).map_or(*start, |(_, n)| n.span.start);

      parts.extend([
        fresh_line(),
        anchor(),
        text(self.comment_text(c)),
        hardline(),
      ]);

      if c.placement == Placement::Leading
        && self.blank_between(c.span.end, next)
      {
        parts.push(hardline());
      }
    }

    concat(parts)
  }

  fn anchor(node: NodeRef<'_>) -> usize {
    match node {
      NodeRef::Module(m) => m.name.as_ref().map_or(0, |name| name.span.start),
      _ => node.span().start,
    }
  }

  fn comment_text(&self, comment: &Attached) -> String {
    let span = &comment.span;

    self.source[span.start..span.end].trim_end().to_string()
  }

  fn leading_start(&self, node: NodeRef<'_>) -> usize {
    self
      .comments
      .get(node)
      .iter()
      .find(|c| c.placement == Placement::Leading)
      .map_or(node.span().start, |c| c.span.start)
  }

  fn blank_between(&self, from: usize, to: usize) -> bool {
    from < to
      && to <= self.source.len()
      && line_breaks(&self.source[from..to]) >= 2
  }

  fn dangling(&self, node: NodeRef<'_>) -> Doc {
    let start = node.span().start;

    concat(
      self
        .comments
        .get(node)
        .iter()
        .filter(|c| c.placement == Placement::Dangling)
        .map(|c| {
          let body = text(self.comment_text(c));

          if has_line_break(&self.source[start..c.span.start]) {
            concat([hardline(), body])
          } else {
            line_suffix(concat([text(" "), body]))
          }
        }),
    )
  }

  fn has_dangling(&self, node: NodeRef<'_>) -> bool {
    self.comments.get(node).iter().any(|c| c.placement == Placement::Dangling)
  }

  fn empty(&self, node: NodeRef<'_>, open: &str, close: &str) -> Doc {
    if self.has_dangling(node) {
      concat([
        text(open),
        nest(INDENT, self.dangling(node)),
        hardline(),
        closing(close),
      ])
    } else {
      text(format!("{open}{close}"))
    }
  }

  fn open(&self, node: NodeRef<'_>, bracket: &str) -> Doc {
    let comments = self
      .comments
      .get(node)
      .iter()
      .filter(|c| c.placement == Placement::AfterBracket)
      .map(|c| {
        breaking_suffix(concat([text(" "), text(self.comment_text(c))]))
      });

    concat([text(bracket)].into_iter().chain(comments))
  }

  fn lone_tail(&self, node: NodeRef<'_>, tail: String) -> Doc {
    let comments = self
      .comments
      .get(node)
      .iter()
      .filter(|c| c.placement == Placement::Dangling)
      .map(|c| concat([text(self.comment_text(c)), hardline()]));

    concat([anchor()].into_iter().chain(comments).chain([text(tail)]))
  }

  fn empty_args(&self, node: NodeRef<'_>) -> Doc {
    if self.has_dangling(node) { self.empty(node, "(", ")") } else { nil() }
  }

  pub(crate) fn lines_with(
    &self,
    items: Vec<(NodeRef<'_>, Doc)>,
    always_blank: bool,
    first: &Doc,
  ) -> Doc {
    let mut parts = Vec::new();
    let mut prev_end = None;

    for (node, doc) in items {
      match prev_end {
        None => parts.push(first.clone()),
        Some(end) => {
          parts.push(hardline());

          if always_blank || self.blank_between(end, self.leading_start(node)) {
            parts.push(hardline());
          }
        }
      }

      parts.push(doc);
      prev_end = Some(node.span().end);
    }

    concat(parts)
  }

  fn module(&self, m: &Module) -> Doc {
    let node = m.as_node();
    let header =
      m.name.as_ref().map(|name| text(format!("module {}", name.text)));
    let zones = m.zones.iter().map(|zone| self.zone(zone));
    let mut parts: Vec<Doc> = header.into_iter().chain(zones).collect();

    if parts.is_empty() {
      let comments = self
        .comments
        .get(node)
        .iter()
        .filter(|c| c.placement == Placement::Dangling)
        .map(|c| text(self.comment_text(c)));

      return self.wrap(node, join(comments, &hardline()));
    }

    let end = nest(INDENT, concat([fresh_line(), tail_anchor()]));

    parts = vec![
      join(parts, &concat([nest(INDENT, hardline()), hardline()])),
      self.dangling(node),
      end,
    ];

    self.wrap(node, concat(parts))
  }

  fn zone(&self, z: &Zone) -> Doc {
    let node = z.as_node();
    let always_blank = match z.kind {
      ZoneKind::Builtin(builtin) => matches!(
        builtin,
        Builtin::Traits
          | Builtin::Types
          | Builtin::Effects
          | Builtin::Binds
          | Builtin::Functions
          | Builtin::Impls
      ),
      ZoneKind::Plugin(id) => plugins::blank_between_entries(id),
    };
    let items = match z.kind {
      ZoneKind::Builtin(_) => {
        z.decls.iter().map(|decl| (decl.as_node(), self.decl(decl))).collect()
      }
      ZoneKind::Plugin(id) => self.plugin_entries(id, z),
    };
    let body = concat([
      self.lines_with(items, always_blank, &hardline()),
      self.dangling(node),
    ]);

    self.wrap(node, concat([text(z.kind.as_str()), nest(INDENT, body)]))
  }

  fn plugin_entries<'z>(
    &self,
    id: PluginId,
    z: &'z Zone,
  ) -> Vec<(NodeRef<'z>, Doc)> {
    let file = SourceFile::new("fmt.px", self.source);
    let lexed = lex(&file, &mut DiagnosticBag::default());
    let entries = plugins::entries(&file, &lexed, z);
    let printed = plugins::print(id, entries.clone())
      .unwrap_or_else(|_| entries.iter().map(polar_plugin::reindent).collect());

    z.decls
      .iter()
      .zip(printed)
      .map(|(decl, lines)| {
        let doc = concat(lines.into_iter().enumerate().map(|(i, line)| {
          let body = line.trim_start();
          let depth = u16::try_from(line.len() - body.len()).unwrap_or(0);

          if i == 0 {
            text(body)
          } else {
            nest(depth, concat([hardline(), text(body)]))
          }
        }));

        (decl.as_node(), self.wrap(decl.as_node(), doc))
      })
      .collect()
  }

  fn decl(&self, decl: &Decl) -> Doc {
    match decl {
      Decl::Import(import) => self.import(import),
      Decl::Trait(t) => self.trait_decl(t),
      Decl::Type(ty) => self.type_decl(ty),
      Decl::Const(c) => self.const_decl(c),
      Decl::Fn(f) => self.fn_decl(f),
      Decl::Impl(i) => self.impl_decl(i),
      Decl::Export(export) => self.export(export),
      Decl::Host(h) => self.host_decl(h),
      Decl::Effect(e) => self.effect_decl(e),
      Decl::Extern(e) => self.extern_decl(e),
      Decl::Bind(b) => self.bind_decl(b),
      Decl::Plugin(p) => ice("a plugin entry outside its zone", Some(&p.span)),
    }
  }

  fn host_decl(&self, h: &HostDecl) -> Doc {
    self.wrap(h.as_node(), name(&h.name))
  }

  fn effect_decl(&self, e: &EffectDecl) -> Doc {
    let node = e.as_node();
    let native = if e.native { "native " } else { "" };
    let host =
      e.host.as_ref().map_or_else(String::new, |h| format!(" in {}", h.text));
    let head = text(format!("{native}{}{host} ", e.name.text));
    let ops = e.ops.iter().map(|m| (m.as_node(), self.method_sig(m))).collect();

    self.wrap(node, concat([head, self.braced(node, ops, false)]))
  }

  fn bind_decl(&self, b: &BindDecl) -> Doc {
    let node = b.as_node();
    let force = if b.force { "force " } else { "" };

    if let Some(target) = &b.from {
      let bridge = format!(
        "{force}{} in {} from {}",
        b.effect.text, b.host.text, target.text
      );

      return self.wrap(node, text(bridge));
    }

    let head = text(format!("{force}{} in {} ", b.effect.text, b.host.text));

    if let Some(module) = &b.module {
      return self.wrap(node, concat([head, text("= "), self.string(module)]));
    }

    let ops = b.ops.iter().map(|f| (f.as_node(), self.fn_decl(f))).collect();

    self.wrap(node, concat([head, self.braced(node, ops, true)]))
  }

  fn extern_decl(&self, e: &ExternDecl) -> Doc {
    let signature = concat([
      name(&e.name),
      self.params(e.as_node(), &e.params),
      self.signature_tail(Some(&e.return_type), e.effects.as_ref()),
    ]);
    let target = concat([
      line(),
      text("= "),
      self.string(&e.module),
      text(" "),
      name(&e.export),
    ]);

    self.wrap(e.as_node(), group(concat([signature, nest(INDENT, target)])))
  }

  fn trait_decl(&self, t: &TraitDecl) -> Doc {
    let node = t.as_node();
    let head = text(format!("{}<{}> ", t.name.text, t.param.text));
    let methods =
      t.methods.iter().map(|m| (m.as_node(), self.method_sig(m))).collect();
    let recipe = t.recipe.as_ref().map_or_else(nil, |r| self.recipe(r));

    self.wrap(node, concat([head, self.braced(node, methods, false), recipe]))
  }

  fn braced(
    &self,
    node: NodeRef<'_>,
    items: Vec<(NodeRef<'_>, Doc)>,
    always_blank: bool,
  ) -> Doc {
    let after_bracket = self
      .comments
      .get(node)
      .iter()
      .any(|c| c.placement == Placement::AfterBracket);

    if items.is_empty() && !after_bracket && !self.has_dangling(node) {
      return text("{}");
    }

    concat([
      self.open(node, "{"),
      nest(
        INDENT,
        concat([
          self.lines_with(items, always_blank, &hardline()),
          self.dangling(node),
        ]),
      ),
      hardline(),
      closing("}"),
    ])
  }

  fn method_sig(&self, m: &MethodSig) -> Doc {
    let doc = concat([
      name(&m.name),
      self.params(m.as_node(), &m.params),
      self.signature_tail(Some(&m.return_type), m.effects.as_ref()),
    ]);

    self.wrap(m.as_node(), doc)
  }

  fn recipe(&self, r: &Recipe) -> Doc {
    let node = r.as_node();
    let cases =
      r.cases.iter().map(|f| (f.as_node(), self.fn_decl(f))).collect();

    concat([
      text(" "),
      self
        .wrap(node, concat([text("derive "), self.braced(node, cases, true)])),
    ])
  }

  fn impl_decl(&self, i: &ImplDecl) -> Doc {
    let node = i.as_node();
    let head =
      concat([name(&i.trait_name), text(" for "), self.type_ref(&i.target)]);
    let head = group(concat([head, self.bounds(&i.bounds)]));
    let methods =
      i.methods.iter().map(|f| (f.as_node(), self.fn_decl(f))).collect();

    self.wrap(node, concat([head, text(" "), self.braced(node, methods, true)]))
  }

  fn bounds(&self, bounds: &[Bound]) -> Doc {
    if bounds.is_empty() {
      return nil();
    }

    let list = bounds.iter().enumerate().map(|(i, b)| {
      let keyword = if i == 0 { "where " } else { "" };

      self.wrap(
        b.as_node(),
        text(format!("{keyword}{}<{}>", b.trait_name.text, b.var.text)),
      )
    });

    nest(INDENT, concat([line(), join(list, &text(", "))]))
  }

  fn method_list(methods: Option<&Vec<Name>>) -> Doc {
    let Some(methods) = methods else { return nil() };

    if methods.is_empty() {
      return text(" {}");
    }

    let names = methods.iter().map(name).collect();

    concat([text(" "), group(Self::list(text("{"), names, "}", true))])
  }

  fn import(&self, import: &Import) -> Doc {
    let path: Vec<&str> = import.path.iter().map(|n| n.text.as_str()).collect();
    let mut out = path.join(".");

    if let Some(alias) = &import.alias {
      out.push_str(" as ");
      out.push_str(&alias.text);
    }

    self.wrap(
      import.as_node(),
      concat([text(out), Self::method_list(import.methods.as_ref())]),
    )
  }

  fn export(&self, export: &ExportDecl) -> Doc {
    self.wrap(
      export.as_node(),
      concat([name(&export.name), Self::method_list(export.methods.as_ref())]),
    )
  }

  fn type_decl(&self, t: &TypeDecl) -> Doc {
    let mut head = t.name.text.clone();

    if !t.params.is_empty() {
      let params: Vec<&str> =
        t.params.iter().map(|p| p.text.as_str()).collect();

      head.push('<');
      head.push_str(&params.join(", "));
      head.push('>');
    }

    let body = match &t.body {
      TypeBody::Alias(ty) => concat([text(" = "), self.type_expr(ty)]),
      TypeBody::Variants(v) if v.ctors.is_empty() => nil(),
      TypeBody::Variants(v) => concat([text(" ="), self.variants(v)]),
    };

    let derive = t.derive.as_ref().map_or_else(nil, |d| self.derive(d));

    self.wrap(t.as_node(), group(concat([text(head), body, derive])))
  }

  fn variants(&self, v: &VariantBody) -> Doc {
    let lone = v.ctors.len() == 1 && v.ctors[0].args.is_empty();
    let mut parts = Vec::new();

    for (i, ctor) in v.ctors.iter().enumerate() {
      let broken = concat([line(), text("| ")]);

      parts.push(match (i, lone) {
        (0, true) => if_break(broken, text(" | ")),
        (0, false) => if_break(broken, text(" ")),
        _ => broken,
      });
      parts.push(self.ctor_decl(ctor));
    }

    self.wrap_with(v.as_node(), nest(INDENT, concat(parts)), true)
  }

  fn ctor_decl(&self, c: &CtorDecl) -> Doc {
    let doc = if c.args.is_empty() {
      concat([name(&c.name), self.empty_args(c.as_node())])
    } else {
      let args = c.args.iter().map(|a| self.type_expr(a)).collect();

      concat([
        name(&c.name),
        self.delimited(c.as_node(), "(", args, ")", false),
      ])
    };

    self.wrap(c.as_node(), doc)
  }

  fn derive(&self, d: &Derive) -> Doc {
    let names: Vec<&str> = d.names.iter().map(|n| n.text.as_str()).collect();

    concat([
      text(" "),
      self.wrap(d.as_node(), text(format!("derive({})", names.join(", ")))),
    ])
  }

  fn const_decl(&self, c: &ConstDecl) -> Doc {
    let ty = c
      .ty
      .as_ref()
      .map_or_else(nil, |ty| concat([text(": "), self.type_expr(ty)]));
    let doc = concat([name(&c.name), ty, text(" = "), self.expr(&c.value)]);

    self.wrap(c.as_node(), doc)
  }

  fn fn_decl(&self, f: &FnDecl) -> Doc {
    let signature = concat([
      name(&f.name),
      self.params(f.as_node(), &f.params),
      self.signature_tail(f.return_type.as_ref(), f.effects.as_ref()),
    ]);
    let signature = if f.bounds.is_empty() {
      signature
    } else {
      concat([signature, group(self.bounds(&f.bounds))])
    };
    let doc =
      concat([signature, text(" "), self.block(&f.body, BlockStyle::Broken)]);

    self.wrap(f.as_node(), doc)
  }

  fn params(&self, node: NodeRef<'_>, params: &[Param]) -> Doc {
    let docs = params.iter().map(|p| self.param(p)).collect();

    group(Self::list(self.open(node, "("), docs, ")", false))
  }

  fn param(&self, p: &Param) -> Doc {
    let head = match &p.pattern {
      Some(pattern) => self.pattern(pattern),
      None => name(&p.name),
    };
    let doc = match &p.ty {
      Some(ty) => concat([head, text(": "), self.type_expr(ty)]),
      None => head,
    };

    self.wrap(p.as_node(), doc)
  }

  fn signature_tail(
    &self,
    ret: Option<&TypeExpr>,
    effects: Option<&EffectRow>,
  ) -> Doc {
    let Some(ret) = ret else { return nil() };

    let captures = effects.is_some()
      && matches!(ret, TypeExpr::Fn(f) if f.effects.is_none());
    let ret = self.type_expr(ret);
    let ret = if captures { parens(ret) } else { ret };
    let effects = effects.map_or_else(nil, |e| self.effect_row(e));

    concat([text(" -> "), ret, effects])
  }

  pub(crate) fn type_expr(&self, t: &TypeExpr) -> Doc {
    match t {
      TypeExpr::Ref(r) => self.type_ref(r),
      TypeExpr::Var(v) => self.wrap(v.as_node(), name(&v.name)),
      TypeExpr::Fn(f) => self.fn_type(f),
      TypeExpr::Record(r) => self.record_type(r),
      TypeExpr::Invalid(i) => {
        ice("the formatter reached an invalid type", Some(&i.span))
      }
    }
  }

  fn type_ref(&self, r: &TypeRef) -> Doc {
    let doc = if r.args.is_empty() {
      name(&r.name)
    } else {
      let args = r.args.iter().map(|a| self.type_expr(a));

      concat([name(&r.name), text("<"), join(args, &text(", ")), text(">")])
    };

    self.wrap(r.as_node(), doc)
  }

  fn fn_type(&self, f: &FnType) -> Doc {
    let params = f.params.iter().map(|p| self.type_expr(p)).collect();
    let doc = concat([
      text("function"),
      group(Self::list(self.open(f.as_node(), "("), params, ")", false)),
      self.signature_tail(Some(&f.ret), f.effects.as_ref()),
    ]);

    self.wrap(f.as_node(), doc)
  }

  fn record_type(&self, r: &RecordType) -> Doc {
    let node = r.as_node();

    if r.fields.is_empty() && r.tail.is_none() {
      return self.wrap(node, self.empty(node, "{", "}"));
    }

    let fields: Vec<Doc> =
      r.fields.iter().map(|f| self.field_type(f)).collect();
    let has_fields = !fields.is_empty();
    let mut inner = join(fields, &concat([text(","), line()]));

    inner = match &r.tail {
      Some(tail) if has_fields => concat([
        inner,
        if_break(concat([text(","), line()]), text(" ")),
        tail_anchor(),
        text(format!("| {}", tail.text)),
      ]),
      Some(tail) => self.lone_tail(node, format!("| {}", tail.text)),
      None => concat([inner, if_break(text(","), nil())]),
    };

    let doc = group(concat([
      self.open(node, "{"),
      nest(INDENT, concat([line(), inner])),
      line(),
      closing("}"),
    ]));

    self.wrap(node, doc)
  }

  fn field_type(&self, f: &FieldType) -> Doc {
    self.wrap(
      f.as_node(),
      concat([name(&f.name), text(": "), self.type_expr(&f.ty)]),
    )
  }

  fn effect_row(&self, e: &EffectRow) -> Doc {
    let node = e.as_node();

    let row = if e.entries.is_empty() && e.tail.is_none() {
      self.empty(node, "{", "}")
    } else {
      let entries: Vec<Doc> =
        e.entries.iter().map(|r| self.type_ref(r)).collect();
      let has_entries = !entries.is_empty();
      let mut inner = join(entries, &concat([text(","), line()]));

      inner = match &e.tail {
        Some(tail) if has_entries => concat([
          inner,
          if_break(concat([text(","), line()]), text(" ")),
          tail_anchor(),
          text(format!("| {}", tail.text)),
        ]),
        Some(tail) => self.lone_tail(node, format!("| {}", tail.text)),
        None => concat([inner, if_break(text(","), nil())]),
      };

      group(concat([
        self.open(node, "{"),
        nest(INDENT, concat([softline(), inner])),
        softline(),
        closing("}"),
      ]))
    };

    concat([text(" "), self.wrap(node, concat([text("/ "), row]))])
  }

  fn block(&self, b: &Block, style: BlockStyle) -> Doc {
    let mut items: Vec<(NodeRef<'_>, Doc)> = Vec::new();

    for stmt in &b.stmts {
      let later = !items.is_empty();

      items.push((stmt.as_node(), self.stmt(stmt, later)));
    }

    let later = !items.is_empty();

    items.push((b.result.as_node(), self.statement_expr(&b.result, later)));

    let brk = match style {
      BlockStyle::Broken => hardline(),
      BlockStyle::Group | BlockStyle::Inline => line(),
    };
    let body = self.lines_with(items, false, &brk);
    let doc = concat([
      self.open(b.as_node(), "{"),
      nest(INDENT, body),
      brk,
      closing("}"),
    ]);
    let doc = if style == BlockStyle::Group { group(doc) } else { doc };

    self.wrap(b.as_node(), doc)
  }

  fn stmt(&self, stmt: &Stmt, later: bool) -> Doc {
    match stmt {
      Stmt::Let(l) => self.let_stmt(l),
      Stmt::Expr(e) => {
        self.wrap(e.as_node(), self.statement_expr(&e.expr, later))
      }
    }
  }

  fn statement_expr(&self, e: &Expr, later: bool) -> Doc {
    if later && starts_with_minus(e) {
      self.parenthesized(e)
    } else {
      self.expr(e)
    }
  }

  fn parenthesized(&self, e: &Expr) -> Doc {
    let node = e.as_node();
    let bare = Printer { bare: Some(NodeKey::of(node)), ..*self };

    self.wrap(node, parens(bare.expr(e)))
  }

  fn let_stmt(&self, l: &LetStmt) -> Doc {
    let ty = l
      .ty
      .as_ref()
      .map_or_else(nil, |ty| concat([text(": "), self.type_expr(ty)]));
    let doc = concat([
      text("let "),
      self.pattern(&l.pattern),
      ty,
      text(" = "),
      self.expr(&l.value),
    ]);

    self.wrap(l.as_node(), doc)
  }

  pub(crate) fn expr(&self, e: &Expr) -> Doc {
    match e {
      Expr::Int(l) => self.wrap(l.as_node(), text(&l.raw)),
      Expr::Float(l) => self.wrap(l.as_node(), text(&l.raw)),
      Expr::Bool(l) => {
        self.wrap(l.as_node(), text(if l.value { "true" } else { "false" }))
      }
      Expr::String(s) => self.string(s),
      Expr::Var(v) => self.wrap(v.as_node(), name(&v.name)),
      Expr::Field(f) => self.field_access(f),
      Expr::Call(c) => self.call(c),
      Expr::Pipe(p) => self.pipe(p),
      Expr::Binary(b) => self.binary(b),
      Expr::Unary(u) => self.unary(u),
      Expr::Record(r) => self.record_lit(r),
      Expr::List(l) => self.list_lit(l),
      Expr::Lambda(l) => self.lambda(l),
      Expr::Block(b) => self.block(b, BlockStyle::Group),
      Expr::If(i) => self.wrap(i.as_node(), group(self.if_chain(i))),
      Expr::Match(m) => self.match_expr(m),
      Expr::Throw(t) => self.throw_expr(t),
      Expr::Return(r) => self.return_expr(r),
      Expr::Try(t) => self.try_expr(t),
      Expr::Invalid(i) => {
        ice("the formatter reached an invalid expression", Some(&i.span))
      }
    }
  }

  pub(crate) fn string(&self, s: &StringLit) -> Doc {
    let mut out = String::from("\"");

    for part in &s.parts {
      match part {
        StringPart::Text(t) => out.push_str(&t.raw),
        StringPart::Interp(i) => {
          let flat = Printer { flat: true, ..*self };

          out.push_str("#{");
          out.push_str(&render(&group(flat.expr(&i.expr)), usize::MAX));
          out.push('}');
        }
      }
    }

    out.push('"');

    self.wrap(s.as_node(), text(out))
  }

  fn postfix_target(&self, e: &Expr) -> Doc {
    if postfix_target_needs_parens(e) {
      self.parenthesized(e)
    } else {
      self.expr(e)
    }
  }

  fn field_access(&self, f: &FieldAccess) -> Doc {
    let doc =
      concat([self.postfix_target(&f.target), text("."), name(&f.field)]);

    self.wrap(f.as_node(), doc)
  }

  fn call(&self, c: &Call) -> Doc {
    let args = c.args.iter().map(|a| self.expr(a)).collect();
    let doc = concat([
      self.postfix_target(&c.callee),
      group(Self::list(self.open(c.as_node(), "("), args, ")", false)),
    ]);

    self.wrap(c.as_node(), doc)
  }

  fn operand(&self, parent: TokenKind, child: &Expr, side: Side) -> Doc {
    let entry = entry(self.table, parent);

    if operand_needs_parens(entry, child, side, self.table) {
      self.parenthesized(child)
    } else {
      self.expr(child)
    }
  }

  fn pipe(&self, p: &Pipe) -> Doc {
    let token = TokenKind::PipeOp;
    let mut chain = vec![p];

    if entry(self.table, token).assoc == Assoc::Left {
      while let Expr::Pipe(inner) = &*chain[chain.len() - 1].left {
        if !self.chainable(inner.as_node()) {
          break;
        }

        chain.push(inner);
      }
    }

    let innermost = chain[chain.len() - 1];
    let first = concat([
      self.before(chain[1..].iter().map(|inner| inner.as_node())),
      self.operand(token, &innermost.left, Side::Left),
    ]);
    let outer = p.as_node();
    let stages = chain.iter().rev().map(|stage| {
      let node = stage.as_node();
      let trailing = if NodeKey::of(node) == NodeKey::of(outer) {
        nil()
      } else {
        self.trailing(node)
      };

      concat([
        line(),
        text("|> "),
        self.operand(token, &stage.right, Side::Right),
        trailing,
      ])
    });

    let doc = group(concat([first, nest(INDENT, concat(stages))]));

    self.wrap(p.as_node(), doc)
  }

  fn binary(&self, b: &Binary) -> Doc {
    if matches!(b.op, BinaryOp::And | BinaryOp::Or) {
      return self.logical_chain(b);
    }

    let token = binary_token(b.op);
    let doc = concat([
      self.operand(token, &b.left, Side::Left),
      text(format!(" {} ", b.op.as_str())),
      self.operand(token, &b.right, Side::Right),
    ]);

    self.wrap(b.as_node(), doc)
  }

  fn logical_chain(&self, b: &Binary) -> Doc {
    let token = binary_token(b.op);
    let mut chain = vec![b];

    if entry(self.table, token).assoc == Assoc::Right {
      while let Expr::Binary(inner) = &*chain[chain.len() - 1].right {
        if inner.op != b.op || !self.only_before(inner.as_node()) {
          break;
        }

        chain.push(inner);
      }
    }

    let op = text(format!("{} ", b.op.as_str()));
    let last = chain[chain.len() - 1];
    let mut rest = Vec::new();

    for link in &chain[1..] {
      rest.extend([
        line(),
        op.clone(),
        self.before([link.as_node()]),
        self.operand(token, &link.left, Side::Left),
      ]);
    }

    rest.extend([line(), op, self.operand(token, &last.right, Side::Right)]);

    let first = self.operand(token, &b.left, Side::Left);
    let doc = group(concat([first, nest(INDENT, concat(rest))]));

    self.wrap(b.as_node(), doc)
  }

  fn unary(&self, u: &Unary) -> Doc {
    let operand = if unary_operand_needs_parens(&u.operand, self.table) {
      self.parenthesized(&u.operand)
    } else {
      self.expr(&u.operand)
    };

    self.wrap(u.as_node(), concat([text(u.op.as_str()), operand]))
  }

  fn record_lit(&self, r: &RecordLit) -> Doc {
    let node = r.as_node();

    if r.spread.is_none() && r.fields.is_empty() {
      return self.wrap(node, self.empty(node, "{", "}"));
    }

    let spread = r.spread.iter().map(|s| concat([text(".."), self.expr(s)]));
    let fields = r.fields.iter().map(|f| self.field_init(f));

    self.wrap(
      node,
      self.delimited(node, "{", spread.chain(fields).collect(), "}", true),
    )
  }

  fn list_lit(&self, l: &ListLit) -> Doc {
    let node = l.as_node();
    let items = l.items.iter().map(|i| self.expr(i));
    let tail = l.tail.iter().map(|t| concat([text(".."), self.expr(t)]));

    self.wrap(
      node,
      self.delimited(node, "[", items.chain(tail).collect(), "]", false),
    )
  }

  fn field_init(&self, f: &FieldInit) -> Doc {
    self.wrap(
      f.as_node(),
      concat([name(&f.name), text(": "), self.expr(&f.value)]),
    )
  }

  fn lambda(&self, l: &Lambda) -> Doc {
    let doc = concat([
      text("function"),
      self.params(l.as_node(), &l.params),
      self.signature_tail(l.return_type.as_ref(), l.effects.as_ref()),
      text(" "),
      self.block(&l.body, BlockStyle::Group),
    ]);

    self.wrap(l.as_node(), doc)
  }

  fn if_chain(&self, i: &If) -> Doc {
    let cond = if exposes_record(&i.cond) {
      self.parenthesized(&i.cond)
    } else {
      self.expr(&i.cond)
    };
    let else_doc = match i.else_branch.as_deref() {
      Some(Else::Block(b)) => {
        concat([text(" else "), self.block(b, BlockStyle::Inline)])
      }
      Some(Else::If(inner)) => concat([
        text(" else "),
        self.wrap(inner.as_node(), self.if_chain(inner)),
      ]),
      None => nil(),
    };

    concat([
      text("if "),
      cond,
      text(" "),
      self.block(&i.then_branch, BlockStyle::Inline),
      else_doc,
    ])
  }

  fn match_expr(&self, m: &Match) -> Doc {
    let subjects = m.subjects.iter().map(|subject| {
      if exposes_record(subject) {
        self.parenthesized(subject)
      } else {
        self.expr(subject)
      }
    });
    let head = concat([
      text("match "),
      join(subjects, &text(", ")),
      text(" "),
      self.open(m.as_node(), "{"),
    ]);

    self.wrap(m.as_node(), self.arms(head, &m.arms))
  }

  fn arms(&self, head: Doc, arms: &[MatchArm]) -> Doc {
    if self.flat {
      let arms = arms.iter().map(|arm| self.match_arm(arm));

      return concat([head, text(" "), join(arms, &text(", ")), text(" }")]);
    }

    let items = arms
      .iter()
      .enumerate()
      .map(|(i, arm)| {
        let block_body = matches!(arm.body, Expr::Block(_));
        let next_negative = arms.get(i + 1).is_some_and(|next| {
          matches!(
            next.rows.first().and_then(|row| row.first()),
            Some(Pattern::Lit(PLit { negative: true, .. }))
          )
        });
        let comma =
          if block_body && !next_negative { nil() } else { text(",") };

        (arm.as_node(), concat([self.match_arm(arm), comma]))
      })
      .collect();

    concat([
      head,
      nest(INDENT, self.lines_with(items, false, &hardline())),
      hardline(),
      if arms.is_empty() { closer(0, text("}")) } else { closing("}") },
    ])
  }

  fn return_expr(&self, r: &Return) -> Doc {
    let bare = matches!(&*r.value, Expr::Record(lit) if lit.fields.is_empty() && lit.spread.is_none() && lit.span == r.span);
    let doc = if bare {
      text("return")
    } else {
      concat([text("return "), self.expr(&r.value)])
    };

    self.wrap(r.as_node(), doc)
  }

  fn throw_expr(&self, t: &Throw) -> Doc {
    self.wrap(t.as_node(), concat([text("throw "), self.expr(&t.value)]))
  }

  fn try_expr(&self, t: &Try) -> Doc {
    let style = if self.flat { BlockStyle::Inline } else { BlockStyle::Broken };
    let head = concat([
      text("try "),
      self.block(&t.body, style),
      text(" catch "),
      self.open(t.as_node(), "{"),
    ]);

    self.wrap(t.as_node(), self.arms(head, &t.arms))
  }

  fn match_arm(&self, arm: &MatchArm) -> Doc {
    let rows = arm
      .rows
      .iter()
      .map(|row| join(row.iter().map(|p| self.pattern(p)), &text(", ")));
    let guard = arm
      .guard
      .as_ref()
      .map_or_else(nil, |guard| concat([text(" if "), self.expr(guard)]));

    self.wrap(
      arm.as_node(),
      concat([
        join(rows, &text(" | ")),
        guard,
        text(" -> "),
        self.expr(&arm.body),
      ]),
    )
  }

  fn pattern(&self, p: &Pattern) -> Doc {
    match p {
      Pattern::Wildcard(w) => self.wrap(w.as_node(), text("_")),
      Pattern::Var(v) => self.wrap(v.as_node(), name(&v.name)),
      Pattern::Lit(l) => self.pattern_lit(l),
      Pattern::Ctor(c) => self.pattern_ctor(c),
      Pattern::Record(r) => self.pattern_record(r),
      Pattern::List(l) => self.pattern_list(l),
      Pattern::Or(o) => self.wrap(
        o.as_node(),
        join(o.alternatives.iter().map(|p| self.pattern(p)), &text(" | ")),
      ),
      Pattern::Invalid(i) => {
        ice("the formatter reached an invalid pattern", Some(&i.span))
      }
    }
  }

  fn pattern_lit(&self, l: &PLit) -> Doc {
    let lit = match &l.lit {
      PatLit::Int(i) => self.wrap(i.as_node(), text(&i.raw)),
      PatLit::Float(f) => self.wrap(f.as_node(), text(&f.raw)),
      PatLit::Bool(b) => {
        self.wrap(b.as_node(), text(if b.value { "true" } else { "false" }))
      }
      PatLit::String(s) => self.string(s),
    };
    let sign = if l.negative { text("-") } else { nil() };

    self.wrap(l.as_node(), concat([sign, lit]))
  }

  fn pattern_ctor(&self, c: &PCtor) -> Doc {
    let node = c.as_node();
    let doc = if c.args.is_empty() {
      concat([name(&c.name), self.empty_args(node)])
    } else {
      let args = c.args.iter().map(|a| self.pattern(a)).collect();

      concat([name(&c.name), self.delimited(node, "(", args, ")", false)])
    };

    self.wrap(node, doc)
  }

  fn pattern_record(&self, r: &PRecord) -> Doc {
    let node = r.as_node();

    if r.fields.is_empty() && !r.open {
      return self.wrap(node, self.empty(node, "{", "}"));
    }

    let fields: Vec<Doc> =
      r.fields.iter().map(|f| self.pattern_field(f)).collect();
    let sep = concat([text(","), line()]);
    let inner = match (r.open, fields.is_empty()) {
      (true, true) => self.lone_tail(node, "..".to_string()),
      (true, false) => {
        concat([join(fields, &sep), sep.clone(), tail_anchor(), text("..")])
      }
      (false, _) => concat([join(fields, &sep), if_break(text(","), nil())]),
    };

    let doc = group(concat([
      self.open(node, "{"),
      nest(INDENT, concat([line(), inner])),
      line(),
      closing("}"),
    ]));

    self.wrap(node, doc)
  }

  fn pattern_list(&self, l: &PList) -> Doc {
    let node = l.as_node();
    let items = l.items.iter().map(|i| self.pattern(i));
    let tail = l.tail.iter().map(|t| concat([text(".."), self.pattern(t)]));

    self.wrap(
      node,
      self.delimited(node, "[", items.chain(tail).collect(), "]", false),
    )
  }

  fn pattern_field(&self, f: &PField) -> Doc {
    self.wrap(
      f.as_node(),
      concat([name(&f.name), text(": "), self.pattern(&f.pattern)]),
    )
  }

  fn list(open: Doc, items: Vec<Doc>, close: &str, spaced: bool) -> Doc {
    if items.is_empty() {
      return concat([open, text(close)]);
    }

    let brk = if spaced { line() } else { softline() };

    concat([
      open,
      nest(
        INDENT,
        concat([
          brk.clone(),
          join(items, &concat([text(","), line()])),
          if_break(text(","), nil()),
        ]),
      ),
      brk,
      closing(close),
    ])
  }

  fn delimited(
    &self,
    node: NodeRef<'_>,
    open: &str,
    items: Vec<Doc>,
    close: &str,
    spaced: bool,
  ) -> Doc {
    if items.is_empty() {
      return self.empty(node, open, close);
    }

    group(Self::list(self.open(node, open), items, close, spaced))
  }
}

pub(super) fn opens_list(node: NodeRef<'_>) -> bool {
  match node {
    NodeRef::Call(c) => !c.args.is_empty(),
    NodeRef::FnDecl(f) => !f.params.is_empty(),
    NodeRef::Lambda(l) => !l.params.is_empty(),
    NodeRef::FnType(f) => !f.params.is_empty(),
    NodeRef::CtorDecl(c) => !c.args.is_empty(),
    NodeRef::PCtor(c) => !c.args.is_empty(),
    NodeRef::Match(m) => !m.arms.is_empty(),
    NodeRef::Try(t) => !t.arms.is_empty(),
    NodeRef::RecordLit(r) => r.spread.is_some() || !r.fields.is_empty(),
    NodeRef::ListLit(l) => !l.items.is_empty() || l.tail.is_some(),
    NodeRef::PList(l) => !l.items.is_empty() || l.tail.is_some(),
    NodeRef::PRecord(r) => !r.fields.is_empty() || r.open,
    NodeRef::RecordType(r) => !r.fields.is_empty() || r.tail.is_some(),
    NodeRef::EffectRow(e) => !e.entries.is_empty() || e.tail.is_some(),
    NodeRef::Block(_)
    | NodeRef::TraitDecl(_)
    | NodeRef::ImplDecl(_)
    | NodeRef::EffectDecl(_)
    | NodeRef::BindDecl(_)
    | NodeRef::Recipe(_) => true,
    _ => false,
  }
}

pub(super) fn prints_dangling(node: NodeRef<'_>) -> bool {
  match node {
    NodeRef::Module(_)
    | NodeRef::Zone(_)
    | NodeRef::TraitDecl(_)
    | NodeRef::ImplDecl(_)
    | NodeRef::EffectDecl(_)
    | NodeRef::BindDecl(_)
    | NodeRef::Recipe(_) => true,
    NodeRef::RecordLit(r) => r.spread.is_none() && r.fields.is_empty(),
    NodeRef::RecordType(r) => r.fields.is_empty(),
    NodeRef::EffectRow(e) => e.entries.is_empty(),
    NodeRef::PRecord(r) => r.fields.is_empty(),
    NodeRef::ListLit(l) => l.items.is_empty() && l.tail.is_none(),
    NodeRef::PList(l) => l.items.is_empty() && l.tail.is_none(),
    NodeRef::CtorDecl(c) => c.args.is_empty(),
    NodeRef::PCtor(c) => c.args.is_empty(),
    _ => false,
  }
}

pub(crate) fn name(n: &Name) -> Doc {
  text(n.text.as_str())
}

fn closing(bracket: &str) -> Doc {
  closer(INDENT, text(bracket))
}

fn parens(doc: Doc) -> Doc {
  concat([text("("), doc, closer(0, text(")"))])
}
