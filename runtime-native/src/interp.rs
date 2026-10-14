use crate::{
  bindings, builtins,
  value::{Closure, CtorFnData, Env, Frame, Native, Value, show},
};
use polar_compiler::{
  backend::codegen::emit::dict_name,
  core::{
    dictionaries::{inlined, prelude_primitive},
    ir::{
      CDecl, CExpr, CExprKind, CExtern, CModule, DeclKind, Lit, PatternTest,
      PrimOp,
    },
    serial::Bundle,
  },
  shared::source::SourceFile,
};
use std::{collections::HashMap, io::Write, rc::Rc};

#[derive(Debug, Clone)]
pub enum Control {
  Throw { tag: String, value: Value },
  Return(Value),
  MatchFail(String),
  Arithmetic(String),
  Fail(String),
}

impl Control {
  #[must_use]
  pub fn fail(message: impl Into<String>) -> Self {
    Self::Fail(message.into())
  }

  #[must_use]
  pub fn throw(tag: impl Into<String>, value: Value) -> Self {
    Self::Throw { tag: tag.into(), value }
  }
}

pub type R = Result<Value, Control>;

enum Slot {
  Empty,
  Running,
  Done(Value),
}

struct ModRt {
  ir: &'static CModule,
  by_id: HashMap<u32, usize>,
  by_name: HashMap<&'static str, usize>,
  cache: Vec<Slot>,
  binds: HashMap<&'static str, HashMap<&'static str, &'static CDecl>>,
  externs: HashMap<&'static str, &'static CExtern>,
  source: Option<SourceFile>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunOutcome {
  pub exit_code: i32,
}

pub struct Interp {
  mods: Vec<ModRt>,
  paths: HashMap<String, usize>,
  main: usize,
  pub args: Vec<String>,
  pub exit_code: Option<i32>,
  pub env_overlay: Vec<(String, Option<String>)>,
  pub out: Box<dyn Write>,
  pub err: Box<dyn Write>,
}

fn rt_of(ir: &'static CModule) -> ModRt {
  let mut by_id = HashMap::new();
  let mut by_name = HashMap::new();

  for (i, d) in ir.decls.iter().enumerate() {
    by_id.insert(d.sym.id, i);
    by_name.insert(&*d.sym.name, i);
  }

  let mut binds: HashMap<&'static str, HashMap<&'static str, &'static CDecl>> =
    HashMap::new();

  for bind in &ir.binds {
    let ops = binds.entry(bind.effect.as_str()).or_default();

    for op in &bind.ops {
      ops.insert(&*op.sym.name, op);
    }
  }

  ModRt {
    ir,
    by_id,
    by_name,
    cache: ir.decls.iter().map(|_| Slot::Empty).collect(),
    binds,
    externs: ir.externs.iter().map(|e| (e.name.as_str(), e)).collect(),
    source: ir
      .source
      .as_ref()
      .map(|(n, t)| SourceFile::new(n.clone(), t.clone())),
  }
}

impl Interp {
  #[must_use]
  pub fn new(bundle: Bundle, args: Vec<String>) -> Self {
    let bundle: &'static Bundle = Box::leak(Box::new(bundle));
    let mut mods = vec![rt_of(&bundle.main)];
    let mut paths = HashMap::new();

    for (path, module) in &bundle.modules {
      paths.insert(path.clone(), mods.len());
      mods.push(rt_of(module));
    }

    Self {
      mods,
      paths,
      main: 0,
      args,
      exit_code: None,
      env_overlay: Vec::new(),
      out: Box::new(std::io::stdout()),
      err: Box::new(std::io::stderr()),
    }
  }

  pub fn run_main(&mut self) -> RunOutcome {
    let result = self.call_global(self.main, "main", Vec::new());
    let _ = self.out.flush();
    let mut code = self.exit_code.unwrap_or(0);

    if let Err(control) = result {
      code = 1;
      self.report(&control);
    }

    let _ = self.out.flush();

    RunOutcome { exit_code: code }
  }

