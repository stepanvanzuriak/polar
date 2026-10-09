use polar_compiler::core::{
  dump::{dump_expr, dump_expr_shape, dump_module, dump_module_shape},
  ir::{
    CArm, CDecl, CExpr, CExprKind, CModule, CPattern, CtorId, CtorInfo,
    DeclKind, Lit, PatternTest, PrimOp, Sym, SymGen,
  },
};

fn e(kind: CExprKind) -> CExpr {
  CExpr::new(kind, None)
}

fn var(sym: &Sym) -> CExpr {
  e(CExprKind::Var(sym.clone()))
}

fn num(n: f64) -> CExpr {
  e(CExprKind::Lit(Lit::Number(n)))
}

fn shapes() -> Vec<CtorInfo> {
  vec![
    CtorInfo { name: "Circle".into(), arity: 1, owner: "Shape".into() },
    CtorInfo { name: "Square".into(), arity: 0, owner: "Shape".into() },
  ]
}

#[test]
fn dump_shows_symbol_ids() {
  let mut syms = SymGen::default();
  let _f = syms.fresh("f");
  let double = syms.fresh("double");
  let _a = syms.fresh("a");
  let _b = syms.fresh("b");
  let x = syms.fresh("x");

  let expr = e(CExprKind::Let {
    sym: Some(x.clone()),
    value: Box::new(e(CExprKind::App {
      func: Box::new(var(&double)),
      args: vec![num(21.0)],
    })),
    body: Box::new(var(&x)),
  });

  assert_eq!(
    dump_expr(&expr, &[]),
    "(let x/4 (app (var double/1) (lit 21)) (var x/4))"
  );
  assert_eq!(
    dump_expr_shape(&expr, &[]),
    "(let x (app (var double) (lit 21)) (var x))"
  );
}

#[test]
fn sym_gen_is_per_pass() {
  let mut first = SymGen::default();
  let mut second = SymGen::default();

  assert_eq!(first.fresh("x"), second.fresh("x"));
  assert_eq!(first.fresh("y").id, 1);
}

#[test]
fn same_spelling_gets_distinct_symbols() {
  let mut syms = SymGen::default();
  let outer = syms.fresh("x");
  let inner = syms.fresh("x");

  assert_ne!(outer, inner);
  assert_eq!(outer.name, inner.name);
}

#[test]
fn ctor_id_indexes_the_module() {
  let module = CModule {
    decls: vec![],
    ctors: shapes(),
    std_imports: vec![],
    user_imports: vec![],
    impls: vec![],
    binds: vec![],
    bridges: vec![],
    host: None,
    externs: vec![],
    bind_refs: vec![],
  };

  assert_eq!(module.ctor(CtorId(0)).name, "Circle");
  assert_eq!(module.ctor(CtorId(1)).name, "Square");
  assert_eq!(module.ctor(CtorId(1)).arity, 0);
}

#[test]
fn discarded_statement_has_no_sym() {
  let expr = e(CExprKind::Let {
    sym: None,
    value: Box::new(e(CExprKind::App {
      func: Box::new(e(CExprKind::Builtin { module: "Log", member: "info" })),
      args: vec![e(CExprKind::Lit(Lit::String("hi".into())))],
    })),
    body: Box::new(num(0.0)),
  });

  assert_eq!(
    dump_expr(&expr, &[]),
    r#"(let _ (app (builtin Log info) (lit "hi")) (lit 0))"#
  );
}

#[test]
fn slots_start_unset() {
  let expr = num(1.0);

  assert!(expr.ty.is_none());
  assert!(expr.effects.is_none());
}

#[test]
fn prim_arities() {
  assert_eq!(PrimOp::Neg.arity(), 1);
  assert_eq!(PrimOp::Not.arity(), 1);
  assert_eq!(PrimOp::Add.arity(), 2);
  assert_eq!(PrimOp::Eq.arity(), 2);
}

