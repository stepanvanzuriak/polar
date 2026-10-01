use polar_compiler::{
  shared::codes::DiagnosticCode::{
    self, DeclWrongZone, ExpectedDeclaration, RedundantDeclarationKeyword,
    UnexpectedToken, VisibilityKeyword, WrongIdentifierCase, ZoneDuplicate,
    ZoneOutOfOrder,
  },
  shared::diagnostic::{Diagnostic, DiagnosticBag},
  shared::source::SourceFile,
  syntax::ast::{
    Decl, FnDecl, Module, TypeBody, TypeDecl, TypeExpr, ZoneKind,
    dump::dump_ast_shape,
  },
  syntax::lexer::lex,
  syntax::parser::parse,
};

mod common;

use common::{invariants::check_invariants, sexp::sexp};

fn parse_module(src: &str) -> (Module, Vec<Diagnostic>) {
  let file = SourceFile::new("test.px", src);
  let mut bag = DiagnosticBag::default();
  let lexed = lex(&file, &mut bag);
  let module = parse(&file, &lexed, &mut bag);

  check_invariants(src, &module);

  (module, bag.into_sorted())
}

fn dump_module(src: &str) -> (String, Vec<Diagnostic>) {
  let (module, diagnostics) = parse_module(src);

  (dump_ast_shape(&module), diagnostics)
}

fn type_source(entry: &str) -> String {
  format!("types\n  {entry}\n")
}

fn fn_source(entry: &str) -> String {
  format!("functions\n  {entry}\n")
}

fn codes(diagnostics: &[Diagnostic]) -> Vec<DiagnosticCode> {
  diagnostics.iter().map(|d| d.code).collect()
}

fn zone_kinds(module: &Module) -> Vec<ZoneKind> {
  module.zones.iter().map(|z| z.kind).collect()
}

fn find_code(diagnostics: &[Diagnostic], code: DiagnosticCode) -> &Diagnostic {
  diagnostics.iter().find(|d| d.code == code).unwrap_or_else(|| {
    panic!("expected a {code} diagnostic, got {diagnostics:?}")
  })
}

fn fn_decl(decl: &Decl) -> &FnDecl {
  match decl {
    Decl::Fn(fn_decl) => fn_decl,
    other => panic!("expected a `FnDecl`, got {other:?}"),
  }
}

fn type_decl(decl: &Decl) -> &TypeDecl {
  match decl {
    Decl::Type(type_decl) => type_decl,
    other => panic!("expected a `TypeDecl`, got {other:?}"),
  }
}

fn decl_names(module: &Module) -> Vec<&str> {
  module
    .zones
    .iter()
    .flat_map(|zone| &zone.decls)
    .map(|decl| match decl {
      Decl::Import(import) => import.path[0].text.as_str(),
      Decl::Trait(trait_decl) => trait_decl.name.text.as_str(),
      Decl::Impl(impl_decl) => impl_decl.trait_name.text.as_str(),
      Decl::Type(type_decl) => type_decl.name.text.as_str(),
      Decl::Const(const_decl) => const_decl.name.text.as_str(),
      Decl::Fn(fn_decl) => fn_decl.name.text.as_str(),
      Decl::Export(export) => export.name.text.as_str(),
      Decl::Host(host) => host.name.text.as_str(),
      Decl::Effect(effect) => effect.name.text.as_str(),
      Decl::Extern(ext) => ext.name.text.as_str(),
      Decl::Bind(bind) => bind.effect.text.as_str(),
      Decl::Plugin(_) => "",
    })
    .collect()
}

fn single_decl(module: &Module, kind: ZoneKind) -> &Decl {
  assert_eq!(zone_kinds(module), [kind]);

  match &module.zones[0].decls[..] {
    [decl] => decl,
    other => panic!("expected exactly one declaration, got {other:?}"),
  }
}

mod zones {

  use super::*;

  #[test]
  fn import_path() {
    let (dump, diagnostics) = dump_module("uses\n  Foo.Bar\n");

    assert_eq!(dump, sexp!(Module (Zone uses (Import Foo Bar))));

    assert!(diagnostics.is_empty(), "unexpected diagnostics: {diagnostics:?}");
  }