  fn report(&mut self, control: &Control) {
    let message = match control {
      Control::Throw { value, .. } => {
        format!("uncaught error: {}", show(value))
      }
      Control::MatchFail(m) => format!("error: uncaught PolarMatchError: {m}"),
      Control::Arithmetic(m) => {
        format!("error: uncaught PolarArithmeticError: {m}")
      }
      Control::Fail(m) => format!("error: {m}"),
      Control::Return(_) => "error: stray return".to_string(),
    };

    let _ = writeln!(self.err, "{message}");
  }

  #[must_use]
  pub fn env_var(&self, name: &str) -> Option<String> {
    match self.env_overlay.iter().rev().find(|(k, _)| k == name) {
      Some((_, value)) => value.clone(),
      None => std::env::var(name).ok(),
    }
  }

  pub(crate) fn module_ir(&self, m: usize) -> &'static CModule {
    self.mods[m].ir
  }

  pub(crate) fn main_module(&self) -> usize {
    self.main
  }

  pub(crate) fn eval_in(&mut self, e: &'static CExpr, m: usize) -> R {
    self.eval_with(e, None, m, false)
  }

  pub(crate) fn local_op(&mut self, m: usize, effect: &str, op: &str) -> R {
    let Some(decl) =
      self.mods[m].binds.get(effect).and_then(|ops| ops.get(op)).copied()
    else {
      return Err(Control::fail(format!("`{effect}` has no operation `{op}`")));
    };

    Ok(Value::Closure(Rc::new(Closure {
      params: &decl.params,
      body: &decl.body,
      env: None,
      module: m,
    })))
  }

  pub(crate) fn report_control(&mut self, control: &Control) {
    self.report(control);
  }

  fn call_global(&mut self, m: usize, name: &str, args: Vec<Value>) -> R {
    let Some(&idx) = self.mods[m].by_name.get(name) else {
      return Err(Control::fail(format!(
        "this program does not export `{name}`"
      )));
    };
    let f = self.global(m, idx)?;

    self.apply(&f, args)
  }

  fn global(&mut self, m: usize, idx: usize) -> R {
    match &self.mods[m].cache[idx] {
      Slot::Done(v) => return Ok(v.clone()),
      Slot::Running => {
        return Err(Control::fail(format!(
          "`{}` depends on itself while it is being computed",
          self.mods[m].ir.decls[idx].sym.name
        )));
      }
      Slot::Empty => {}
    }

    let decl: &'static CDecl = &self.mods[m].ir.decls[idx];
    let value = match decl.kind {
      DeclKind::Function => Value::Closure(Rc::new(Closure {
        params: &decl.params,
        body: &decl.body,
        env: None,
        module: m,
      })),
      DeclKind::Constant => {
        self.mods[m].cache[idx] = Slot::Running;
        self.eval_with(&decl.body, None, m, false).inspect_err(|_| {
          self.mods[m].cache[idx] = Slot::Empty;
        })?
      }
    };

    self.mods[m].cache[idx] = Slot::Done(value.clone());

    Ok(value)
  }

  fn module_path(&self, path: &str) -> Result<usize, Control> {
    self.paths.get(path).copied().ok_or_else(|| {
      Control::fail(format!("module `{path}` is not in this program"))
    })
  }

  fn member(&mut self, path: &str, name: &str) -> R {
    let m = self.module_path(path)?;
    let Some(&idx) = self.mods[m].by_name.get(name) else {
      return Err(Control::fail(format!("`{path}` has no `{name}`")));
    };

    self.global(m, idx)
  }

