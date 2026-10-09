use polar_compiler::{
  shared::source::Span,
  syntax::ast::{
    Binary, BinaryOp, BindDecl, Block, BoolLit, Bound, Call, ConstDecl,
    CtorDecl, Decl, Derive, EffectDecl, EffectRow, Else, ExportDecl, Expr,
    ExprStmt, ExternDecl, FieldAccess, FieldInit, FieldType, FloatLit, FnDecl,
    FnType, HostDecl, If, ImplDecl, Import, IntLit, Lambda, LetStmt, Match,
    MatchArm, MethodSig, Module, Name, PCtor, PField, PLit, PRecord, PVar,
    PWildcard, Param, PatLit, Pattern, Pipe, Recipe, RecordLit, RecordType,
    Stmt, StringInterp, StringLit, StringPart, StringText, Throw, TraitDecl,
    Try, TypeBody, TypeDecl, TypeExpr, TypeRef, TypeVar, Unary, UnaryOp, Var,
    VariantBody, Zone, ZoneKind,
  },
};
use proptest::{
  collection::vec, option, prelude::*, sample::select, strategy::BoxedStrategy,
};
use std::sync::Arc;

pub fn sp() -> Span {
  Span::empty(Arc::from("gen.px"), 0)
}

fn name(text: &str) -> Name {
  Name { text: text.to_string(), span: sp() }
}

const LOWER: &[&str] =
  &["a", "b", "c", "x", "y", "n", "go", "ok", "total", "user_id", "e", "f2"];
const UPPER: &[&str] =
  &["A", "B", "C", "Foo", "Bar", "Some", "None", "Post", "User", "Db", "Log"];

fn lower() -> impl Strategy<Value = Name> + Clone {
  select(LOWER).prop_map(name)
}

fn upper() -> impl Strategy<Value = Name> + Clone {
  select(UPPER).prop_map(name)
}

fn int_raw() -> impl Strategy<Value = String> + Clone {
  prop_oneof![
    "[0-9]{1,5}",
    "[1-9][0-9]{0,2}(_[0-9]{3}){1,2}",
    "[1-9](_[0-9]){1,2}",
  ]
}

fn float_raw() -> impl Strategy<Value = String> + Clone {
  prop_oneof![
    "[0-9]{1,3}\\.[0-9]{1,3}",
    "[0-9]{1,2}\\.[0-9]{1,2}[eE][+-]?[0-9]{1,2}",
    "[1-9][0-9]{0,2}[eE][0-9]{1,2}",
    "[1-9]_[0-9]{3}\\.[0-9](_[0-9]){0,2}",
  ]
}

fn int_lit() -> impl Strategy<Value = IntLit> + Clone {
  int_raw().prop_map(|raw| IntLit { span: sp(), raw })
}

fn float_lit() -> impl Strategy<Value = FloatLit> + Clone {
  float_raw().prop_map(|raw| FloatLit { span: sp(), raw })
}

fn bool_lit() -> impl Strategy<Value = BoolLit> + Clone {
  any::<bool>().prop_map(|value| BoolLit { span: sp(), value })
}

fn text_piece() -> impl Strategy<Value = (String, String)> + Clone {
  let plain = prop_oneof![
    "[a-zA-Z0-9 ,.:!?(){}<>|/-]",
    select(&["é", "😀", "日", "ß", "—"][..]).prop_map(str::to_string),
  ]
  .prop_map(|s| (s.clone(), s));

  let escape = select(
    &[
      ("\\n", "\n"),
      ("\\t", "\t"),
      ("\\r", "\r"),
      ("\\\\", "\\"),
      ("\\\"", "\""),
      ("\\#", "#"),
      ("\\u{e9}", "é"),
      ("\\u{1F600}", "😀"),
      ("\\u{41}", "A"),
    ][..],
  )
  .prop_map(|(raw, value)| (raw.to_string(), value.to_string()));

  prop_oneof![4 => plain, 1 => escape]
}

fn string_text() -> impl Strategy<Value = StringText> + Clone {
  vec(text_piece(), 1..6).prop_map(|pieces| {
    let (raw, value) = pieces.into_iter().unzip();

    StringText { span: sp(), raw, value }
  })
}

