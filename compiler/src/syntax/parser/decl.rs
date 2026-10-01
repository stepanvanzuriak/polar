use super::Parser;
use crate::{
  shared::codes::DiagnosticCode::{
    BridgeWithBody, DeclWrongZone, ExpectedDeclaration, ExternTarget,
    MethodNeedsAnnotation, ModifierWrongZone, RedundantDeclarationKeyword,
    TraitParamCount, UnexpectedToken, UnknownRecipeCase, VisibilityKeyword,
    WrongIdentifierCase, ZoneDuplicate, ZoneOutOfOrder,
  },
  shared::diagnostic::{Diagnostic, Label},
  shared::source::Span,
  syntax::ast::{
    BindDecl, Block, Bound, Builtin, ConstDecl, CtorDecl, Decl, Derive,
    EffectDecl, ExportDecl, ExternDecl, FnDecl, HostDecl, ImplDecl, Import,
    MethodSig, Module, Name, Param, PluginEntry, PluginId, Recipe, StringLit,
    StringPart, TraitDecl, TypeBody, TypeDecl, VariantBody, Zone, ZoneKind,
  },
  syntax::lexer::token::{
    CommentKind, Token,
    TokenKind::{
      self, Arrow, Bar, Colon, Comma, Dot, Eof, Eq, Gt, InterpEnd, InterpStart,
      KwAs, KwBind, KwBinds, KwConstants, KwDerive, KwEffect, KwEffects,
      KwExport, KwExports, KwExtern, KwExterns, KwFor, KwForce, KwFunction,
      KwFunctions, KwHost, KwHosts, KwImpl, KwImpls, KwImport, KwIn, KwLet,
      KwModule, KwNative, KwPub, KwTrait, KwTraits, KwType, KwTypes, KwUses,
      KwWhere, LBrace, LParen, Lower, Lt, RBrace, RParen, StringStart, Upper,
    },
  },
  syntax::plugins,
};

