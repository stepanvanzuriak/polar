use polar_compiler::{
  shared::codes::DiagnosticCode,
  shared::diagnostic::{Diagnostic, DiagnosticBag, Label, dump_diagnostics},
  shared::source::{SourceFile, Span},
  syntax::lexer::{
    dump::dump_tokens,
    token::{Token, TokenKind},
  },
};
use std::sync::Arc;

fn span(start: usize, end: usize) -> Span {
  Span::new(Arc::from("test.px"), start, end)
}

mod tokens {
  use super::*;

  #[test]
  fn dump_one_token() {
    let file = SourceFile::new("test.px", "function");
    let tokens = vec![Token {
      kind: TokenKind::KwFunction,
      span: span(0, 8),
      newline_before: false,
      value: None,
    }];

    assert_eq!(dump_tokens(&file, &tokens), "1:1   KwFunction  \"function\"\n");
  }

  #[test]
  fn dump_newline_marker() {
    let file = SourceFile::new("test.px", "a\nb");
    let tokens = vec![Token {
      kind: TokenKind::Lower,
      span: span(2, 3),
      newline_before: true,
      value: None,
    }];

    assert_eq!(dump_tokens(&file, &tokens), "2:1 \u{21b5} Lower       \"b\"\n");
  }

  #[test]
  fn dump_escapes_raw_text() {
    let file = SourceFile::new("test.px", "\\n");
    let tokens = vec![Token {
      kind: TokenKind::StringPart,
      span: span(0, 2),
      newline_before: false,
      value: Some("\n".to_string()),
    }];

    let dump = dump_tokens(&file, &tokens);
    let raw_text = format!("{:?}", "\\n");
    let value_text = format!("{:?}", "\n");

    let raw_pos =
      dump.find(&raw_text).expect("dump should show raw token text");
    let value_pos =
      dump.find(&value_text).expect("dump should show the parsed value");

    assert!(
      raw_pos < value_pos,
      "raw text must appear before the value suffix"
    );
  }

  #[test]
  fn dump_omits_equal_value() {
    let file = SourceFile::new("test.px", "ab");
    let tokens = vec![Token {
      kind: TokenKind::StringPart,
      span: span(0, 2),
      newline_before: false,
      value: Some("ab".to_string()),
    }];

    let dump = dump_tokens(&file, &tokens);

    assert!(
      !dump.contains("value="),
      "value suffix should be omitted when it duplicates the raw text"
    );
  }

  #[test]
  fn dump_column_counts_code_points() {
    let file = SourceFile::new("test.px", "\u{1F600}x");
    let tokens = vec![Token {
      kind: TokenKind::Lower,
      span: span(4, 5),
      newline_before: false,
      value: None,
    }];

    assert_eq!(dump_tokens(&file, &tokens), "1:2   Lower       \"x\"\n");
  }
}

mod diagnostics {
  use super::*;

  #[test]
  fn dump_diagnostics_is_sorted() {
    let file = SourceFile::new("test.px", "x".repeat(30));
    let mut bag = DiagnosticBag::default();

    bag.push(Diagnostic::error(
      DiagnosticCode::InternalCompilerError,
      "second problem",
      Label::new(span(20, 24)),
    ));
    bag.push(Diagnostic::warning(
      DiagnosticCode::InternalCompilerError,
      "first problem",
      Label::new(span(4, 6)),
    ));

    let dump = dump_diagnostics(&file, &bag.into_sorted());

    let first_pos =
      dump.find("first problem").expect("earlier diagnostic missing");
    let second_pos =
      dump.find("second problem").expect("later diagnostic missing");

    assert!(
      first_pos < second_pos,
      "diagnostics must dump in source order, not push order"
    );
    assert!(dump.contains("warning POLAR0001 1:5-1:7 first problem"));
    assert!(dump.contains("error POLAR0001 1:21-1:25 second problem"));
  }
}