  #[test]
  fn import_alias() {
    let (dump, diagnostics) = dump_module("uses\n  Foo.Bar as B\n");

    assert_eq!(dump, sexp!(Module (Zone uses (Import Foo Bar alias=B))));

    assert!(diagnostics.is_empty(), "unexpected diagnostics: {diagnostics:?}");
  }

  #[test]
  fn module_header() {
    let (module, _) = parse_module("module Blog\n types\n Id<a> = Int");

    assert_eq!(module.name.unwrap().text, "Blog");
  }

  #[test]
  fn no_module_header() {
    let (module, _) = parse_module("types\n Id<a> = Int");

    assert_eq!(module.name, None);
  }

  #[test]
  fn zone_order() {
    let (module, diagnostics) = parse_module("types\nuses\n  Foo.Bar\n");

    assert_eq!(zone_kinds(&module), [ZoneKind::TYPES, ZoneKind::USES]);
    assert_eq!(codes(&diagnostics), [ZoneOutOfOrder]);
  }

  #[test]
  fn repeated_zone() {
    let (module, diagnostics) = parse_module("functions\nfunctions\n");

    assert_eq!(zone_kinds(&module), [ZoneKind::FUNCTIONS, ZoneKind::FUNCTIONS]);
    assert_eq!(codes(&diagnostics), [ZoneDuplicate]);
  }

  #[test]
  fn empty_zone_is_legal() {
    let (module, diagnostics) = parse_module("module Blog\n functions\n");

    assert_eq!(zone_kinds(&module), [ZoneKind::FUNCTIONS]);
    assert!(module.zones[0].decls.is_empty());
    assert!(diagnostics.is_empty(), "unexpected diagnostics: {diagnostics:?}");
  }

  #[test]
  fn declaration_before_any_zone() {
    let (_, diagnostics) = parse_module("module Blog\n f() { 0 }");

    let expected = find_code(&diagnostics, ExpectedDeclaration);
    let help = expected.help.as_deref().unwrap_or_default();

    assert!(
      help.contains("`functions`"),
      "help should name `functions`: {help}"
    );
  }

  #[test]
  fn wrong_zone() {
    let (module, diagnostics) = parse_module(&fn_source("Id = Int"));

    match single_decl(&module, ZoneKind::FUNCTIONS) {
      Decl::Type(type_decl) => assert_eq!(type_decl.name.text, "Id"),
      other => panic!(
        "expected the `TypeDecl` to stay in the `functions` zone, got {other:?}"
      ),
    }

    let wrong_zone = find_code(&diagnostics, DeclWrongZone);

    assert!(
      wrong_zone.message.contains("types"),
      "expected the diagnostic to name the `types` zone, got: {}",
      wrong_zone.message
    );
  }

  #[test]
  fn keyword_is_supplied_by_the_zone() {
    let src = fn_source("function f() { 0 }");
    let (module, diagnostics) = parse_module(&src);

    assert_eq!(codes(&diagnostics), [RedundantDeclarationKeyword]);

    match single_decl(&module, ZoneKind::FUNCTIONS) {
      Decl::Fn(fn_decl) => {
        assert_eq!(fn_decl.name.text, "f");
        assert_eq!(
          fn_decl.span.start,
          src.find("f()").unwrap(),
          "the `FnDecl` should start at its name, as if `function` were absent"
        );
      }
      other => panic!("expected a `FnDecl`, got {other:?}"),
    }
  }

  #[test]
  fn visibility_is_a_zone() {
    let (module, diagnostics) = parse_module(&fn_source("pub f() { 0 }"));

    assert_eq!(codes(&diagnostics), [VisibilityKeyword]);

    let visibility = find_code(&diagnostics, VisibilityKeyword);

    assert!(
      visibility.message.contains("`exports`"),
      "expected the diagnostic to name the `exports` zone, got: {}",
      visibility.message
    );

    match single_decl(&module, ZoneKind::FUNCTIONS) {
      Decl::Fn(fn_decl) => assert_eq!(fn_decl.name.text, "f"),
      other => panic!("expected a `FnDecl`, got {other:?}"),
    }
  }