fn string_lit(
  interp: Option<BoxedStrategy<Expr>>,
) -> impl Strategy<Value = StringLit> + Clone {
  let part = match interp {
    Some(expr) => prop_oneof![
      2 => string_text().prop_map(StringPart::Text),
      1 => expr.prop_map(|e| StringPart::Interp(StringInterp {
        span: sp(),
        expr: Box::new(e),
      })),
    ]
    .boxed(),
    None => string_text().prop_map(StringPart::Text).boxed(),
  };

  vec(part, 0..4).prop_map(|parts| {
    let mut merged: Vec<StringPart> = Vec::new();

    for part in parts {
      match (merged.last_mut(), part) {
        (Some(StringPart::Text(prev)), StringPart::Text(next)) => {
          prev.raw.push_str(&next.raw);
          prev.value.push_str(&next.value);
        }
        (_, part) => merged.push(part),
      }
    }

    StringLit { span: sp(), parts: merged }
  })
}

fn plain_type_ref() -> impl Strategy<Value = TypeRef> + Clone {
  upper().prop_map(|name| TypeRef { span: sp(), name, args: vec![] })
}

fn effect_row(
  ty: BoxedStrategy<TypeExpr>,
) -> impl Strategy<Value = EffectRow> + Clone {
  let entry = (upper(), vec(ty, 0..2)).prop_map(|(name, args)| TypeRef {
    span: sp(),
    name,
    args,
  });

  (vec(entry, 0..3), option::of(lower()))
    .prop_map(|(entries, tail)| EffectRow { span: sp(), entries, tail })
}

pub fn type_expr() -> BoxedStrategy<TypeExpr> {
  let leaf = prop_oneof![
    plain_type_ref().prop_map(TypeExpr::Ref),
    lower().prop_map(|name| TypeExpr::Var(TypeVar { span: sp(), name })),
  ];

  leaf
    .prop_recursive(3, 16, 3, |inner| {
      let inner = inner.boxed();

      prop_oneof![
        (upper(), vec(inner.clone(), 1..3)).prop_map(|(name, args)| {
          TypeExpr::Ref(TypeRef { span: sp(), name, args })
        }),
        (
          vec(inner.clone(), 0..3),
          inner.clone(),
          option::of(effect_row(inner.clone()))
        )
          .prop_map(|(params, ret, effects)| {
            TypeExpr::Fn(FnType {
              span: sp(),
              params,
              ret: Box::new(ret),
              effects,
            })
          }),
        (vec((lower(), inner.clone()), 0..3), option::of(lower())).prop_map(
          |(fields, tail)| {
            TypeExpr::Record(RecordType {
              span: sp(),
              fields: fields
                .into_iter()
                .map(|(name, ty)| FieldType { span: sp(), name, ty })
                .collect(),
              tail,
            })
          }
        ),
      ]
    })
    .boxed()
}

fn signature()
-> impl Strategy<Value = (Option<TypeExpr>, Option<EffectRow>)> + Clone {
  option::of((type_expr(), option::of(effect_row(type_expr())))).prop_map(
    |sig| match sig {
      Some((ret, effects)) => (Some(ret), effects),
      None => (None, None),
    },
  )
}

fn params() -> impl Strategy<Value = Vec<Param>> + Clone {
  vec((lower(), option::of(type_expr())), 0..3).prop_map(|params| {
    params
      .into_iter()
      .map(|(name, ty)| Param { span: sp(), name, pattern: None, ty })
      .collect()
  })
}

fn pat_lit() -> impl Strategy<Value = PLit> + Clone {
  prop_oneof![
    (int_lit(), any::<bool>()).prop_map(|(lit, negative)| PLit {
      span: sp(),
      lit: PatLit::Int(lit),
      negative,
    }),
    (float_lit(), any::<bool>()).prop_map(|(lit, negative)| PLit {
      span: sp(),
      lit: PatLit::Float(lit),
      negative,
    }),
    bool_lit().prop_map(|lit| PLit {
      span: sp(),
      lit: PatLit::Bool(lit),
      negative: false,
    }),
    string_lit(None).prop_map(|lit| PLit {
      span: sp(),
      lit: PatLit::String(lit),
      negative: false,
    }),
  ]
}