mod ast {
  use polar_compiler::{
    shared::source::Span,
    syntax::ast::{
      Block, CtorDecl, Decl, Derive, EffectRow, ExportDecl, Expr, FieldType,
      FnDecl, FnType, Import, InvalidExpr, InvalidType, Module, Name, Param,
      RecordType, TypeBody, TypeDecl, TypeExpr, TypeRef, TypeVar, VariantBody,
      Zone, ZoneKind,
      dump::{dump_ast, dump_ast_shape},
    },
  };
  use std::sync::Arc;

  struct Src(&'static str);

  impl Src {
    fn all(&self) -> Span {
      Span::new(Arc::from("test.px"), 0, self.0.len())
    }

    fn at(&self, needle: &str, nth: usize) -> Span {
      let (start, _) =
        self.0.match_indices(needle).nth(nth).unwrap_or_else(|| {
          panic!("no occurrence {nth} of {needle:?} in the fixture")
        });

      Span::new(Arc::from("test.px"), start, start + needle.len())
    }

    fn point(&self, needle: &str, nth: usize) -> Span {
      Span::empty(Arc::from("test.px"), self.at(needle, nth).start)
    }

    fn cover(&self, from: (&str, usize), to: (&str, usize)) -> Span {
      self.at(from.0, from.1).join(&self.at(to.0, to.1))
    }

    fn name(&self, needle: &str, nth: usize) -> Name {
      Name { text: needle.to_string(), span: self.at(needle, nth) }
    }

    fn type_ref(&self, needle: &str, nth: usize) -> TypeExpr {
      TypeExpr::Ref(TypeRef {
        span: self.at(needle, nth),
        name: self.name(needle, nth),
        args: vec![],
      })
    }
  }

  fn placeholder(span: Span) -> Block {
    Block {
      span: span.clone(),
      stmts: vec![],
      result: Box::new(Expr::Invalid(InvalidExpr { span })),
    }
  }

  fn module(src: &Src, zones: Vec<Zone>) -> Module {
    Module { span: src.all(), name: None, zones }
  }

  #[test]
  fn empty_module() {
    let src = Src("");

    assert_eq!(dump_ast(&module(&src, vec![])), "(Module 0..0)\n");
  }

  #[test]
  fn empty_zone() {
    let src = Src("types\n");
    let zone =
      Zone { span: src.at("types", 0), kind: ZoneKind::TYPES, decls: vec![] };

    assert_eq!(
      dump_ast(&module(&src, vec![zone])),
      "\
(Module 0..6
  (Zone types 0..5))
"
    );
  }