  fn op(&mut self, m: usize, effect: &str, op: &str) -> R {
    let host = self.mods[m].ir.host.as_deref();

    if self.mods[m]
      .ir
      .bridges
      .iter()
      .any(|b| b.effect == effect && Some(b.host.as_str()) == host)
    {
      return Ok(Value::Bridge(Rc::new(crate::value::BridgeCall {
        module: m,
        effect: effect.to_string(),
        op: op.to_string(),
      })));
    }

    let owner = self.mods[m]
      .ir
      .bind_refs
      .iter()
      .find(|(e, _)| e == effect)
      .and_then(|(_, path)| path.clone());
    let target = match owner {
      Some(path) => self.module_path(&path)?,
      None => m,
    };
    let Some(decl) =
      self.mods[target].binds.get(effect).and_then(|ops| ops.get(op)).copied()
    else {
      return Err(Control::fail(format!(
        "`{effect}` has no binding for `{op}` on the native target"
      )));
    };

    Ok(Value::Closure(Rc::new(Closure {
      params: &decl.params,
      body: &decl.body,
      env: None,
      module: target,
    })))
  }

  fn extern_value(&mut self, m: usize, name: &str) -> R {
    let Some(ext) = self.mods[m].externs.get(name).copied() else {
      return Err(Control::fail(format!("unknown extern `{name}`")));
    };
    let f = ext.rs.as_deref().and_then(bindings::lookup);

    Ok(Value::Native(Rc::new(Native { name: ext.label.clone(), f })))
  }

  pub fn apply(&mut self, f: &Value, args: Vec<Value>) -> R {
    match f {
      Value::Closure(c) => {
        let env = bind_params(c, args)?;

        self.eval_with(c.body, env, c.module, true)
      }
      Value::Native(n) => match n.f {
        Some(f) => f(self, &args),
        None => Err(Control::fail(format!(
          "`{}` is not supported on the native target",
          n.name
        ))),
      },
      Value::CtorFn(c) => Ok(Value::variant(c.name, args)),
      Value::Bridge(call) => {
        let (m, effect, op) =
          (call.module, call.effect.clone(), call.op.clone());

        crate::bridge::call(self, m, &effect, &op, &args)
      }
      other => Err(Control::fail(format!("{} is not a function", show(other)))),
    }
  }

  fn lookup(&mut self, env: &Env, id: u32, m: usize) -> R {
    let mut at = env;

    while let Some(frame) = at {
      if frame.id == id {
        return Ok(frame.value.clone());
      }

      at = &frame.parent;
    }

    match self.mods[m].by_id.get(&id).copied() {
      Some(idx) => self.global(m, idx),
      None => Err(Control::fail(format!("unbound symbol {id}"))),
    }
  }

  fn match_failure(&self, e: &CExpr, m: usize) -> Control {
    Control::MatchFail(format!("no pattern matched ({})", self.location(e, m)))
  }

  pub fn eval_with(
    &mut self,
    e: &'static CExpr,
    env: Env,
    m: usize,
    in_function: bool,
  ) -> R {
    let mut boundary = in_function;
    let result = self.eval_loop(e, env, m, &mut boundary);

    match result {
      Err(Control::Return(v)) if boundary => Ok(v),
      other => other,
    }
  }

  fn eval(&mut self, e: &'static CExpr, env: &Env, m: usize) -> R {
    self.eval_with(e, env.clone(), m, false)
  }