  #[test]
  fn exports_are_names() {
    let (dump, diagnostics) = dump_module("exports\n  main\n  slug\n");

    assert_eq!(
      dump,
      sexp!(Module (Zone exports (ExportDecl main) (ExportDecl slug)))
    );

    assert!(diagnostics.is_empty(), "unexpected diagnostics: {diagnostics:?}");
  }
}

mod types {

  use super::*;

  #[test]
  fn record_alias_with_derive() {
    let (dump, diagnostics) = dump_module(&type_source(
      "\
Post = {
    id: Id<Post>,
    title: String,
    author_id: Id<User>,
  } derive(Json, Sql, Eq)",
    ));

    assert_eq!(
      dump,
      sexp!(Module
        (Zone types
          (TypeDecl Post
            (RecordType
              (FieldType id (TypeRef Id (TypeRef Post)))
              (FieldType title (TypeRef String))
              (FieldType author_id (TypeRef Id (TypeRef User))))
            (Derive Json Sql Eq))))
    );

    assert!(diagnostics.is_empty(), "unexpected diagnostics: {diagnostics:?}");
  }

  #[test]
  fn alias_to_named_type() {
    let (dump, diagnostics) = dump_module(&type_source("Id = Int"));

    assert_eq!(
      dump,
      sexp!(Module
        (Zone types
          (TypeDecl Id
            (TypeRef Int))))
    );

    assert!(diagnostics.is_empty(), "unexpected diagnostics: {diagnostics:?}");
  }

  #[test]
  fn generic_alias() {
    let (dump, diagnostics) = dump_module(&type_source("Box<a> = { v: a }"));

    assert_eq!(
      dump,
      sexp!(Module
        (Zone types
          (TypeDecl Box a
            (RecordType
              (FieldType v (TypeVar a))))))
    );

    assert!(diagnostics.is_empty(), "unexpected diagnostics: {diagnostics:?}");
  }

  #[test]
  fn variant_nullary() {
    let (dump, diagnostics) = dump_module(&type_source("Color = Red | Green"));

    assert_eq!(
      dump,
      sexp!(Module
        (Zone types
          (TypeDecl Color
            (VariantBody (CtorDecl Red) (CtorDecl Green)))))
    );

    assert!(diagnostics.is_empty(), "unexpected diagnostics: {diagnostics:?}");
  }

  #[test]
  fn variant_with_arguments() {
    let (dump, diagnostics) =
      dump_module(&type_source("Shape = Circle(Float) | Square(Float)"));

    assert_eq!(
      dump,
      sexp!(Module
        (Zone types
          (TypeDecl Shape
            (VariantBody
              (CtorDecl Circle (TypeRef Float))
              (CtorDecl Square (TypeRef Float))))))
    );

    assert!(diagnostics.is_empty(), "unexpected diagnostics: {diagnostics:?}");
  }

  #[test]
  fn variant_leading_bar() {
    let (dump, diagnostics) =
      dump_module(&type_source("Color = | Red | Green"));

    assert_eq!(
      dump,
      sexp!(Module
        (Zone types
          (TypeDecl Color
            (VariantBody (CtorDecl Red) (CtorDecl Green)))))
    );

    assert!(diagnostics.is_empty(), "unexpected diagnostics: {diagnostics:?}");
  }

  #[test]
  fn single_nullary_variant() {
    let (dump, diagnostics) = dump_module(&type_source("Unit = | Unit"));

    assert_eq!(
      dump,
      sexp!(Module (Zone types (TypeDecl Unit (VariantBody (CtorDecl Unit)))))
    );

    assert!(diagnostics.is_empty(), "unexpected diagnostics: {diagnostics:?}");
  }

  #[test]
  fn bare_upper_is_an_alias() {
    let (dump, diagnostics) = dump_module(&type_source("A = B"));

    assert_eq!(dump, sexp!(Module (Zone types (TypeDecl A (TypeRef B)))));

    assert!(diagnostics.is_empty(), "unexpected diagnostics: {diagnostics:?}");
  }

  #[test]
  fn upper_with_arguments_is_a_variant() {
    let (dump, diagnostics) = dump_module(&type_source("A = B(Int)"));

    assert_eq!(
      dump,
      sexp!(Module
        (Zone types
          (TypeDecl A (VariantBody (CtorDecl B (TypeRef Int))))))
    );

    assert!(diagnostics.is_empty(), "unexpected diagnostics: {diagnostics:?}");
  }

  #[test]
  fn generic_variant() {
    let (dump, diagnostics) =
      dump_module(&type_source("List<a> = Nil | Cons(a, List<a>)"));

    assert_eq!(
      dump,
      sexp!(Module
        (Zone types
          (TypeDecl List a
            (VariantBody
              (CtorDecl Nil)
              (CtorDecl Cons (TypeVar a) (TypeRef List (TypeVar a)))))))
    );

    assert!(diagnostics.is_empty(), "unexpected diagnostics: {diagnostics:?}");
  }

  #[test]
  fn derive_on_a_variant() {
    let (dump, diagnostics) = dump_module(&type_source("S = A | B derive(Eq)"));

    assert_eq!(
      dump,
      sexp!(Module
        (Zone types
          (TypeDecl S
            (VariantBody (CtorDecl A) (CtorDecl B))
            (Derive Eq))))
    );

    assert!(diagnostics.is_empty(), "unexpected diagnostics: {diagnostics:?}");
  }

  #[test]
  fn multi_line_variant() {
    let (dump, diagnostics) =
      dump_module("types\n  Status =\n    | Draft\n    | Published(Int)\n");

    assert_eq!(
      dump,
      sexp!(Module
        (Zone types
          (TypeDecl Status
            (VariantBody (CtorDecl Draft) (CtorDecl Published (TypeRef Int))))))
    );

    assert!(diagnostics.is_empty(), "unexpected diagnostics: {diagnostics:?}");
  }
}