impl Parser<'_> {
  pub(super) fn module(&mut self) -> Module {
    let file = self.peek(0).span.file.clone();
    let span = Span::new(file, 0, self.file.text().len());
    let name = self.module_header();
    let mut zones: Vec<Zone> = Vec::new();

    while !self.at(Eof) {
      let before = self.pos;

      if let Some(kind) = self.zone_keyword() {
        let zone = self.zone(kind);

        self.check_zone_order(&zones, &zone);
        zones.push(zone);
      } else {
        self.expected_declaration(None);
        self.skip_to_zone();
      }

      self.progress(before);
    }

    Module { span, name, zones }
  }

  fn module_header(&mut self) -> Option<Name> {
    self.eat(KwModule)?;

    Some(self.upper_name("as the module name"))
  }

  pub(crate) fn zone_keyword(&self) -> Option<ZoneKind> {
    self.zone_of(self.peek(0))
  }

  pub(crate) fn zone_of(&self, token: &Token) -> Option<ZoneKind> {
    if let Some(builtin) = zone_kind(token.kind) {
      return Some(ZoneKind::Builtin(builtin));
    }

    self.plugin_keyword(token).map(ZoneKind::Plugin)
  }

  fn plugin_keyword(&self, token: &Token) -> Option<PluginId> {
    if token.kind != Lower || self.enabled.is_empty() {
      return None;
    }

    let text = self.file.slice(&token.span);

    self
      .enabled
      .iter()
      .copied()
      .find(|id| plugins::keyword(*id) == text)
      .filter(|_| self.file.position_at(token.span.start).column == 0)
  }

  fn zone(&mut self, kind: ZoneKind) -> Zone {
    let start = self.bump().span.clone();
    let mut decls = Vec::new();

    match kind {
      ZoneKind::Builtin(builtin) => {
        while !self.at(Eof) && self.zone_keyword().is_none() {
          let before = self.pos;

          if let Some(decl) = self.decl(builtin) {
            decls.push(decl);
          }

          self.progress(before);
        }
      }
      ZoneKind::Plugin(_) => self.plugin_entries(&mut decls),
    }

    Zone { span: self.span_from(&start), kind, decls }
  }

  fn plugin_entries(&mut self, decls: &mut Vec<Decl>) {
    let mut indent = None;

    while !self.at(Eof) && self.zone_keyword().is_none() {
      let first = self.bump().clone();
      let indent = *indent.get_or_insert(self.column(&first.span));
      let mut text = self.file.slice(&first.span).to_string();
      let mut last = first.span.clone();

      while !self.at(Eof) && self.zone_keyword().is_none() {
        let token = self.peek(0).clone();

        if token.newline_before && self.column(&token.span) <= indent {
          break;
        }

        if token.newline_before {
          text.push('\n');
        } else if token.span.start > last.end {
          text.push(' ');
        }

        text.push_str(self.file.slice(&token.span));
        last = token.span.clone();
        self.bump();
      }

      let span = first.span.join(&last);

      decls.push(Decl::Plugin(PluginEntry { span, text }));
    }
  }

  fn decl(&mut self, zone: Builtin) -> Option<Decl> {
    let start = self.pos;
    let docs = self.docs_before(self.peek(0).span.start);

    self.skip_declaration_keywords();

    let Some(mut shape) = self.decl_shape(Some(zone)) else {
      self.not_a_declaration(zone);

      return None;
    };

    if self.modifier_in_wrong_zone(shape, zone) {
      shape = zone;
    }

    let (decl, span) = match shape {
      Builtin::Uses => {
        let import = self.import();
        let span = import.span.clone();

        (Decl::Import(import), span)
      }
      Builtin::Traits => {
        let trait_decl = self.trait_decl(docs);
        let span = trait_decl.span.clone();

        (Decl::Trait(trait_decl), span)
      }
      Builtin::Types => {
        let type_decl = self.type_decl(docs);
        let span = type_decl.span.clone();

        (Decl::Type(type_decl), span)
      }
      Builtin::Constants => {
        let const_decl = self.const_decl(docs);
        let span = const_decl.span.clone();

        (Decl::Const(const_decl), span)
      }
      Builtin::Functions => {
        let fn_decl = self.fn_decl(docs);
        let span = fn_decl.span.clone();

        (Decl::Fn(fn_decl), span)
      }
      Builtin::Impls => {
        let impl_decl = self.impl_decl(docs);
        let span = impl_decl.span.clone();

        (Decl::Impl(impl_decl), span)
      }
      Builtin::Exports => {
        let export = self.export();
        let span = export.span.clone();

        (Decl::Export(export), span)
      }
      Builtin::Hosts => {
        let host = self.host_decl(docs);
        let span = host.span.clone();

        (Decl::Host(host), span)
      }
      Builtin::Effects => {
        let effect = self.effect_decl(docs);
        let span = effect.span.clone();

        (Decl::Effect(effect), span)
      }
      Builtin::Externs => {
        let extern_decl = self.extern_decl(docs);
        let span = extern_decl.span.clone();

        (Decl::Extern(extern_decl), span)
      }
      Builtin::Binds => {
        let bind = self.bind_decl(docs);
        let span = bind.span.clone();

        (Decl::Bind(bind), span)
      }
    };

    if std::mem::take(&mut self.body_unclosed) {
      return None;
    }

    if self.recovering {
      let depth = brace_depth(&self.tokens[start..self.pos]);

      self.synchronise(depth);

      return None;
    }

    if shape != zone {
      self.wrong_zone(span, shape, zone);
    }

    Some(decl)
  }

  fn decl_shape(&self, zone: Option<Builtin>) -> Option<Builtin> {
    match (self.peek(0).kind, self.peek(1).kind) {
      (Upper, _) if zone == Some(Builtin::Hosts) => Some(Builtin::Hosts),
      (Upper, KwIn) if zone == Some(Builtin::Binds) => Some(Builtin::Binds),
      (Upper, KwIn) if self.at_from(3) => Some(Builtin::Binds),
      (KwForce, Upper) => Some(Builtin::Binds),
      (KwNative, Upper) | (Upper, KwIn) => Some(Builtin::Effects),
      (Upper, LBrace) if zone == Some(Builtin::Effects) => {
        Some(Builtin::Effects)
      }
      (Lower, LParen) if zone == Some(Builtin::Externs) => {
        Some(Builtin::Externs)
      }
      (Lower, Colon) => Some(Builtin::Constants),
      (Lower, Eq) if zone == Some(Builtin::Constants) => {
        Some(Builtin::Constants)
      }
      (Upper, Lt | LBrace) if zone == Some(Builtin::Traits) => {
        Some(Builtin::Traits)
      }
      (Upper, KwFor) => Some(Builtin::Impls),
      (Upper | Lower, Eq | Lt) => Some(Builtin::Types),
      (Upper, _) if zone == Some(Builtin::Types) => Some(Builtin::Types),
      (Upper, Dot | KwAs) => Some(Builtin::Uses),
      (Upper, _) if zone == Some(Builtin::Uses) => Some(Builtin::Uses),
      (Upper, _) if zone == Some(Builtin::Exports) => Some(Builtin::Exports),
      (Lower | Upper, LParen) => Some(Builtin::Functions),
      (Lower, _) if zone == Some(Builtin::Exports) => Some(Builtin::Exports),
      _ => None,
    }
  }

  fn skip_declaration_keywords(&mut self) {
    loop {
      let token = self.peek(0);
      let keyword = token.kind.display_name();
      let span = token.span.clone();

      let diagnostic = match token.kind {
        KwFunction | KwType | KwImport | KwTrait | KwImpl | KwHost
        | KwEffect | KwBind | KwExtern => Diagnostic::error(
          RedundantDeclarationKeyword,
          format!("{keyword} is not needed: the zone supplies the keyword"),
          Label::new(span),
        )
        .with_help("remove it"),
        KwPub | KwExport => Diagnostic::error(
          VisibilityKeyword,
          "visibility is declared in the `exports` zone",
          Label::new(span),
        )
        .with_help("remove it and list the name under `exports`"),
        _ => return,
      };

      self.diagnostics.push(diagnostic);
      self.bump();
    }
  }

  fn decl_name(&mut self, kind: TokenKind, what: &str) -> Name {
    let (other, letter) = if kind == Upper {
      (Lower, "an uppercase")
    } else {
      (Upper, "a lowercase")
    };

    if !self.at(other) {
      return self.name(kind, &format!("as the {what} name"));
    }

    let span = self.bump().span.clone();
    let text = self.file.slice(&span).to_string();
    let fixed = recase(&text, kind);

    self.diagnostics.push(
      Diagnostic::error(
        WrongIdentifierCase,
        format!("{what} name `{text}` should start with {letter} letter"),
        Label::new(span.clone()),
      )
      .with_help(format!("write it as `{fixed}`")),
    );

    Name { text, span }
  }

  fn type_decl(&mut self, docs: Vec<Span>) -> TypeDecl {
    let name = self.decl_name(Upper, "type");
    let params = self.type_params();
    let body = if self.at_opaque_end() {
      let end = self.span_from(&name.span).end;

      TypeBody::Variants(VariantBody {
        span: Span::empty(name.span.file.clone(), end),
        ctors: Vec::new(),
      })
    } else {
      self.expect(Eq, "after the type name");
      self.type_body()
    };
    let derive = self.derive();

    TypeDecl {
      span: self.span_from(&name.span),
      docs,
      name,
      params,
      body,
      derive,
    }
  }

  fn at_opaque_end(&self) -> bool {
    let token = self.peek(0);

    token.kind == Eof || (token.newline_before && token.kind != Eq)
  }

  fn type_params(&mut self) -> Vec<Name> {
    let mut params = Vec::new();

    if self.eat(Lt).is_none() {
      return params;
    }

    while !self.at(Gt) && !self.at(Eof) {
      let before = self.pos;

      params.push(self.lower_name("as a type parameter"));

      if self.eat(Comma).is_none() {
        break;
      }

      self.progress(before);
    }

    self.expect(Gt, "to close the type parameters");

    params
  }

  fn type_body(&mut self) -> TypeBody {
    if self.is_variant_body() {
      TypeBody::Variants(self.variants())
    } else {
      TypeBody::Alias(self.type_expr())
    }
  }

  fn is_variant_body(&self) -> bool {
    if self.at(Bar) || (self.at(Upper) && self.peek(1).kind == LParen) {
      return true;
    }

    let mut depth = 0usize;

    for (i, token) in self.tokens[self.pos..].iter().enumerate() {
      let next_entry =
        i > 0 && token.newline_before && starts_entry(token.kind);

      if depth == 0
        && (token.kind == Eof
          || token.kind == KwDerive
          || self.zone_of(token).is_some()
          || next_entry)
      {
        return false;
      }

      match token.kind {
        LParen | LBrace | Lt => depth += 1,
        RParen | RBrace | Gt => depth = depth.saturating_sub(1),
        Bar if depth == 0 => return true,
        _ => {}
      }
    }

    false
  }

  fn variants(&mut self) -> VariantBody {
    let start = self.peek(0).span.clone();
    let mut ctors = Vec::new();

    self.eat(Bar);

    loop {
      let before = self.pos;

      ctors.push(self.ctor());

      if self.eat(Bar).is_none() {
        break;
      }

      self.progress(before);
    }

    VariantBody { span: self.span_from(&start), ctors }
  }

  fn ctor(&mut self) -> CtorDecl {
    let name = self.decl_name(Upper, "constructor");
    let mut args = Vec::new();

    if self.eat(LParen).is_some() {
      while !self.at(RParen) && !self.at(Eof) {
        let before = self.pos;

        args.push(self.type_expr());

        if self.eat(Comma).is_none() {
          break;
        }

        self.progress(before);
      }

      self.expect(RParen, "to close the constructor's arguments");
    }

    CtorDecl { span: self.span_from(&name.span), name, args }
  }

  fn derive(&mut self) -> Option<Derive> {
    let start = self.eat(KwDerive)?.span.clone();
    let mut names = Vec::new();

    self.expect(LParen, "after `derive`");

    while !self.at(RParen) && !self.at(Eof) {
      let before = self.pos;

      names.push(self.upper_name("as a derive name"));

      if self.eat(Comma).is_none() {
        break;
      }

      self.progress(before);
    }

    self.expect(RParen, "to close the derive list");

    Some(Derive { span: self.span_from(&start), names })
  }

  fn import(&mut self) -> Import {
    let start = self.peek(0).span.clone();
    let mut path = vec![self.upper_name("to start an import path")];

    while self.eat(Dot).is_some() {
      path.push(self.upper_name("after `.` in an import path"));
    }

    let alias = if self.eat(KwAs).is_some() {
      Some(self.upper_name("after `as`"))
    } else {
      None
    };
    let methods = self.method_list();

    Import { span: self.span_from(&start), path, alias, methods }
  }

  fn method_list(&mut self) -> Option<Vec<Name>> {
    if !self.at(LBrace) || self.peek(0).newline_before {
      return None;
    }

    self.bump();

    let mut names = Vec::new();

    while !self.at(RBrace) && !self.at(Eof) {
      let before = self.pos;

      names.push(self.lower_name("as a method name"));

      if self.eat(Comma).is_none() {
        break;
      }

      self.progress(before);
    }

    self.expect(RBrace, "to close the method list");

    Some(names)
  }

  fn trait_decl(&mut self, docs: Vec<Span>) -> TraitDecl {
    let name = self.decl_name(Upper, "trait");
    let param = self.trait_param(&name);
    let methods = self.method_sigs("trait");
    let recipe = self.recipe();

    TraitDecl {
      span: self.span_from(&name.span),
      docs,
      name,
      param,
      methods,
      recipe,
    }
  }

  fn trait_param(&mut self, name: &Name) -> Name {
    let Some(open) = self.eat(Lt).map(|t| t.span.clone()) else {
      self.trait_param_count(name.span.clone(), &name.text);

      return Name { text: "a".to_string(), span: self.missing_span() };
    };
    let mut params = Vec::new();

    while !self.at(Gt) && !self.at(Eof) {
      let before = self.pos;

      params.push(self.lower_name("as the trait's type parameter"));

      if self.eat(Comma).is_none() {
        break;
      }

      self.progress(before);
    }

    self.expect(Gt, "to close the trait's type parameter");

    if params.len() != 1 {
      let span = self.span_from(&open);

      self.trait_param_count(span, &name.text);
    }

    params.into_iter().next().unwrap_or_else(|| Name {
      text: "a".to_string(),
      span: self.missing_span(),
    })
  }

  fn trait_param_count(&mut self, span: Span, name: &str) {
    self.diagnostics.push(
      Diagnostic::error(
        TraitParamCount,
        "a trait takes exactly one type parameter",
        Label::new(span),
      )
      .with_help(format!("write it as `{name}<a>`")),
    );
  }

  fn method_sigs(&mut self, what: &str) -> Vec<MethodSig> {
    let mut methods = Vec::new();

    self.expect(LBrace, &format!("to open the {what}"));

    while !self.at(RBrace) && !self.at(Eof) && self.zone_keyword().is_none() {
      let before = self.pos;
      let docs = self.docs_before(self.peek(0).span.start);

      methods.push(self.method_sig(docs, what));

      if self.pos == before {
        self.expected("a method signature");
        self.bump();
      }
    }

    self.expect(RBrace, &format!("to close the {what}"));

    methods
  }

  fn method_sig(&mut self, docs: Vec<Span>, what: &str) -> MethodSig {
    let (noun, owner) = if what == "effect" {
      ("operation", "an effect operation")
    } else {
      ("method", "a trait method")
    };
    let name = self.decl_name(Lower, noun);
    let params = self.params();

    self.params_need_types(&params, owner);

    let (return_type, effects) = if self.eat(Arrow).is_some() {
      (self.type_expr(), self.effect_row())
    } else {
      let owner = owner.split_once(' ').map_or(owner, |(_, rest)| rest);

      self.diagnostics.push(
        Diagnostic::error(
          MethodNeedsAnnotation,
          format!("the {owner} `{}` needs a return type", name.text),
          Label::new(name.span.clone()),
        )
        .with_help("add `-> Type` after the parameters"),
      );

      (self.invalid_type(), None)
    };

    MethodSig {
      span: self.span_from(&name.span),
      docs,
      name,
      params,
      return_type,
      effects,
    }
  }

  fn params_need_types(&mut self, params: &[Param], owner: &str) {
    for param in params {
      if let Some(pattern) = &param.pattern {
        self.diagnostics.push(
          Diagnostic::error(
            UnexpectedToken,
            format!("a parameter of {owner} must be a name, not a pattern"),
            Label::new(pattern.span().clone()),
          )
          .with_help("only functions with a body can destructure parameters"),
        );
      } else if param.ty.is_none() {
        self.diagnostics.push(
          Diagnostic::error(
            MethodNeedsAnnotation,
            format!(
              "the parameter `{}` of {owner} needs a type",
              param.name.text
            ),
            Label::new(param.name.span.clone()),
          )
          .with_help(format!("write it as `{}: a`", param.name.text)),
        );
      }
    }
  }

  fn host_decl(&mut self, docs: Vec<Span>) -> HostDecl {
    let name = self.decl_name(Upper, "host");

    HostDecl { span: name.span.clone(), docs, name }
  }

  fn effect_decl(&mut self, docs: Vec<Span>) -> EffectDecl {
    let start = self.peek(0).span.clone();
    let native = self.eat(KwNative).is_some();

    self.eat(KwForce);

    let name = self.decl_name(Upper, "effect");
    let host = if self.eat(KwIn).is_some() {
      Some(self.upper_name("as the host"))
    } else {
      None
    };
    let ops = self.method_sigs("effect");

    EffectDecl { span: self.span_from(&start), docs, native, name, host, ops }
  }

  fn bind_decl(&mut self, docs: Vec<Span>) -> BindDecl {
    let start = self.peek(0).span.clone();
    let force = self.eat(KwForce).is_some();

    self.eat(KwNative);

    let effect = self.decl_name(Upper, "effect");

    self.expect(KwIn, "after the effect name");

    let host = self.upper_name("as the host");
    let mut ops = Vec::new();
    let mut module = None;
    let from = if self.at_from(0) {
      self.bump();
      Some(self.upper_name("as the host the bridge forwards to"))
    } else {
      None
    };

    if let Some(target) = &from {
      if self.at(Eq) || self.at(LBrace) {
        self.bridge_with_body(&effect, &host, target);
      }
    } else if self.eat(Eq).is_some() {
      module = self.bind_module(&effect, &host);
    } else {
      self.expect(LBrace, "to open the binding");
      self.members(&mut ops);
      self.expect(RBrace, "to close the binding");
    }

    BindDecl {
      span: self.span_from(&start),
      docs,
      force,
      effect,
      host,
      from,
      module,
      ops,
    }
  }

  fn bridge_with_body(&mut self, effect: &Name, host: &Name, target: &Name) {
    let span = self.peek(0).span.clone();

    self.diagnostics.push(
      Diagnostic::error(
        BridgeWithBody,
        format!(
          "the bridge of `{}` in `{}` forwards to `{}`, so it can't have a body",
          effect.text, host.text, target.text
        ),
        Label::new(span),
      )
      .with_help(format!(
        "remove the body, or remove `from {}` to bind `{}` in `{}` directly",
        target.text, effect.text, host.text
      )),
    );
    self.recovering = true;

    if self.eat(Eq).is_some() {
      if self.at(StringStart) {
        self.string_lit();
      }
    } else if self.eat(LBrace).is_some() {
      let mut skipped = Vec::new();

      self.members(&mut skipped);
      self.expect(RBrace, "to close the binding");
    }
  }

  fn extern_decl(&mut self, docs: Vec<Span>) -> ExternDecl {
    let name = self.decl_name(Lower, "extern");
    let params = self.params();

    self.params_need_types(&params, "an extern");

    let (return_type, effects) = if self.eat(Arrow).is_some() {
      (self.type_expr(), self.effect_row())
    } else {
      self.diagnostics.push(
        Diagnostic::error(
          MethodNeedsAnnotation,
          format!("the extern `{}` needs a return type", name.text),
          Label::new(name.span.clone()),
        )
        .with_help("add `-> Type` after the parameters"),
      );

      (self.invalid_type(), None)
    };

    let (module, export) = self.extern_target(&name);

    ExternDecl {
      span: self.span_from(&name.span),
      docs,
      name,
      params,
      return_type,
      effects,
      module,
      export,
    }
  }

  fn extern_target(&mut self, name: &Name) -> (StringLit, Name) {
    if self.eat(Eq).is_none() || !self.at(StringStart) {
      let span = self.error_span();

      self.diagnostics.push(
        Diagnostic::error(
          ExternTarget,
          format!("the extern `{}` needs a JavaScript target", name.text),
          Label::new(span),
        )
        .with_help("write `= \"./file.js\" export_name`"),
      );

      let missing = self.missing_span();

      return (
        StringLit { span: missing.clone(), parts: vec![] },
        Name { text: String::new(), span: missing },
      );
    }

    let module = self.string_lit();

    if let Some(interp) = module.parts.iter().find_map(|part| match part {
      StringPart::Interp(interp) => Some(interp),
      StringPart::Text(_) => None,
    }) {
      self.diagnostics.push(
        Diagnostic::error(
          ExternTarget,
          "the module of an extern can't be interpolated",
          Label::new(interp.span.clone()),
        )
        .with_help("write the module path out in full"),
      );
    }

    let export = if self.at(Upper) {
      self.upper_name("as the export name")
    } else {
      self.lower_name("as the export name")
    };

    (module, export)
  }

  fn bind_module(&mut self, effect: &Name, host: &Name) -> Option<StringLit> {
    if !self.at(StringStart) {
      let span = self.error_span();

      self.diagnostics.push(
        Diagnostic::error(
          ExternTarget,
          format!(
            "the binding of `{}` in `{}` needs a JavaScript module",
            effect.text, host.text
          ),
          Label::new(span),
        )
        .with_help(
          "write `= \"./file.js\"`, or give the binding a `{ … }` body",
        ),
      );
      self.recovering = true;

      return None;
    }

    let module = self.string_lit();

    if let Some(interp) = module.parts.iter().find_map(|part| match part {
      StringPart::Interp(interp) => Some(interp),
      StringPart::Text(_) => None,
    }) {
      self.diagnostics.push(
        Diagnostic::error(
          ExternTarget,
          "the module of a binding can't be interpolated",
          Label::new(interp.span.clone()),
        )
        .with_help("write the module path out in full"),
      );
    }

    Some(module)
  }

  fn modifier_in_wrong_zone(&mut self, shape: Builtin, zone: Builtin) -> bool {
    let token = self.peek(0);
    let (modifier, noun) = match (token.kind, shape, zone) {
      (KwForce, Builtin::Binds, Builtin::Effects) => ("force", "an effect"),
      (KwNative, Builtin::Effects, Builtin::Binds) => ("native", "a binding"),
      _ => return false,
    };
    let goes = if modifier == "force" { "a binding" } else { "an effect" };

    self.diagnostics.push(
      Diagnostic::error(
        ModifierWrongZone,
        format!("`{modifier}` goes on {goes}, not {noun}"),
        Label::new(token.span.clone()),
      )
      .with_help(
        "`native` marks an effect that can't move; `force` binds one anyway",
      ),
    );

    true
  }

  fn recipe(&mut self) -> Option<Recipe> {
    let start = self.eat(KwDerive)?.span.clone();
    let mut cases: Vec<FnDecl> = Vec::new();

    self.expect(LBrace, "to open the derive recipe");
    self.members(&mut cases);
    self.expect(RBrace, "to close the derive recipe");

    for (i, case) in cases.iter().enumerate() {
      let known = matches!(case.name.text.as_str(), "record" | "variant");
      let repeated = cases[..i].iter().any(|c| c.name.text == case.name.text);

      if case.name.text.is_empty() || (known && !repeated) {
        continue;
      }

      let message = if known {
        format!("the recipe has more than one `{}` case", case.name.text)
      } else {
        format!("`{}` is not a recipe case", case.name.text)
      };

      self.diagnostics.push(
        Diagnostic::error(
          UnknownRecipeCase,
          message,
          Label::new(case.name.span.clone()),
        )
        .with_help("a recipe has a `record` case and a `variant` case"),
      );
    }

    Some(Recipe { span: self.span_from(&start), cases })
  }

  fn impl_decl(&mut self, docs: Vec<Span>) -> ImplDecl {
    let trait_name = self.decl_name(Upper, "trait");

    self.expect(KwFor, "after the trait name");

    let target = self.type_ref();
    let bounds = self.bounds();
    let mut methods = Vec::new();

    self.expect(LBrace, "to open the impl");
    self.members(&mut methods);
    self.expect(RBrace, "to close the impl");

    ImplDecl {
      span: self.span_from(&trait_name.span),
      docs,
      trait_name,
      target,
      bounds,
      methods,
    }
  }

  fn members(&mut self, out: &mut Vec<FnDecl>) {
    while !self.at(RBrace) && !self.at(Eof) && self.zone_keyword().is_none() {
      let before = self.pos;
      let docs = self.docs_before(self.peek(0).span.start);

      out.push(self.fn_decl(docs));

      if self.body_unclosed {
        return;
      }

      if self.pos == before {
        self.bump();
      }
    }
  }

  fn bounds(&mut self) -> Vec<Bound> {
    let mut bounds = Vec::new();

    let Some(mut start) = self.eat(KwWhere).map(|t| t.span.clone()) else {
      return bounds;
    };

    loop {
      if !self.at(Upper) {
        self.expected("a trait name");
        break;
      }

      let before = self.pos;
      let trait_name = self.upper_name("as a trait name");

      self.expect(Lt, "after the trait name");

      let var = self.lower_name("as a type variable");

      self.expect(Gt, "to close the bound");

      bounds.push(Bound { span: self.span_from(&start), trait_name, var });

      if self.eat(Comma).is_none() {
        break;
      }

      start = self.peek(0).span.clone();

      self.progress(before);
    }

    bounds
  }

  fn const_decl(&mut self, docs: Vec<Span>) -> ConstDecl {
    let name = self.decl_name(Lower, "constant");
    let ty =
      if self.eat(Colon).is_some() { Some(self.type_expr()) } else { None };

    self.expect(Eq, "after the constant name");

    let value = self.expr();

    ConstDecl { span: self.span_from(&name.span), docs, name, ty, value }
  }

  fn fn_decl(&mut self, docs: Vec<Span>) -> FnDecl {
    let name = self.decl_name(Lower, "function");
    let params = self.params();

    let (return_type, effects) = if self.eat(Arrow).is_some() {
      (Some(self.type_expr()), self.effect_row())
    } else {
      (None, None)
    };
    let bounds = self.bounds();
    let body = self.fn_body(&name.span);

    FnDecl {
      span: self.span_from(&name.span),
      docs,
      name,
      params,
      return_type,
      effects,
      bounds,
      body,
    }
  }

  pub(super) fn params(&mut self) -> Vec<Param> {
    let mut params = Vec::new();

    self.expect(LParen, "after the function name");

    while !self.at(RParen) && !self.at(Eof) {
      let before = self.pos;
      let param = if self.at_param_pattern() {
        self.pattern_param(params.len())
      } else {
        self.param()
      };

      params.push(param);

      if self.eat(Comma).is_none() {
        break;
      }

      self.progress(before);
    }

    self.expect(RParen, "to close the parameter list");

    params
  }

  fn at_param_pattern(&self) -> bool {
    matches!(
      self.peek(0).kind,
      LBrace | TokenKind::LBracket | Upper | TokenKind::Underscore
    )
  }

  fn pattern_param(&mut self, index: usize) -> Param {
    let pattern = self.pattern();
    let span = pattern.span().clone();
    let name = Name { text: format!("$arg{index}"), span: span.clone() };
    let ty =
      if self.eat(Colon).is_some() { Some(self.type_expr()) } else { None };

    Param { span: self.span_from(&span), name, pattern: Some(pattern), ty }
  }

  fn param(&mut self) -> Param {
    let name = self.lower_name("as a parameter name");
    let ty =
      if self.eat(Colon).is_some() { Some(self.type_expr()) } else { None };

    Param { span: self.span_from(&name.span), name, pattern: None, ty }
  }

  fn fn_body(&mut self, name: &Span) -> Block {
    if !self.at(LBrace) || self.body_is_closed() {
      return self.block("to open the function body");
    }

    let open = self.peek(0).span.clone();
    let limit = self.unclosed_body_end(name);

    self.diagnostics.push(
      Diagnostic::error(
        UnexpectedToken,
        "this function body is never closed",
        Label::new(open),
      )
      .with_help("add the missing `}`"),
    );

    self.limit = Some(limit);

    let body = self.block("to open the function body");

    self.limit = None;
    self.body_unclosed = true;

    body
  }

  fn body_is_closed(&self) -> bool {
    let mut depth = 0usize;

    for token in &self.tokens[self.pos..] {
      match token.kind {
        LBrace | InterpStart => depth += 1,
        RBrace | InterpEnd => depth -= 1,
        Eof => return false,
        _ if self.zone_of(token).is_some() => return false,
        _ => {}
      }

      if depth == 0 {
        return true;
      }
    }

    false
  }

  fn unclosed_body_end(&self, name: &Span) -> usize {
    let column = |span: &Span| self.file.position_at(span.start).column;
    let indent = column(name);

    (self.pos + 1..self.tokens.len())
      .find(|&i| {
        let token = &self.tokens[i];

        token.kind == Eof
          || (token.newline_before
            && (self.zone_of(token).is_some() || column(&token.span) <= indent))
      })
      .unwrap_or(self.tokens.len() - 1)
  }

  fn export(&mut self) -> ExportDecl {
    let name = if self.at(Upper) {
      self.upper_name("as an exported trait")
    } else {
      self.lower_name("as an exported name")
    };
    let methods = self.method_list();

    ExportDecl { span: self.span_from(&name.span), name, methods }
  }

  fn docs_before(&self, start: usize) -> Vec<Span> {
    let text = self.file.text();
    let end = self.comments.partition_point(|c| c.span.start < start);
    let mut first = end;
    let mut gap_end = start;

    while let Some(comment) = first.checked_sub(1).map(|i| &self.comments[i]) {
      if comment.kind != CommentKind::Doc
        || !text[comment.span.end..gap_end].trim().is_empty()
      {
        break;
      }

      gap_end = comment.span.start;
      first -= 1;
    }

    self.comments[first..end].iter().map(|c| c.span.clone()).collect()
  }

  fn synchronise(&mut self, mut depth: usize) {
    while !self.at(Eof) {
      let token = self.peek(0);
      let entry = token.newline_before && starts_entry(token.kind);

      if depth == 0 && (self.zone_keyword().is_some() || entry) {
        break;
      }

      match self.bump().kind {
        LBrace | InterpStart => depth += 1,
        RBrace | InterpEnd => depth = depth.saturating_sub(1),
        _ => {}
      }
    }
  }

  fn check_zone_order(&mut self, zones: &[Zone], zone: &Zone) {
    let previous = zones.last().map(|last| last.kind);
    let is_zone_duplicate = zones.iter().any(|seen| seen.kind == zone.kind);
    let message = if is_zone_duplicate {
      format!("the `{}` zone appears more than once", zone.kind.as_str())
    } else {
      match previous {
        Some(previous) if zone.kind < previous => format!(
          "the `{}` zone must come before `{}`",
          zone.kind.as_str(),
          previous.as_str()
        ),
        _ => return,
      }
    };

    let error_code =
      if is_zone_duplicate { ZoneDuplicate } else { ZoneOutOfOrder };
    let order: Vec<String> = self
      .zone_order()
      .into_iter()
      .map(|kind| format!("`{}`", kind.as_str()))
      .collect();

    self.diagnostics.push(
      Diagnostic::error(error_code, message, Label::new(zone.span.clone()))
        .with_help(format!(
          "zones run {}, each at most once",
          order.join(" -> ")
        )),
    );
  }

  fn zone_order(&self) -> Vec<ZoneKind> {
    let mut order: Vec<ZoneKind> = Builtin::ALL
      .into_iter()
      .map(ZoneKind::Builtin)
      .chain(self.enabled.iter().copied().map(ZoneKind::Plugin))
      .collect();

    order.sort();
    order
  }

  fn wrong_zone(&mut self, span: Span, belongs: Builtin, written: Builtin) {
    self.diagnostics.push(
      Diagnostic::error(
        DeclWrongZone,
        format!(
          "{} belongs in the `{}` zone, not `{}`",
          decl_noun(belongs),
          belongs.as_str(),
          written.as_str()
        ),
        Label::new(span),
      )
      .with_help(format!("move it under `{}`", belongs.as_str())),
    );
  }

  fn disabled_plugin(&self) -> Option<String> {
    let token = self.peek(0);
    let alone = self.peek(1).newline_before || self.peek(1).kind == Eof;

    if token.kind != Lower || !alone || self.column(&token.span.clone()) != 0 {
      return None;
    }

    Some(self.file.slice(&token.span).to_string())
  }

  fn not_a_declaration(&mut self, zone: Builtin) {
    self.expected_declaration(Some(zone));

    if self.at(Eof) || self.zone_keyword().is_some() {
      return;
    }

    if self.disabled_plugin().is_some() {
      self.skip_to_zone();
      return;
    }

    let depth = usize::from(self.bump().kind == LBrace);

    self.synchronise(depth);
  }

  fn expected_declaration(&mut self, zone: Option<Builtin>) {
    let found = self.peek(0).kind;
    let span = self.error_span();
    let disabled = self.disabled_plugin();

    let help = if let Some(keyword) = disabled {
      format!(
        "if `{keyword}` is a plugin zone, add the package that defines it to \
         [dependencies] in polar.toml"
      )
    } else if found == KwLet {
      "top-level values go in a `constants` zone, like `pi = 3.14`".to_string()
    } else if let Some(zone) = zone {
      format!("the `{}` zone holds {}", zone.as_str(), zone_contents(zone))
    } else if let Some(shape) = self.decl_shape(None) {
      format!("{} goes under a `{}` zone", decl_noun(shape), shape.as_str())
    } else {
      let names: Vec<String> = self
        .zone_order()
        .into_iter()
        .map(|kind| format!("`{}`", kind.as_str()))
        .collect();
      let (last, rest) = names
        .split_last()
        .map_or((String::new(), &[][..]), |(last, rest)| (last.clone(), rest));

      format!("declarations live in a zone: {} or {last}", rest.join(", "))
    };

    self.diagnostics.push(
      Diagnostic::error(
        ExpectedDeclaration,
        format!("{} cannot start a declaration here", found.display_name()),
        Label::new(span),
      )
      .with_help(help),
    );
  }

  fn skip_to_zone(&mut self) {
    while !self.at(Eof) && self.zone_keyword().is_none() {
      self.bump();
    }
  }
}