  #[test]
  fn type_alias() {
    let src = Src("types\n  Id = Int\n");
    let zone = Zone {
      span: src.cover(("types", 0), ("Int", 0)),
      kind: ZoneKind::TYPES,
      decls: vec![Decl::Type(TypeDecl {
        span: src.cover(("Id", 0), ("Int", 0)),
        docs: vec![],
        name: src.name("Id", 0),
        params: vec![],
        body: TypeBody::Alias(src.type_ref("Int", 0)),
        derive: None,
      })],
    };

    assert_eq!(
      dump_ast(&module(&src, vec![zone])),
      "\
(Module 0..17
  (Zone types 0..16
    (TypeDecl Id 8..16
      (TypeRef Int 13..16))))
"
    );
  }

  #[test]
  fn fn_signature() {
    let src = Src("functions\n  f() -> Int {}\n");
    let zone = Zone {
      span: src.cover(("functions", 0), ("{}", 0)),
      kind: ZoneKind::FUNCTIONS,
      decls: vec![Decl::Fn(FnDecl {
        bounds: vec![],
        span: src.cover(("f(", 0), ("{}", 0)),
        docs: vec![],
        name: src.name("f", 1),
        params: vec![],
        return_type: Some(src.type_ref("Int", 0)),
        effects: None,
        body: placeholder(src.at("{}", 0)),
      })],
    };

    assert_eq!(
      dump_ast(&module(&src, vec![zone])),
      "\
(Module 0..26
  (Zone functions 0..25
    (FnDecl f 12..25
      (TypeRef Int 19..22)
      (Block 23..25
        (InvalidExpr 23..25)))))
"
    );
  }

  #[test]
  fn effect_row_span_starts_at_the_slash() {
    let src = Src("functions\n  f() -> Int / {Db} {}\n");
    let zone = Zone {
      span: src.cover(("functions", 0), ("{}", 0)),
      kind: ZoneKind::FUNCTIONS,
      decls: vec![Decl::Fn(FnDecl {
        bounds: vec![],
        span: src.cover(("f(", 0), ("{}", 0)),
        docs: vec![],
        name: src.name("f", 1),
        params: vec![],
        return_type: Some(src.type_ref("Int", 0)),
        effects: Some(EffectRow {
          span: src.at("/ {Db}", 0),
          entries: vec![TypeRef {
            span: src.at("Db", 0),
            name: src.name("Db", 0),
            args: vec![],
          }],
          tail: None,
        }),
        body: placeholder(src.at("{}", 0)),
      })],
    };

    assert_eq!(
      dump_ast(&module(&src, vec![zone])),
      "\
(Module 0..33
  (Zone functions 0..32
    (FnDecl f 12..32
      (TypeRef Int 19..22)
      (EffectRow 23..29
        (TypeRef Db 26..28))
      (Block 30..32
        (InvalidExpr 30..32)))))
"
    );
  }

  #[test]
  fn none_fields_are_absent() {
    let src = Src("functions\n  f() {}\n");
    let zone = Zone {
      span: src.cover(("functions", 0), ("{}", 0)),
      kind: ZoneKind::FUNCTIONS,
      decls: vec![Decl::Fn(FnDecl {
        bounds: vec![],
        span: src.cover(("f(", 0), ("{}", 0)),
        docs: vec![],
        name: src.name("f", 1),
        params: vec![],
        return_type: None,
        effects: None,
        body: placeholder(src.at("{}", 0)),
      })],
    };

    let dump = dump_ast(&module(&src, vec![zone]));

    assert_eq!(
      dump,
      "\
(Module 0..19
  (Zone functions 0..18
    (FnDecl f 12..18
      (Block 16..18
        (InvalidExpr 16..18)))))
"
    );
    assert!(!dump.contains("TypeRef"));
    assert!(!dump.contains("EffectRow"));
  }

  #[test]
  fn indent_is_two_spaces_per_level() {
    let src = Src("types\n  A = Id<Post>\n");
    let zone = Zone {
      span: src.cover(("types", 0), ("Id<Post>", 0)),
      kind: ZoneKind::TYPES,
      decls: vec![Decl::Type(TypeDecl {
        span: src.cover(("A", 0), ("Id<Post>", 0)),
        docs: vec![],
        name: src.name("A", 0),
        params: vec![],
        body: TypeBody::Alias(TypeExpr::Ref(TypeRef {
          span: src.at("Id<Post>", 0),
          name: src.name("Id", 0),
          args: vec![src.type_ref("Post", 0)],
        })),
        derive: None,
      })],
    };

    let dump = dump_ast(&module(&src, vec![zone]));
    let indents: Vec<usize> =
      dump.lines().map(|line| line.len() - line.trim_start().len()).collect();

    assert_eq!(
      dump,
      "\
(Module 0..21
  (Zone types 0..20
    (TypeDecl A 8..20
      (TypeRef Id 12..20
        (TypeRef Post 15..19)))))
"
    );
    assert_eq!(indents, [0, 2, 4, 6, 8]);
  }

  #[test]
  fn docs_do_not_dump() {
    let src = Src("functions\n  f() {}\n");
    let zone = Zone {
      span: src.cover(("functions", 0), ("{}", 0)),
      kind: ZoneKind::FUNCTIONS,
      decls: vec![Decl::Fn(FnDecl {
        bounds: vec![],
        span: src.cover(("f(", 0), ("{}", 0)),
        docs: vec![src.at("functions", 0)],
        name: src.name("f", 1),
        params: vec![],
        return_type: None,
        effects: None,
        body: placeholder(src.at("{}", 0)),
      })],
    };

    assert!(!dump_ast(&module(&src, vec![zone])).contains("0..9"));
  }

  const EVERY_KIND: &str = "\
uses
  net/http as web