#[test]
fn every_kind_dumps() {
  let mut syms = SymGen::default();
  let s = syms.fresh("s");
  let r = syms.fresh("r");
  let n = syms.fresh("n");
  let p = syms.fresh("p");

  let case = e(CExprKind::Case {
    scrutinee: Box::new(var(&s)),
    arms: vec![
      arm(
        CPattern::Ctor {
          ctor: CtorId(0),
          args: vec![CPattern::Bind(r.clone())],
        },
        var(&r),
      ),
      arm(
        CPattern::Record {
          fields: vec![("a".into(), CPattern::Lit(Lit::Number(-1.0)))],
        },
        num(0.5),
      ),
      arm(CPattern::Wildcard, num(0.0)),
    ],
  });

  let flat = e(CExprKind::If {
    cond: Box::new(e(CExprKind::Test {
      test: PatternTest::Tag(CtorId(0)),
      target: Box::new(var(&s)),
    })),
    then_branch: Box::new(e(CExprKind::Field {
      target: Box::new(var(&s)),
      name: "_0".into(),
    })),
    else_branch: Box::new(e(CExprKind::If {
      cond: Box::new(e(CExprKind::Test {
        test: PatternTest::Lit(Lit::Bool(true)),
        target: Box::new(var(&s)),
      })),
      then_branch: Box::new(e(CExprKind::Concat {
        parts: vec![e(CExprKind::Lit(Lit::String("n = ".into()))), var(&n)],
      })),
      else_branch: Box::new(e(CExprKind::MatchFail)),
    })),
  });

  let values = e(CExprKind::App {
    func: Box::new(e(CExprKind::Lam {
      params: vec![p.clone()],
      body: Box::new(e(CExprKind::Prim {
        op: PrimOp::Neg,
        args: vec![var(&p)],
      })),
    })),
    args: vec![
      e(CExprKind::Record { fields: vec![("a".into(), num(1.0))] }),
      e(CExprKind::Update {
        base: Box::new(var(&p)),
        fields: vec![(
          "b".into(),
          e(CExprKind::Ctor { ctor: CtorId(1), args: vec![] }),
        )],
      }),
      e(CExprKind::CtorFn { ctor: CtorId(0) }),
    ],
  });

  let all = e(CExprKind::Let {
    sym: Some(n.clone()),
    value: Box::new(case),
    body: Box::new(e(CExprKind::Let {
      sym: None,
      value: Box::new(values),
      body: Box::new(flat),
    })),
  });

  assert_eq!(
    dump_expr_shape(&all, &shapes()),
    "(let n \
       (case (var s) \
         (arm (Circle r) (var r)) \
         (arm (record (a -1)) (lit 0.5)) \
         (arm _ (lit 0))) \
       (let _ \
         (app (lam (p) (prim Neg (var p))) \
           (record (a (lit 1))) \
           (update (var p) (b (ctor Square))) \
           (ctorfn Circle)) \
         (if (test (tag Circle) (var s)) \
           (field (var s) _0) \
           (if (test (lit true) (var s)) \
             (concat (lit \"n = \") (var n)) \
             (match-fail)))))"
      .split_whitespace()
      .collect::<Vec<_>>()
      .join(" ")
      .replace("( ", "(")
  );
}

fn module() -> CModule {
  let mut syms = SymGen::default();
  let main = syms.fresh("main");
  let area = syms.fresh("area");
  let shape = syms.fresh("shape");

  CModule {
    ctors: shapes(),
    std_imports: vec![],
    user_imports: vec![],
    impls: vec![],
    binds: vec![],
    bridges: vec![],
    host: None,
    externs: vec![],
    bind_refs: vec![],
    decls: vec![
      CDecl {
        kind: DeclKind::Function,
        sym: area.clone(),
        exported: false,
        params: vec![shape.clone()],
        body: var(&shape),
        origin: None,
        effects: None,
      },
      CDecl {
        kind: DeclKind::Function,
        sym: main,
        exported: true,
        params: vec![],
        body: e(CExprKind::App {
          func: Box::new(var(&area)),
          args: vec![num(1.0)],
        }),
        origin: None,
        effects: None,
      },
    ],
  }
}

#[test]
fn module_dump() {
  assert_eq!(
    dump_module(&module()),
    "\
(module
  (ctors (Circle 1 Shape) (Square 0 Shape))
  (fn area/1 (shape/2) (var shape/2))
  (fn main/0 export () (app (var area/1) (lit 1))))
"
  );
  assert!(!dump_module_shape(&module()).contains('/'));
}

#[test]
fn dump_is_stable() {
  assert_eq!(dump_module(&module()), dump_module(&module()));
}

fn arm(pattern: CPattern, body: CExpr) -> CArm {
  CArm { pattern, guard: None, body, origin: None }
}