mod functions {

  use super::*;

  #[test]
  fn fn_no_params() {
    let (dump, diagnostics) = dump_module(&fn_source("f() { 0 }"));

    assert_eq!(
      dump,
      sexp!(Module (Zone functions (FnDecl f (Block (IntLit raw="0")))))
    );

    assert!(diagnostics.is_empty(), "unexpected diagnostics: {diagnostics:?}");
  }

  #[test]
  fn fn_typed_params() {
    let (dump, diagnostics) =
      dump_module(&fn_source("f(a: Int, b: String) { 0 }"));

    assert_eq!(
      dump,
      sexp!(Module
        (Zone functions
          (FnDecl f
            (Param a (TypeRef Int))
            (Param b (TypeRef String))
            (Block (IntLit raw="0")))))
    );

    assert!(diagnostics.is_empty(), "unexpected diagnostics: {diagnostics:?}");
  }

  #[test]
  fn fn_untyped_param() {
    let (module, diagnostics) = parse_module(&fn_source("f(a) { 0 }"));
    let f = fn_decl(single_decl(&module, ZoneKind::FUNCTIONS));

    match &f.params[..] {
      [param] => {
        assert_eq!(param.name.text, "a");
        assert_eq!(param.ty, None);
      }
      other => panic!("expected one `Param`, got {other:?}"),
    }

    assert!(diagnostics.is_empty(), "unexpected diagnostics: {diagnostics:?}");
  }

  #[test]
  fn fn_return_type() {
    let (module, diagnostics) = parse_module(&fn_source("f() -> Int { 0 }"));
    let f = fn_decl(single_decl(&module, ZoneKind::FUNCTIONS));

    assert!(
      matches!(&f.return_type, Some(TypeExpr::Ref(ty)) if ty.name.text == "Int")
    );
    assert_eq!(f.effects, None);
    assert!(diagnostics.is_empty(), "unexpected diagnostics: {diagnostics:?}");
  }