pub fn pattern() -> BoxedStrategy<Pattern> {
  let leaf = prop_oneof![
    Just(Pattern::Wildcard(PWildcard { span: sp() })),
    lower().prop_map(|name| Pattern::Var(PVar { span: sp(), name })),
    pat_lit().prop_map(Pattern::Lit),
    upper().prop_map(|name| Pattern::Ctor(PCtor {
      span: sp(),
      name,
      args: vec![]
    })),
  ];

  leaf
    .prop_recursive(3, 12, 3, |inner| {
      prop_oneof![
        (upper(), vec(inner.clone(), 1..3)).prop_map(|(name, args)| {
          Pattern::Ctor(PCtor { span: sp(), name, args })
        }),
        (vec((lower(), inner), 0..3), any::<bool>()).prop_map(
          |(fields, open)| {
            Pattern::Record(PRecord {
              span: sp(),
              fields: fields
                .into_iter()
                .map(|(name, pattern)| PField { span: sp(), name, pattern })
                .collect(),
              open,
            })
          }
        ),
      ]
    })
    .boxed()
}

const BINARY_OPS: &[BinaryOp] = &[
  BinaryOp::Or,
  BinaryOp::And,
  BinaryOp::Eq,
  BinaryOp::NotEq,
  BinaryOp::Lt,
  BinaryOp::LtEq,
  BinaryOp::Gt,
  BinaryOp::GtEq,
  BinaryOp::Add,
  BinaryOp::Sub,
  BinaryOp::Mul,
  BinaryOp::Div,
  BinaryOp::Rem,
  BinaryOp::BitAnd,
  BinaryOp::BitOr,
  BinaryOp::BitXor,
];

fn leaf_expr() -> BoxedStrategy<Expr> {
  prop_oneof![
    int_lit().prop_map(Expr::Int),
    float_lit().prop_map(Expr::Float),
    bool_lit().prop_map(Expr::Bool),
    string_lit(None).prop_map(Expr::String),
    prop_oneof![3 => lower(), 1 => upper()]
      .prop_map(|name| Expr::Var(Var { span: sp(), name })),
  ]
  .boxed()
}

pub fn expr() -> BoxedStrategy<Expr> {
  expr_with(true, 4)
}

fn expr_with(stmts: bool, depth: u32) -> BoxedStrategy<Expr> {
  leaf_expr()
    .prop_recursive(depth, 40, 4, move |inner| {
      let inner = inner.boxed();
      let block = block(inner.clone(), stmts);
      let interp = if stmts { expr_with(false, 2) } else { inner.clone() };
      let binary = (select(BINARY_OPS), inner.clone(), inner.clone()).prop_map(
        |(op, left, right)| {
          Expr::Binary(Binary {
            span: sp(),
            op,
            op_span: sp(),
            left: Box::new(left),
            right: Box::new(right),
          })
        }
      );

      let nested = (
        select(BINARY_OPS),
        select(BINARY_OPS),
        inner.clone(),
        inner.clone(),
        inner.clone(),
        any::<bool>(),
      )
        .prop_map(|(outer, op, a, b, c, left)| {
          let binary = |op, left: Expr, right: Expr| {
            Expr::Binary(Binary {
              span: sp(),
              op,
              op_span: sp(),
              left: Box::new(left),
              right: Box::new(right),
            })
          };
          let pair = binary(op, a, b);

          if left { binary(outer, pair, c) } else { binary(outer, c, pair) }
        });

      prop_oneof![
        2 => nested,
        1 => (inner.clone(), lower()).prop_map(|(target, field)| {
          Expr::Field(FieldAccess { span: sp(), target: Box::new(target), field })
        }),
        1 => (inner.clone(), vec(inner.clone(), 0..3)).prop_map(|(callee, args)| {
          Expr::Call(Call { span: sp(), callee: Box::new(callee), args })
        }),
        1 => (inner.clone(), inner.clone()).prop_map(|(left, right)| {
          Expr::Pipe(Pipe {
            span: sp(),
            left: Box::new(left),
            right: Box::new(right),
          })
        }),
        3 => binary,
        1 => (select(&[UnaryOp::Negate, UnaryOp::Not, UnaryOp::BitNot][..]), inner.clone()).prop_map(
          |(op, operand)| {
            Expr::Unary(Unary { span: sp(), op, operand: Box::new(operand) })
          }
        ),
        1 => (option::of(inner.clone()), vec((lower(), inner.clone()), 0..3))
          .prop_map(|(spread, fields)| {
            Expr::Record(RecordLit {
              span: sp(),
              spread: spread.map(Box::new),
              fields: fields
                .into_iter()
                .map(|(name, value)| FieldInit { span: sp(), name, value })
                .collect(),
            })
          }),
        1 => (params(), signature(), block.clone()).prop_map(
          |(params, (return_type, effects), body)| {
            Expr::Lambda(Box::new(Lambda {
              span: sp(),
              params,
              return_type,
              effects,
              body: Box::new(body),
            }))
          }
        ),
        1 => block.clone().prop_map(Expr::Block),
        1 => if_expr(inner.clone(), block.clone()).prop_map(Expr::If),
        1 => (inner.clone(), vec((pattern(), inner.clone()), 0..3)).prop_map(
          |(scrutinee, arms)| {
            Expr::Match(Match {
              span: sp(),
              subjects: vec![scrutinee],
              arms: arms_of(arms),
            })
          }
        ),
        1 => string_lit(Some(interp)).prop_map(Expr::String),
        2 => raising(&inner, block.clone()),
      ]
    })
    .boxed()
}