  #[allow(clippy::too_many_lines, reason = "one arm per Core expression kind")]
  fn eval_loop(
    &mut self,
    mut e: &'static CExpr,
    mut env: Env,
    mut m: usize,
    boundary: &mut bool,
  ) -> R {
    loop {
      match &e.kind {
        CExprKind::Lit(lit) => return Ok(lit_value(lit)),
        CExprKind::Var(sym) => return self.lookup(&env, sym.id, m),
        CExprKind::Builtin { module, member } => {
          return builtins::lookup(module, member).map_or_else(
            || {
              Err(Control::fail(format!(
                "`{module}.{member}` is not available on the native target"
              )))
            },
            |f| {
              Ok(Value::Native(Rc::new(Native {
                name: format!("{module}.{member}"),
                f: Some(f),
              })))
            },
          );
        }
        CExprKind::Std { module, member } => {
          return self.member(&format!("Std.{module}"), member);
        }
        CExprKind::User { module, member } => {
          return self.member(module, member);
        }
        CExprKind::Method { .. } | CExprKind::Case { .. } => {
          return Err(Control::fail("unexpected unelaborated IR"));
        }
        CExprKind::Dict { trait_path, target, module, args } => {
          let name = dict_name(trait_path, target, module.as_deref());
          let dict = match module.as_deref() {
            None => {
              let Some(&idx) = self.mods[m].by_name.get(name.as_str()) else {
                return Err(Control::fail(format!("no dictionary `{name}`")));
              };

              self.global(m, idx)?
            }
            Some(path) => self.member(path, &name)?,
          };

          if args.is_empty() {
            return Ok(dict);
          }

          let mut values = Vec::with_capacity(args.len());

          for a in args {
            values.push(self.eval(a, &env, m)?);
          }

          return self.apply(&dict, values);
        }
        CExprKind::Lam { params, body } => {
          return Ok(Value::Closure(Rc::new(Closure {
            params,
            body,
            env: env.clone(),
            module: m,
          })));
        }
        CExprKind::App { func, args } if inlined(e) => {
          let CExprKind::Field { target, name } = &func.kind else {
            return Err(Control::fail("inlined call without a dictionary"));
          };
          let mut values = Vec::with_capacity(args.len());

          for a in args {
            values.push(self.eval(a, &env, m)?);
          }

          return Ok(match (name.as_str(), prelude_primitive(target, name)) {
            ("eq", _) => Value::Bool(values[0].strict_eq(&values[1])),
            (_, Some("String")) => values.swap_remove(0),
            (_, Some("Float")) => Value::str(crate::value::float_to_string(
              values[0].as_num().unwrap_or(f64::NAN),
            )),
            _ => Value::str(values[0].coerce_string()),
          });
        }
        CExprKind::App { func, args } => {
          let f = self.eval(func, &env, m)?;
          let mut values = Vec::with_capacity(args.len());

          for a in args {
            values.push(self.eval(a, &env, m)?);
          }

          match &f {
            Value::Closure(c) => {
              env = bind_params(c, values)?;
              e = c.body;
              m = c.module;
              *boundary = true;
            }
            other => return self.apply(other, values),
          }
        }
        CExprKind::Prim { op, args } => {
          return self.prim(*op, args, &env, m, e);
        }
        CExprKind::Let { sym, value, body } => {
          let v = self.eval(value, &env, m)?;

          if let Some(sym) = sym {
            env = Some(Rc::new(Frame { id: sym.id, value: v, parent: env }));
          }

          e = body;
        }
        CExprKind::If { cond, then_branch, else_branch } => {
          let c = self.eval(cond, &env, m)?;

          e = if c.as_bool() == Some(true) { then_branch } else { else_branch };
        }
        CExprKind::Record { fields } => {
          let mut out = Vec::with_capacity(fields.len());

          for (k, v) in fields {
            out.push((k.as_str(), self.eval(v, &env, m)?));
          }

          return Ok(Value::record(out));
        }
        CExprKind::Update { base, fields } => {
          let base = self.eval(base, &env, m)?;
          let Value::Record(existing) = base else {
            return Err(Control::fail("update of a non-record"));
          };
          let mut out: Vec<(&'static str, Value)> = (*existing).clone();

          for (k, v) in fields {
            let value = self.eval(v, &env, m)?;

            match out.iter_mut().find(|(name, _)| *name == k.as_str()) {
              Some(slot) => slot.1 = value,
              None => out.push((k.as_str(), value)),
            }
          }

          return Ok(Value::record(out));
        }
        CExprKind::Field { target, name } => {
          let t = self.eval(target, &env, m)?;

          if let Value::Variant(v) = &t {
            if name == "$" {
              return Ok(Value::str(v.name));
            }

            if let Some(i) =
              name.strip_prefix('_').and_then(|n| n.parse::<usize>().ok())
              && let Some(field) = v.fields.get(i)
            {
              return Ok(field.clone());
            }
          }

          return t.field(name).cloned().ok_or_else(|| {
            Control::fail(format!("no field `{name}` on {}", show(&t)))
          });
        }
        CExprKind::Ctor { ctor, args } => {
          let name = self.ctor_name(m, ctor.0);
          let mut values = Vec::with_capacity(args.len());

          for a in args {
            values.push(self.eval(a, &env, m)?);
          }

          return Ok(Value::variant(name, values));
        }
        CExprKind::CtorFn { ctor } => {
          let info = &self.mods[m].ir.ctors[ctor.0 as usize];

          return Ok(Value::CtorFn(Rc::new(CtorFnData {
            name: &info.name,
            arity: info.arity,
          })));
        }
        CExprKind::Concat { parts } => {
          let mut out = String::new();

          for part in parts {
            match &part.kind {
              CExprKind::Lit(Lit::String(s)) => out.push_str(s),
              _ => out.push_str(&self.eval(part, &env, m)?.coerce_string()),
            }
          }

          return Ok(Value::str(out));
        }
        CExprKind::Test { test, target } => {
          let t = self.eval(target, &env, m)?;

          return Ok(Value::Bool(match test {
            PatternTest::Tag(ctor) => {
              matches!(&t, Value::Variant(v)
                if v.name == self.mods[m].ir.ctors[ctor.0 as usize].name)
            }
            PatternTest::Lit(lit) => t.strict_eq(&lit_value(lit)),
          }));
        }
        CExprKind::MatchFail => return Err(self.match_failure(e, m)),
        CExprKind::Op { effect, op } => return self.op(m, effect, op),
        CExprKind::Extern { name } => return self.extern_value(m, name),
        CExprKind::Throw { value, tag } => {
          let v = self.eval(value, &env, m)?;

          return Err(Control::throw(tag.clone(), v));
        }
        CExprKind::Return { value } => {
          let v = self.eval(value, &env, m)?;

          return Err(Control::Return(v));
        }
        CExprKind::Try { body, caught, handler, handles } => {
          match self.eval(body, &env, m) {
            Err(Control::Throw { tag, value }) if handles.contains(&tag) => {
              env = Some(Rc::new(Frame { id: caught.id, value, parent: env }));
              e = handler;
            }
            other => return other,
          }
        }
      }
    }
  }

  fn int_arithmetic(
    &self,
    op: PrimOp,
    a: f64,
    b: f64,
    at: &CExpr,
    m: usize,
  ) -> R {
    const MAX_SAFE: f64 = 9_007_199_254_740_991.0;

    let result = match op {
      PrimOp::IntAdd => a + b,
      PrimOp::IntSub => a - b,
      PrimOp::IntMul => a * b,
      _ => {
        if b == 0.0 {
          return Err(Control::Arithmetic(format!(
            "division by zero ({})",
            self.location(at, m)
          )));
        }

        if op == PrimOp::IntDiv { (a - a % b) / b } else { a % b }
      }
    };

    if !result.is_finite() || result.abs() > MAX_SAFE {
      return Err(Control::Arithmetic("integer overflow".to_string()));
    }

    Ok(Value::Num(result + 0.0))
  }

  fn location(&self, e: &CExpr, m: usize) -> String {
    match (&self.mods[m].source, &e.origin) {
      (Some(file), Some(span)) => {
        let p = file.position_at(span.start);

        format!("{}:{}:{}", file.name(), p.line + 1, file.code_point_column(p))
      }
      _ => "unknown location".to_string(),
    }
  }

  fn ctor_name(&self, m: usize, id: u32) -> &'static str {
    &self.mods[m].ir.ctors[id as usize].name
  }