  #[test]
  fn fn_return_and_effects() {
    let (dump, diagnostics) =
      dump_module(&fn_source("f() -> Int / {Db} { 0 }"));

    assert_eq!(
      dump,
      sexp!(Module
        (Zone functions
          (FnDecl f
            (TypeRef Int)
            (EffectRow (TypeRef Db))
            (Block (IntLit raw="0")))))
    );

    assert!(diagnostics.is_empty(), "unexpected diagnostics: {diagnostics:?}");
  }

  #[test]
  fn fn_span_starts_at_the_name() {
    let src = fn_source("f() { 0 }");
    let (module, _) = parse_module(&src);
    let f = fn_decl(single_decl(&module, ZoneKind::FUNCTIONS));

    assert_eq!(
      (f.span.start, f.span.end),
      (src.find("f()").unwrap(), src.rfind('}').unwrap() + 1),
      "the `FnDecl` runs from its name through the closing `}}`"
    );
  }

  #[test]
  fn body_span_covers_the_braces() {
    let src = fn_source(r##"f() { { "#{ x }" } }"##);
    let (module, diagnostics) = parse_module(&src);
    let f = fn_decl(single_decl(&module, ZoneKind::FUNCTIONS));

    let body = &f.body.span;
    let open = src.find('{').unwrap();
    let close = src.rfind('}').unwrap() + 1;

    assert_eq!((body.start, body.end), (open, close));
    assert!(diagnostics.is_empty(), "unexpected diagnostics: {diagnostics:?}");
  }

  #[test]
  fn goal_create_post() {
    let src = fn_source(
      "\
create_post(body: NewPost) -> Post / {Db, Auth, Throws<Invalid>} {
    let user = Auth.require()
    let post = Db.insert(Posts, { ..body, author_id: user.id })
    Log.info(\"created #{post.id}\")
    post
  }",
    );
    let (dump, diagnostics) = dump_module(&src);

    assert_eq!(
      dump,
      sexp!(Module
        (Zone functions
          (FnDecl create_post
            (Param body (TypeRef NewPost))
            (TypeRef Post)
            (EffectRow
              (TypeRef Db)
              (TypeRef Auth)
              (TypeRef Throws (TypeRef Invalid)))
            (Block
              (LetStmt (PVar user) (Call (FieldAccess require (Var Auth))))
              (LetStmt
                (PVar post)
                (Call
                  (FieldAccess insert (Var Db))
                  (Var Posts)
                  (RecordLit
                    (Var body)
                    (FieldInit author_id (FieldAccess id (Var user))))))
              (ExprStmt
                (Call
                  (FieldAccess info (Var Log))
                  (StringLit
                    (StringText raw="created " value="created ")
                    (StringInterp (FieldAccess id (Var post))))))
              (Var post)))))
    );

    assert!(diagnostics.is_empty(), "unexpected diagnostics: {diagnostics:?}");
  }

  #[test]
  fn goal_slug() {
    let src = fn_source(
      "\
slug(r: { title: String | rest }) -> String {
    r.title |> String.lowercase |> String.replace(\" \", \"-\")
  }",
    );
    let (dump, diagnostics) = dump_module(&src);

    assert_eq!(
      dump,
      sexp!(Module
        (Zone functions
          (FnDecl slug
            (Param r (RecordType rest (FieldType title (TypeRef String))))
            (TypeRef String)
            (Block
              (Pipe
                (Pipe
                  (FieldAccess title (Var r))
                  (FieldAccess lowercase (Var String)))
                (Call
                  (FieldAccess replace (Var String))
                  (StringLit (StringText raw=" " value=" "))
                  (StringLit (StringText raw="-" value="-"))))))))
    );

    assert!(diagnostics.is_empty(), "unexpected diagnostics: {diagnostics:?}");
  }
}

mod docs {

  use super::*;