types
  Id<k> = k
  Shape = | Circle(Int) | Square
    derive Eq
functions
  run(arg: Int, flag) -> Bool / {Db | eff} {}
  odd(cb: (Int) -> Bool, rec: {num: Int | rest}) -> Bad {}
  bad() -> {}
exports
  run
";

  fn uses_zone(src: &Src) -> Zone {
    Zone {
      span: src.cover(("uses", 0), ("web", 0)),
      kind: ZoneKind::USES,
      decls: vec![Decl::Import(Import {
        methods: None,
        span: src.cover(("net", 0), ("web", 0)),
        path: vec![src.name("net", 0), src.name("http", 0)],
        alias: Some(src.name("web", 0)),
      })],
    }
  }

  fn types_zone(src: &Src) -> Zone {
    let id = Decl::Type(TypeDecl {
      span: src.cover(("Id", 0), ("k", 1)),
      docs: vec![],
      name: src.name("Id", 0),
      params: vec![src.name("k", 0)],
      body: TypeBody::Alias(TypeExpr::Var(TypeVar {
        span: src.at("k", 1),
        name: src.name("k", 1),
      })),
      derive: None,
    });

    let shape = Decl::Type(TypeDecl {
      span: src.cover(("Shape", 0), ("Eq", 0)),
      docs: vec![],
      name: src.name("Shape", 0),
      params: vec![],
      body: TypeBody::Variants(VariantBody {
        span: src.cover(("| Circle", 0), ("Square", 0)),
        ctors: vec![
          CtorDecl {
            span: src.at("Circle(Int)", 0),
            name: src.name("Circle", 0),
            args: vec![src.type_ref("Int", 0)],
          },
          CtorDecl {
            span: src.at("Square", 0),
            name: src.name("Square", 0),
            args: vec![],
          },
        ],
      }),
      derive: Some(Derive {
        span: src.at("derive Eq", 0),
        names: vec![src.name("Eq", 0)],
      }),
    });

    Zone {
      span: src.cover(("types", 0), ("Eq", 0)),
      kind: ZoneKind::TYPES,
      decls: vec![id, shape],
    }
  }

  fn run_fn(src: &Src) -> Decl {
    Decl::Fn(FnDecl {
      bounds: vec![],
      span: src.cover(("run", 0), ("{}", 0)),
      docs: vec![],
      name: src.name("run", 0),
      params: vec![
        Param {
          span: src.cover(("arg", 0), ("Int", 1)),
          name: src.name("arg", 0),
          pattern: None,
          ty: Some(src.type_ref("Int", 1)),
        },
        Param {
          span: src.at("flag", 0),
          name: src.name("flag", 0),
          pattern: None,
          ty: None,
        },
      ],
      return_type: Some(src.type_ref("Bool", 0)),
      effects: Some(EffectRow {
        span: src.at("/ {Db | eff}", 0),
        entries: vec![TypeRef {
          span: src.at("Db", 0),
          name: src.name("Db", 0),
          args: vec![],
        }],
        tail: Some(src.name("eff", 0)),
      }),
      body: placeholder(src.at("{}", 0)),
    })
  }

  fn odd_fn(src: &Src) -> Decl {
    Decl::Fn(FnDecl {
      bounds: vec![],
      span: src.cover(("odd", 0), ("{}", 1)),
      docs: vec![],
      name: src.name("odd", 0),
      params: vec![
        Param {
          span: src.cover(("cb", 0), ("Bool", 1)),
          name: src.name("cb", 0),
          pattern: None,
          ty: Some(TypeExpr::Fn(FnType {
            span: src.at("(Int) -> Bool", 0),
            params: vec![src.type_ref("Int", 2)],
            ret: Box::new(src.type_ref("Bool", 1)),
            effects: None,
          })),
        },
        Param {
          span: src.cover(("rec", 0), ("rest}", 0)),
          name: src.name("rec", 0),
          pattern: None,
          ty: Some(TypeExpr::Record(RecordType {
            span: src.at("{num: Int | rest}", 0),
            fields: vec![FieldType {
              span: src.at("num: Int", 0),
              name: src.name("num", 0),
              ty: src.type_ref("Int", 3),
            }],
            tail: Some(src.name("rest", 0)),
          })),
        },
      ],
      return_type: Some(src.type_ref("Bad", 0)),
      effects: None,
      body: placeholder(src.at("{}", 1)),
    })
  }

  fn bad_fn(src: &Src) -> Decl {
    Decl::Fn(FnDecl {
      bounds: vec![],
      span: src.cover(("bad", 0), ("{}", 2)),
      docs: vec![],
      name: src.name("bad", 0),
      params: vec![],
      return_type: Some(TypeExpr::Invalid(InvalidType {
        span: src.point("{}", 2),
      })),
      effects: None,
      body: placeholder(src.at("{}", 2)),
    })
  }

  fn exports_zone(src: &Src) -> Zone {
    Zone {
      span: src.cover(("exports", 0), ("run", 1)),
      kind: ZoneKind::EXPORTS,
      decls: vec![Decl::Export(ExportDecl {
        methods: None,
        span: src.at("run", 1),
        name: src.name("run", 1),
      })],
    }
  }

  fn every_kind() -> Module {
    let src = Src(EVERY_KIND);

    let functions = Zone {
      span: src.cover(("functions", 0), ("{}", 2)),
      kind: ZoneKind::FUNCTIONS,
      decls: vec![run_fn(&src), odd_fn(&src), bad_fn(&src)],
    };

    module(
      &src,
      vec![uses_zone(&src), types_zone(&src), functions, exports_zone(&src)],
    )
  }

  #[test]
  fn dump_is_stable() {
    let module = every_kind();

    assert_eq!(dump_ast(&module), dump_ast(&module));
  }

  #[test]
  fn every_node_kind_dumps() {
    let dump = dump_ast(&every_kind());

    for kind in [
      "Module",
      "Zone",
      "Import",
      "ExportDecl",
      "Derive",
      "TypeDecl",
      "VariantBody",
      "CtorDecl",
      "FnDecl",
      "Param",
      "TypeRef",
      "TypeVar",
      "FnType",
      "RecordType",
      "FieldType",
      "EffectRow",
      "InvalidType",
      "Block",
      "InvalidExpr",
    ] {
      assert!(dump.contains(&format!("({kind} ")), "{kind} did not dump");
    }

    assert!(!dump.contains('?'), "a node kind dumped as `?`");
  }

  #[test]
  fn shape_dump_elides_spans() {
    assert_eq!(
      dump_ast_shape(&every_kind()),
      "\
(Module
  (Zone uses
    (Import net http alias=web))
  (Zone types
    (TypeDecl Id k
      (TypeVar k))
    (TypeDecl Shape
      (VariantBody
        (CtorDecl Circle
          (TypeRef Int))
        (CtorDecl Square))
      (Derive Eq)))
  (Zone functions
    (FnDecl run
      (Param arg
        (TypeRef Int))
      (Param flag)
      (TypeRef Bool)
      (EffectRow eff
        (TypeRef Db))
      (Block
        (InvalidExpr)))
    (FnDecl odd
      (Param cb
        (FnType
          (TypeRef Int)
          (TypeRef Bool)))
      (Param rec
        (RecordType rest
          (FieldType num
            (TypeRef Int))))
      (TypeRef Bad)
      (Block
        (InvalidExpr)))
    (FnDecl bad
      (InvalidType)
      (Block
        (InvalidExpr))))
  (Zone exports
    (ExportDecl run)))
"
    );
  }
}