fn zone_kind(kind: TokenKind) -> Option<Builtin> {
  match kind {
    KwUses => Some(Builtin::Uses),
    KwHosts => Some(Builtin::Hosts),
    KwTraits => Some(Builtin::Traits),
    KwTypes => Some(Builtin::Types),
    KwConstants => Some(Builtin::Constants),
    KwEffects => Some(Builtin::Effects),
    KwExterns => Some(Builtin::Externs),
    KwBinds => Some(Builtin::Binds),
    KwFunctions => Some(Builtin::Functions),
    KwImpls => Some(Builtin::Impls),
    KwExports => Some(Builtin::Exports),
    _ => None,
  }
}

fn starts_entry(kind: TokenKind) -> bool {
  matches!(
    kind,
    Lower
      | Upper
      | KwFunction
      | KwType
      | KwImport
      | KwTrait
      | KwImpl
      | KwPub
      | KwExport
      | KwLet
      | KwHost
      | KwEffect
      | KwBind
      | KwExtern
      | KwNative
      | KwForce
  )
}

fn brace_depth(tokens: &[Token]) -> usize {
  tokens.iter().fold(0, |depth, token| match token.kind {
    LBrace | InterpStart => depth + 1,
    RBrace | InterpEnd => depth.saturating_sub(1),
    _ => depth,
  })
}