  #[test]
  fn docs_attach_to_the_next_declaration() {
    let src = "\
functions
  /// a
  // plain
  /// b
  f() { 0 }
  /// c
  g() { 0 }
";
    let (module, diagnostics) = parse_module(src);

    let doc = |text: &str| {
      let start = src.find(text).unwrap();

      (start, start + text.len())
    };
    let spans = |decl: &Decl| -> Vec<(usize, usize)> {
      fn_decl(decl).docs.iter().map(|s| (s.start, s.end)).collect()
    };

    let decls = &module.zones[0].decls;

    assert_eq!(
      spans(&decls[0]),
      [doc("/// b")],
      "the plain comment breaks the run"
    );
    assert_eq!(spans(&decls[1]), [doc("/// c")]);
    assert!(diagnostics.is_empty(), "unexpected diagnostics: {diagnostics:?}");
  }

  #[test]
  fn docs_attach_to_type_declarations() {
    let src = "types\n  /// An id.\n  /// Opaque.\n  Id = Int\n";
    let (module, _) = parse_module(src);
    let id = type_decl(single_decl(&module, ZoneKind::TYPES));

    assert_eq!(id.docs.len(), 2);
  }
}

mod recovery {

  use super::*;

  #[test]
  fn sync_to_next_declaration() {
    let (module, diagnostics) =
      parse_module("functions\n  f() { 0 }\n  g(a b) { 0 }\n  h() { 0 }\n");

    assert_eq!(codes(&diagnostics), [UnexpectedToken]);
    assert_eq!(decl_names(&module), ["f", "h"]);
  }

  #[test]
  fn does_not_sync_on_a_nested_keyword() {
    let (module, diagnostics) =
      parse_module("types\n  Bad = { a: Int functions }\n  Ok = Int\n");

    assert_eq!(codes(&diagnostics), [UnexpectedToken]);
    assert_eq!(zone_kinds(&module), [ZoneKind::TYPES]);
    assert_eq!(decl_names(&module), ["Ok"]);
  }

  #[test]
  fn local_recovery_in_a_signature() {
    let (module, diagnostics) =
      parse_module(&fn_source("f(a: , b: Int) { 0 }"));
    let f = fn_decl(single_decl(&module, ZoneKind::FUNCTIONS));

    assert_eq!(codes(&diagnostics), [UnexpectedToken]);
    assert!(matches!(f.params[0].ty, Some(TypeExpr::Invalid(_))));
    assert!(
      matches!(&f.params[1].ty, Some(TypeExpr::Ref(ty)) if ty.name.text == "Int")
    );
  }

  #[test]
  fn unbalanced_close_brace() {
    let (module, diagnostics) = parse_module("functions\n  }\n  f() { 0 }\n");

    assert_eq!(codes(&diagnostics), [ExpectedDeclaration]);
    assert_eq!(decl_names(&module), ["f"]);
  }

  #[test]
  fn declaration_keyword_is_rejected() {
    let (module, diagnostics) = parse_module(&type_source("type Id = Int"));

    assert_eq!(codes(&diagnostics), [RedundantDeclarationKeyword]);
    assert!(
      diagnostics[0].message.contains("the zone supplies the keyword"),
      "got: {}",
      diagnostics[0].message
    );

    let id = type_decl(single_decl(&module, ZoneKind::TYPES));

    assert_eq!(id.name.text, "Id");
    assert!(matches!(id.body, TypeBody::Alias(_)));
  }

  #[test]
  fn top_level_let() {
    let (module, diagnostics) = parse_module(&fn_source("let x = 1"));

    assert_eq!(codes(&diagnostics), [ExpectedDeclaration]);
    assert_eq!(
      diagnostics[0].help.as_deref(),
      Some("top-level values go in a `constants` zone, like `pi = 3.14`")
    );
    assert!(module.zones[0].decls.is_empty());
  }

  #[test]
  fn top_level_garbage() {
    let (module, diagnostics) =
      parse_module("types\n  42\nfunctions\n  f() { 0 }\n");

    assert_eq!(codes(&diagnostics), [ExpectedDeclaration]);
    assert_eq!(zone_kinds(&module), [ZoneKind::TYPES, ZoneKind::FUNCTIONS]);
    assert_eq!(
      decl_names(&module),
      ["f"],
      "parsing continues past the garbage"
    );
  }