fn arms_of(arms: Vec<(Pattern, Expr)>) -> Vec<MatchArm> {
  arms
    .into_iter()
    .map(|(pattern, body)| MatchArm {
      span: sp(),
      rows: vec![vec![pattern]],
      guard: None,
      body,
    })
    .collect()
}

fn raising(
  inner: &BoxedStrategy<Expr>,
  block: impl Strategy<Value = Block> + Clone + 'static,
) -> BoxedStrategy<Expr> {
  prop_oneof![
    1 => inner.clone().prop_map(|value| {
      Expr::Throw(Throw { span: sp(), value: Box::new(value) })
    }),
    1 => (block.clone(), vec((pattern(), inner.clone()), 1..3)).prop_map(
      |(body, arms)| {
        Expr::Try(Try {
          span: sp(),
          body: Box::new(body),
          catch_span: sp(),
          arms: arms_of(arms),
        })
      }
    ),
  ]
  .boxed()
}

fn block(
  expr: BoxedStrategy<Expr>,
  stmts: bool,
) -> impl Strategy<Value = Block> + Clone {
  let stmt = prop_oneof![
    (pattern(), option::of(type_expr()), expr.clone()).prop_map(
      |(pattern, ty, value)| {
        Stmt::Let(LetStmt { span: sp(), pattern, ty, value })
      }
    ),
    expr.clone().prop_map(|expr| Stmt::Expr(ExprStmt { span: sp(), expr })),
  ];
  let count = if stmts { 0..3 } else { 0..1 };

  (vec(stmt, count), expr).prop_map(|(stmts, result)| Block {
    span: sp(),
    stmts,
    result: Box::new(result),
  })
}

fn if_expr(
  expr: BoxedStrategy<Expr>,
  block: impl Strategy<Value = Block> + Clone + 'static,
) -> impl Strategy<Value = If> + Clone {
  let else_if = (expr.clone(), block.clone(), block.clone()).prop_map(
    |(cond, then_branch, else_branch)| {
      Else::If(If {
        span: sp(),
        cond: Box::new(cond),
        then_branch: Box::new(then_branch),
        else_branch: Some(Box::new(Else::Block(else_branch))),
      })
    },
  );
  let else_branch = prop_oneof![
    2 => block.clone().prop_map(Else::Block),
    1 => else_if,
  ];

  (expr, block, else_branch).prop_map(|(cond, then_branch, else_branch)| If {
    span: sp(),
    cond: Box::new(cond),
    then_branch: Box::new(then_branch),
    else_branch: Some(Box::new(else_branch)),
  })
}

fn method_list() -> impl Strategy<Value = Option<Vec<Name>>> + Clone {
  option::of(vec(lower(), 0..3))
}