  fn prim(
    &mut self,
    op: PrimOp,
    args: &'static [CExpr],
    env: &Env,
    m: usize,
    at: &'static CExpr,
  ) -> R {
    if let (PrimOp::And | PrimOp::Or, [left, right]) = (op, args) {
      let l = self.eval(left, env, m)?;
      let short = l.as_bool() == Some(op == PrimOp::Or);

      return if short { Ok(l) } else { self.eval(right, env, m) };
    }

    let mut values = Vec::with_capacity(args.len());

    for a in args {
      values.push(self.eval(a, env, m)?);
    }

    let num = |i: usize| values[i].as_num();

    if matches!(
      op,
      PrimOp::IntAdd
        | PrimOp::IntSub
        | PrimOp::IntMul
        | PrimOp::IntDiv
        | PrimOp::IntMod
    ) {
      let (a, b) = (num(0).unwrap_or(f64::NAN), num(1).unwrap_or(f64::NAN));

      return self.int_arithmetic(op, a, b, at, m);
    }

    Ok(match op {
      PrimOp::Neg => Value::Num(-num(0).unwrap_or(f64::NAN)),
      PrimOp::Not => Value::Bool(values[0].as_bool() != Some(true)),
      PrimOp::BitNot => Value::Num(f64::from(!to_i32(num(0)))),
      PrimOp::Eq => Value::Bool(values[0].strict_eq(&values[1])),
      PrimOp::Ne => Value::Bool(!values[0].strict_eq(&values[1])),
      PrimOp::Add => match (&values[0], &values[1]) {
        (Value::Str(a), b) => Value::str(format!("{a}{}", b.coerce_string())),
        (a, Value::Str(b)) => Value::str(format!("{}{b}", a.coerce_string())),
        _ => {
          Value::Num(num(0).unwrap_or(f64::NAN) + num(1).unwrap_or(f64::NAN))
        }
      },
      PrimOp::Lt | PrimOp::Le | PrimOp::Gt | PrimOp::Ge => {
        let ord = match (&values[0], &values[1]) {
          (Value::Str(a), Value::Str(b)) => {
            let a: Vec<u16> = a.encode_utf16().collect();
            let b: Vec<u16> = b.encode_utf16().collect();

            Some(a.cmp(&b))
          }
          (a, b) => match (a.as_num(), b.as_num()) {
            (Some(a), Some(b)) => a.partial_cmp(&b),
            _ => None,
          },
        };

        Value::Bool(ord.is_some_and(|o| match op {
          PrimOp::Lt => o.is_lt(),
          PrimOp::Le => o.is_le(),
          PrimOp::Gt => o.is_gt(),
          _ => o.is_ge(),
        }))
      }
      PrimOp::Sub
      | PrimOp::Mul
      | PrimOp::Div
      | PrimOp::Mod
      | PrimOp::BitAnd
      | PrimOp::BitOr
      | PrimOp::BitXor => {
        let (a, b) = (num(0).unwrap_or(f64::NAN), num(1).unwrap_or(f64::NAN));

        Value::Num(match op {
          PrimOp::Sub => a - b,
          PrimOp::Mul => a * b,
          PrimOp::Div => a / b,
          PrimOp::Mod => a % b,
          PrimOp::BitAnd => f64::from(to_i32(Some(a)) & to_i32(Some(b))),
          PrimOp::BitOr => f64::from(to_i32(Some(a)) | to_i32(Some(b))),
          _ => f64::from(to_i32(Some(a)) ^ to_i32(Some(b))),
        })
      }
      PrimOp::And
      | PrimOp::Or
      | PrimOp::IntAdd
      | PrimOp::IntSub
      | PrimOp::IntMul
      | PrimOp::IntDiv
      | PrimOp::IntMod => unreachable!("handled above"),
    })
  }
}

fn to_i32(n: Option<f64>) -> i32 {
  let n = n.unwrap_or(0.0);

  if !n.is_finite() {
    return 0;
  }

  let wrapped = n.trunc().rem_euclid(4_294_967_296.0);

  #[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap
  )]
  {
    (wrapped as u32) as i32
  }
}

fn bind_params(c: &Closure, args: Vec<Value>) -> Result<Env, Control> {
  if args.len() < c.params.len() {
    return Err(Control::fail("a function was called with too few arguments"));
  }

  let mut env = c.env.clone();

  for (param, value) in c.params.iter().zip(args) {
    env = Some(Rc::new(Frame { id: param.id, value, parent: env }));
  }

  Ok(env)
}

fn lit_value(lit: &Lit) -> Value {
  match lit {
    Lit::Number(n) => Value::Num(*n),
    Lit::String(s) => Value::str(s),
    Lit::Bool(b) => Value::Bool(*b),
  }
}