fn decl_noun(kind: Builtin) -> &'static str {
  match kind {
    Builtin::Uses => "an import",
    Builtin::Hosts => "a host",
    Builtin::Effects => "an effect",
    Builtin::Externs => "an extern",
    Builtin::Binds => "a binding",
    Builtin::Traits => "a trait declaration",
    Builtin::Types => "a type declaration",
    Builtin::Constants => "a constant",
    Builtin::Functions => "a function declaration",
    Builtin::Impls => "an impl",
    Builtin::Exports => "an export",
  }
}

fn zone_contents(kind: Builtin) -> &'static str {
  match kind {
    Builtin::Uses => "imports, like `Foo.Bar`",
    Builtin::Hosts => "hosts, like `DOM`",
    Builtin::Effects => "effects, like `Storage in DOM { … }`",
    Builtin::Externs => "externs, like `now() -> Int = \"./time.js\" now`",
    Builtin::Binds => "bindings, like `Storage in Node { … }`",
    Builtin::Traits => "traits, like `Show<a> { show(value: a) -> String }`",
    Builtin::Types => "type declarations, like `Id = Int`",
    Builtin::Constants => "constants, like `pi = 3.14`",
    Builtin::Functions => "function declarations, like `main() { 0 }`",
    Builtin::Impls => "impls, like `Show for Status { … }`",
    Builtin::Exports => "exported names, like `main`",
  }
}

fn recase(text: &str, kind: TokenKind) -> String {
  let mut chars = text.chars();

  match chars.next() {
    Some(first) if kind == Upper => first.to_uppercase().chain(chars).collect(),
    Some(first) => first.to_lowercase().chain(chars).collect(),
    None => String::new(),
  }
}