fn import() -> impl Strategy<Value = Decl> {
  (vec(upper(), 1..4), option::of(upper()), method_list()).prop_map(
    |(path, alias, methods)| {
      Decl::Import(Import { span: sp(), path, alias, methods })
    },
  )
}

fn bound() -> impl Strategy<Value = Bound> + Clone {
  (upper(), lower()).prop_map(|(trait_name, var)| Bound {
    span: sp(),
    trait_name,
    var,
  })
}

fn method_sig() -> impl Strategy<Value = MethodSig> + Clone {
  let param = (lower(), type_expr()).prop_map(|(name, ty)| Param {
    span: sp(),
    name,
    pattern: None,
    ty: Some(ty),
  });

  (lower(), vec(param, 0..3), type_expr(), option::of(effect_row(type_expr())))
    .prop_map(|(name, params, return_type, effects)| MethodSig {
      span: sp(),
      docs: vec![],
      name,
      params,
      return_type,
      effects,
    })
}

fn recipe_case(case: &'static str) -> impl Strategy<Value = FnDecl> + Clone {
  (params(), signature(), block(expr(), true)).prop_map(
    move |(params, (return_type, effects), body)| FnDecl {
      span: sp(),
      docs: vec![],
      name: name(case),
      params,
      return_type,
      effects,
      bounds: vec![],
      body,
    },
  )
}

fn trait_decl() -> impl Strategy<Value = Decl> {
  let recipe = option::of((
    option::of(recipe_case("record")),
    option::of(recipe_case("variant")),
  ))
  .prop_map(|cases| {
    cases.map(|(record, variant)| Recipe {
      span: sp(),
      cases: record.into_iter().chain(variant).collect(),
    })
  });

  (upper(), lower(), vec(method_sig(), 1..4), recipe).prop_map(
    |(name, param, methods, recipe)| {
      Decl::Trait(TraitDecl {
        span: sp(),
        docs: vec![],
        name,
        param,
        methods,
        recipe,
      })
    },
  )
}

fn impl_decl() -> impl Strategy<Value = Decl> {
  (upper(), plain_type_ref(), vec(bound(), 0..3), vec(fn_item(), 1..3))
    .prop_map(|(trait_name, target, bounds, methods)| {
      Decl::Impl(ImplDecl {
        span: sp(),
        docs: vec![],
        trait_name,
        target,
        bounds,
        methods,
      })
    })
}

fn host_decl() -> impl Strategy<Value = Decl> {
  upper()
    .prop_map(|name| Decl::Host(HostDecl { span: sp(), docs: vec![], name }))
}

fn effect_decl() -> impl Strategy<Value = Decl> {
  (any::<bool>(), upper(), option::of(upper()), vec(method_sig(), 0..3))
    .prop_map(|(native, name, host, ops)| {
      Decl::Effect(EffectDecl {
        span: sp(),
        docs: vec![],
        native,
        name,
        host,
        ops,
      })
    })
}

fn extern_decl() -> impl Strategy<Value = Decl> {
  let param = (lower(), type_expr()).prop_map(|(name, ty)| Param {
    span: sp(),
    name,
    pattern: None,
    ty: Some(ty),
  });
  let module = string_text().prop_map(|text| StringLit {
    span: sp(),
    parts: vec![StringPart::Text(text)],
  });

  (
    lower(),
    vec(param, 0..3),
    type_expr(),
    option::of(effect_row(type_expr())),
    module,
    prop_oneof![lower(), upper()],
  )
    .prop_map(|(name, params, return_type, effects, module, export)| {
      Decl::Extern(ExternDecl {
        span: sp(),
        docs: vec![],
        name,
        params,
        return_type,
        effects,
        module,
        export,
      })
    })
}

fn bind_decl() -> impl Strategy<Value = Decl> {
  let module = option::of(string_text().prop_map(|text| StringLit {
    span: sp(),
    parts: vec![StringPart::Text(text)],
  }));

  (
    any::<bool>(),
    upper(),
    upper(),
    option::of(upper()),
    module,
    vec(fn_item(), 0..3),
  )
    .prop_map(|(force, effect, host, via, module, ops)| {
      let module = if via.is_some() { None } else { module };
      let ops =
        if via.is_some() || module.is_some() { Vec::new() } else { ops };

      Decl::Bind(BindDecl {
        span: sp(),
        docs: vec![],
        force,
        effect,
        host,
        from: via,
        module,
        ops,
      })
    })
}