  #[test]
  fn lowercase_type_name() {
    let (module, diagnostics) = parse_module(&type_source("post = Int"));

    assert_eq!(codes(&diagnostics), [WrongIdentifierCase]);
    assert_eq!(diagnostics[0].help.as_deref(), Some("write it as `Post`"));
    assert_eq!(
      type_decl(single_decl(&module, ZoneKind::TYPES)).name.text,
      "post"
    );
  }

  #[test]
  fn uppercase_fn_name() {
    let (module, diagnostics) = parse_module(&fn_source("Foo() { 0 }"));

    assert_eq!(codes(&diagnostics), [WrongIdentifierCase]);
    assert_eq!(diagnostics[0].help.as_deref(), Some("write it as `foo`"));
    assert_eq!(
      fn_decl(single_decl(&module, ZoneKind::FUNCTIONS)).name.text,
      "Foo"
    );
  }

  #[test]
  fn lowercase_constructor_name() {
    let (_, diagnostics) = parse_module(&type_source("Color = Red | green"));

    assert_eq!(codes(&diagnostics), [WrongIdentifierCase]);
    assert_eq!(diagnostics[0].help.as_deref(), Some("write it as `Green`"));
  }

  #[test]
  fn unterminated_body_is_dropped() {
    let (module, diagnostics) = parse_module(&fn_source("f() {"));

    assert_eq!(codes(&diagnostics), [UnexpectedToken]);
    assert!(module.zones[0].decls.is_empty());
  }
}

mod properties {

  use super::*;
  use proptest::prelude::*;
  use std::thread;

  const TEXTS: &[&str] = &[
    "module",
    "uses",
    "types",
    "functions",
    "exports",
    "function",
    "let",
    "derive",
    "match",
    "if",
    "else",
    "as",
    "true",
    "false",
    "type",
    "import",
    "pub",
    "export",
    "(",
    ")",
    "{",
    "}",
    "[",
    "]",
    ",",
    ":",
    ".",
    "->",
    "|>",
    "=",
    "==",
    "!=",
    "<",
    "<=",
    ">",
    ">=",
    "+",
    "-",
    "*",
    "/",
    "%",
    "&&",
    "||",
    "!",
    "|",
    "..",
    "_",
    "f",
    "main",
    "rest",
    "Int",
    "Post",
    "Id",
    "42",
    "1.5",
    "\"s\"",
    "\"#{x}\"",
    "/// doc",
    "// note",
  ];

  const SEPARATORS: &[&str] = &[" ", " ", "\n", "\n  ", "\n    "];

  fn soup() -> impl Strategy<Value = String> {
    let piece = prop_oneof![
      8 => prop::sample::select(TEXTS).prop_map(str::to_string),
      1 => (1usize..3000, prop::sample::select(&["(", "{", "<"][..]))
        .prop_map(|(n, open)| open.repeat(n)),
    ];

    prop::collection::vec((piece, prop::sample::select(SEPARATORS)), 0..120)
      .prop_map(|pieces| {
        pieces.into_iter().map(|(text, sep)| text + sep).collect()
      })
  }

  fn parse_on_small_stack(src: String) {
    thread::Builder::new()
      .stack_size(1024 * 1024)
      .spawn(move || parse_module(&src))
      .expect("spawn the parser thread")
      .join()
      .expect("parsing on a small stack completed");
  }

  proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn parse_terminates_with_valid_spans(src in soup()) {
      parse_module(&src);
    }

    #[test]
    fn soup_parses_on_a_small_stack(src in soup()) {
      parse_on_small_stack(src);
    }
  }

  #[test]
  fn deep_types_parse_on_a_small_stack() {
    let n = 5000;

    for (open, close) in
      [("(", ")"), ("function(", ") -> Int"), ("{ a: ", " }")]
    {
      let src =
        format!("types\n  A = {}Int{}\n", open.repeat(n), close.repeat(n));

      parse_on_small_stack(src);
    }
  }
}