fn type_decl() -> impl Strategy<Value = Decl> {
  let ctor = (upper(), vec(type_expr(), 0..3))
    .prop_map(|(name, args)| CtorDecl { span: sp(), name, args });
  let body = prop_oneof![
    type_expr().prop_map(TypeBody::Alias),
    vec(ctor, 1..4)
      .prop_map(|ctors| TypeBody::Variants(VariantBody { span: sp(), ctors })),
  ];
  let derive = option::of(vec(upper(), 0..3))
    .prop_map(|names| names.map(|names| Derive { span: sp(), names }));

  (upper(), vec(lower(), 0..3), body, derive).prop_map(
    |(name, params, body, derive)| {
      Decl::Type(TypeDecl {
        span: sp(),
        docs: vec![],
        name,
        params,
        body,
        derive,
      })
    },
  )
}

fn const_decl() -> impl Strategy<Value = Decl> {
  (lower(), option::of(type_expr()), expr()).prop_map(|(name, ty, value)| {
    Decl::Const(ConstDecl { span: sp(), docs: vec![], name, ty, value })
  })
}

fn fn_item() -> impl Strategy<Value = FnDecl> + Clone {
  (lower(), params(), signature(), vec(bound(), 0..3), block(expr(), true))
    .prop_map(|(name, params, (return_type, effects), bounds, body)| FnDecl {
      span: sp(),
      docs: vec![],
      name,
      params,
      return_type,
      effects,
      bounds,
      body,
    })
}

fn fn_decl() -> impl Strategy<Value = Decl> {
  fn_item().prop_map(Decl::Fn)
}

fn export() -> impl Strategy<Value = Decl> {
  prop_oneof![
    lower().prop_map(|name| Decl::Export(ExportDecl {
      span: sp(),
      name,
      methods: None,
    })),
    (upper(), vec(lower(), 0..3)).prop_map(|(name, methods)| {
      Decl::Export(ExportDecl { span: sp(), name, methods: Some(methods) })
    }),
  ]
}

fn zone(
  kind: ZoneKind,
  decl: BoxedStrategy<Decl>,
  count: std::ops::Range<usize>,
) -> impl Strategy<Value = Option<Zone>> {
  option::of(vec(decl, count))
    .prop_map(move |decls| decls.map(|decls| Zone { span: sp(), kind, decls }))
}

fn early_zones() -> impl Strategy<Value = Vec<Option<Zone>>> {
  (
    zone(ZoneKind::USES, import().boxed(), 0..3),
    zone(ZoneKind::HOSTS, host_decl().boxed(), 0..3),
    zone(ZoneKind::TRAITS, trait_decl().boxed(), 0..2),
    zone(ZoneKind::TYPES, type_decl().boxed(), 0..3),
    zone(ZoneKind::CONSTANTS, const_decl().boxed(), 0..3),
    zone(ZoneKind::EFFECTS, effect_decl().boxed(), 0..2),
  )
    .prop_map(|(a, b, c, d, e, f)| vec![a, b, c, d, e, f])
}

fn late_zones() -> impl Strategy<Value = Vec<Option<Zone>>> {
  (
    zone(ZoneKind::EXTERNS, extern_decl().boxed(), 0..2),
    zone(ZoneKind::BINDS, bind_decl().boxed(), 0..2),
    zone(ZoneKind::FUNCTIONS, fn_decl().boxed(), 1..3),
    zone(ZoneKind::IMPLS, impl_decl().boxed(), 0..2),
    zone(ZoneKind::EXPORTS, export().boxed(), 0..3),
  )
    .prop_map(|(a, b, c, d, e)| vec![a, b, c, d, e])
}

pub fn module() -> impl Strategy<Value = Module> {
  (option::of(upper()), early_zones(), late_zones()).prop_map(
    |(name, early, late)| Module {
      span: sp(),
      name,
      zones: early.into_iter().chain(late).flatten().collect(),
    },
  )
}
