use crate::{shared::ice::invariant, shared::source::Span};

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Program {
  pub items: Vec<Item>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Item {
  ImportNamespace(ImportNamespace),
  ImportNamed(ImportNamed),
  Function(FunctionDecl),
  Stmt(Stmt),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ImportNamespace {
  pub local: Ident,
  pub source: String,
  pub origin: Option<Span>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ImportNamed {
  pub names: Vec<(String, Ident)>,
  pub source: String,
  pub origin: Option<Span>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FunctionDecl {
  pub name: Ident,
  pub params: Vec<Ident>,
  pub body: Vec<Stmt>,
  pub is_async: bool,
  pub exported: bool,
  pub origin: Option<Span>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VarKind {
  Const,
  Let,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VarDecl {
  pub kind: VarKind,
  pub name: Ident,
  pub init: Option<Expr>,
  pub exported: bool,
  pub origin: Option<Span>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
  Var(VarDecl),
  Expr(ExprStmt),
  Return(Return),
  If(If),
  Block(Block),
  Throw(Throw),
  Switch(Switch),
  Break(Break),
  Try(Try),
  While(While),
  Continue(Continue),
}

#[derive(Debug, Clone, PartialEq)]
pub struct While {
  pub test: Expr,
  pub body: Vec<Stmt>,
  pub origin: Option<Span>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Continue {
  pub origin: Option<Span>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Try {
  pub block: Vec<Stmt>,
  pub param: Ident,
  pub handler: Vec<Stmt>,
  pub origin: Option<Span>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExprStmt {
  pub expr: Expr,
  pub origin: Option<Span>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Return {
  pub arg: Option<Expr>,
  pub origin: Option<Span>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct If {
  pub test: Expr,
  pub consequent: Vec<Stmt>,
  pub alternate: Option<Vec<Stmt>>,
  pub origin: Option<Span>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Block {
  pub body: Vec<Stmt>,
  pub origin: Option<Span>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Throw {
  pub arg: Expr,
  pub origin: Option<Span>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Switch {
  pub discriminant: Expr,
  pub cases: Vec<SwitchCase>,
  pub default: Option<Vec<Stmt>>,
  pub origin: Option<Span>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SwitchCase {
  pub test: Literal,
  pub body: Vec<Stmt>,
  pub origin: Option<Span>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Break {
  pub origin: Option<Span>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
  Ident(Ident),
  Literal(Literal),
  TemplateLit(TemplateLit),
  ArrowFn(ArrowFn),
  Call(Call),
  Member(Member),
  ObjectLit(ObjectLit),
  ArrayLit(ArrayLit),
  Binary(Binary),
  Logical(Logical),
  Unary(Unary),
  Conditional(Conditional),
  Assign(Assign),
  Await(Await),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ident {
  pub name: String,
  pub original_name: Option<String>,
  pub origin: Option<Span>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LiteralValue {
  String(String),
  Number(f64),
  Bool(bool),
  Null,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Literal {
  pub value: LiteralValue,
  pub origin: Option<Span>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TemplateLit {
  pub quasis: Vec<String>,
  pub exprs: Vec<Expr>,
  pub origin: Option<Span>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ArrowFn {
  pub params: Vec<Ident>,
  pub body: ArrowBody,
  pub is_async: bool,
  pub origin: Option<Span>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ArrowBody {
  Expr(Box<Expr>),
  Block(Vec<Stmt>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Call {
  pub callee: Box<Expr>,
  pub args: Vec<Expr>,
  pub origin: Option<Span>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Member {
  pub object: Box<Expr>,
  pub property: MemberProp,
  pub origin: Option<Span>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum MemberProp {
  Static(PropName),
  Computed(Box<Expr>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PropName {
  pub name: String,
  pub origin: Option<Span>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ObjectLit {
  pub props: Vec<ObjectProp>,
  pub origin: Option<Span>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ObjectProp {
  KeyValue(Property),
  Spread(Spread),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Property {
  pub key: PropName,
  pub value: Expr,
  pub origin: Option<Span>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Spread {
  pub arg: Expr,
  pub origin: Option<Span>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ArrayLit {
  pub elements: Vec<Expr>,
  pub origin: Option<Span>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
  Add,
  Sub,
  Mul,
  Div,
  Mod,
  Lt,
  Le,
  Gt,
  Ge,
  StrictEq,
  StrictNe,
  BitAnd,
  BitOr,
  BitXor,
}

impl BinaryOp {
  #[must_use]
  pub fn as_str(self) -> &'static str {
    match self {
      Self::Add => "+",
      Self::Sub => "-",
      Self::Mul => "*",
      Self::Div => "/",
      Self::Mod => "%",
      Self::Lt => "<",
      Self::Le => "<=",
      Self::Gt => ">",
      Self::Ge => ">=",
      Self::StrictEq => "===",
      Self::StrictNe => "!==",
      Self::BitAnd => "&",
      Self::BitOr => "|",
      Self::BitXor => "^",
    }
  }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Binary {
  pub op: BinaryOp,
  pub left: Box<Expr>,
  pub right: Box<Expr>,
  pub origin: Option<Span>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogicalOp {
  And,
  Or,
}

impl LogicalOp {
  #[must_use]
  pub fn as_str(self) -> &'static str {
    match self {
      Self::And => "&&",
      Self::Or => "||",
    }
  }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Logical {
  pub op: LogicalOp,
  pub left: Box<Expr>,
  pub right: Box<Expr>,
  pub origin: Option<Span>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
  Negate,
  Not,
  BitNot,
}

impl UnaryOp {
  #[must_use]
  pub fn as_str(self) -> &'static str {
    match self {
      Self::Negate => "-",
      Self::Not => "!",
      Self::BitNot => "~",
    }
  }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Unary {
  pub op: UnaryOp,
  pub arg: Box<Expr>,
  pub origin: Option<Span>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Conditional {
  pub test: Box<Expr>,
  pub consequent: Box<Expr>,
  pub alternate: Box<Expr>,
  pub origin: Option<Span>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Assign {
  pub target: Ident,
  pub value: Box<Expr>,
  pub origin: Option<Span>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Await {
  pub arg: Box<Expr>,
  pub origin: Option<Span>,
}

impl Expr {
  #[must_use]
  pub fn origin(&self) -> Option<&Span> {
    match self {
      Self::Ident(n) => n.origin.as_ref(),
      Self::Literal(n) => n.origin.as_ref(),
      Self::TemplateLit(n) => n.origin.as_ref(),
      Self::ArrowFn(n) => n.origin.as_ref(),
      Self::Call(n) => n.origin.as_ref(),
      Self::Member(n) => n.origin.as_ref(),
      Self::ObjectLit(n) => n.origin.as_ref(),
      Self::ArrayLit(n) => n.origin.as_ref(),
      Self::Binary(n) => n.origin.as_ref(),
      Self::Logical(n) => n.origin.as_ref(),
      Self::Unary(n) => n.origin.as_ref(),
      Self::Conditional(n) => n.origin.as_ref(),
      Self::Assign(n) => n.origin.as_ref(),
      Self::Await(n) => n.origin.as_ref(),
    }
  }

  #[must_use]
  pub fn at(mut self, origin: Option<Span>) -> Self {
    let slot = match &mut self {
      Self::Ident(n) => &mut n.origin,
      Self::Literal(n) => &mut n.origin,
      Self::TemplateLit(n) => &mut n.origin,
      Self::ArrowFn(n) => &mut n.origin,
      Self::Call(n) => &mut n.origin,
      Self::Member(n) => &mut n.origin,
      Self::ObjectLit(n) => &mut n.origin,
      Self::ArrayLit(n) => &mut n.origin,
      Self::Binary(n) => &mut n.origin,
      Self::Logical(n) => &mut n.origin,
      Self::Unary(n) => &mut n.origin,
      Self::Conditional(n) => &mut n.origin,
      Self::Assign(n) => &mut n.origin,
      Self::Await(n) => &mut n.origin,
    };

    *slot = origin;
    self
  }
}

impl Stmt {
  #[must_use]
  pub fn origin(&self) -> Option<&Span> {
    match self {
      Self::Var(n) => n.origin.as_ref(),
      Self::Expr(n) => n.origin.as_ref(),
      Self::Return(n) => n.origin.as_ref(),
      Self::If(n) => n.origin.as_ref(),
      Self::Block(n) => n.origin.as_ref(),
      Self::Throw(n) => n.origin.as_ref(),
      Self::Switch(n) => n.origin.as_ref(),
      Self::Break(n) => n.origin.as_ref(),
      Self::Try(n) => n.origin.as_ref(),
      Self::While(n) => n.origin.as_ref(),
      Self::Continue(n) => n.origin.as_ref(),
    }
  }

  #[must_use]
  pub fn at(mut self, origin: Option<Span>) -> Self {
    let slot = match &mut self {
      Self::Var(n) => &mut n.origin,
      Self::Expr(n) => &mut n.origin,
      Self::Return(n) => &mut n.origin,
      Self::If(n) => &mut n.origin,
      Self::Block(n) => &mut n.origin,
      Self::Throw(n) => &mut n.origin,
      Self::Switch(n) => &mut n.origin,
      Self::Break(n) => &mut n.origin,
      Self::Try(n) => &mut n.origin,
      Self::While(n) => &mut n.origin,
      Self::Continue(n) => &mut n.origin,
    };

    *slot = origin;
    self
  }
}

impl Ident {
  #[must_use]
  pub fn new(name: impl Into<String>) -> Self {
    Self { name: name.into(), original_name: None, origin: None }
  }

  #[must_use]
  pub fn at(mut self, origin: Option<Span>) -> Self {
    self.origin = origin;
    self
  }
}

impl Literal {
  #[must_use]
  #[track_caller]
  pub fn number(n: f64) -> Self {
    invariant(n.is_finite(), || format!("non-finite number literal: {n}"));
    invariant(!(n == 0.0 && n.is_sign_negative()), || {
      "negative zero number literal".to_string()
    });
    invariant(n >= 0.0, || {
      format!("negative number literal {n}: use Unary(Negate, …)")
    });

    Self { value: LiteralValue::Number(n), origin: None }
  }
}

#[must_use]
pub fn ident(name: &str) -> Ident {
  Ident::new(name)
}

#[must_use]
pub fn ident_renamed(name: &str, original: &str) -> Ident {
  Ident { original_name: Some(original.to_string()), ..Ident::new(name) }
}

#[must_use]
pub fn var(name: &str) -> Expr {
  Expr::Ident(ident(name))
}

#[must_use]
#[track_caller]
pub fn number(n: f64) -> Expr {
  if n < 0.0 {
    unary(UnaryOp::Negate, Expr::Literal(Literal::number(-n)))
  } else {
    Expr::Literal(Literal::number(n))
  }
}

#[must_use]
pub fn string(s: &str) -> Expr {
  Expr::Literal(Literal {
    value: LiteralValue::String(s.to_string()),
    origin: None,
  })
}

#[must_use]
pub fn boolean(b: bool) -> Expr {
  Expr::Literal(Literal { value: LiteralValue::Bool(b), origin: None })
}

#[must_use]
pub fn null() -> Expr {
  Expr::Literal(Literal { value: LiteralValue::Null, origin: None })
}

#[must_use]
#[track_caller]
pub fn template(quasis: Vec<String>, exprs: Vec<Expr>) -> Expr {
  invariant(quasis.len() == exprs.len() + 1, || {
    format!(
      "template literal with {} quasis and {} expressions",
      quasis.len(),
      exprs.len()
    )
  });

  Expr::TemplateLit(TemplateLit { quasis, exprs, origin: None })
}

#[must_use]
pub fn call(callee: Expr, args: Vec<Expr>, origin: Option<Span>) -> Expr {
  Expr::Call(Call { callee: Box::new(callee), args, origin })
}

#[must_use]
pub fn member(object: Expr, name: &str) -> Expr {
  Expr::Member(Member {
    object: Box::new(object),
    property: MemberProp::Static(prop_name(name)),
    origin: None,
  })
}

#[must_use]
pub fn computed(object: Expr, property: Expr) -> Expr {
  Expr::Member(Member {
    object: Box::new(object),
    property: MemberProp::Computed(Box::new(property)),
    origin: None,
  })
}

#[must_use]
pub fn prop_name(name: &str) -> PropName {
  PropName { name: name.to_string(), origin: None }
}

#[must_use]
pub fn prop(key: &str, value: Expr) -> ObjectProp {
  ObjectProp::KeyValue(Property { key: prop_name(key), value, origin: None })
}

#[must_use]
pub fn spread(arg: Expr) -> ObjectProp {
  ObjectProp::Spread(Spread { arg, origin: None })
}

#[must_use]
pub fn object(props: Vec<ObjectProp>) -> Expr {
  Expr::ObjectLit(ObjectLit { props, origin: None })
}

#[must_use]
pub fn array(elements: Vec<Expr>) -> Expr {
  Expr::ArrayLit(ArrayLit { elements, origin: None })
}

#[must_use]
pub fn binary(op: BinaryOp, left: Expr, right: Expr) -> Expr {
  Expr::Binary(Binary {
    op,
    left: Box::new(left),
    right: Box::new(right),
    origin: None,
  })
}

#[must_use]
pub fn logical(op: LogicalOp, left: Expr, right: Expr) -> Expr {
  Expr::Logical(Logical {
    op,
    left: Box::new(left),
    right: Box::new(right),
    origin: None,
  })
}

#[must_use]
pub fn unary(op: UnaryOp, arg: Expr) -> Expr {
  Expr::Unary(Unary { op, arg: Box::new(arg), origin: None })
}

#[must_use]
pub fn conditional(test: Expr, consequent: Expr, alternate: Expr) -> Expr {
  Expr::Conditional(Conditional {
    test: Box::new(test),
    consequent: Box::new(consequent),
    alternate: Box::new(alternate),
    origin: None,
  })
}

#[must_use]
pub fn assign(target: Ident, value: Expr) -> Expr {
  Expr::Assign(Assign { target, value: Box::new(value), origin: None })
}

#[must_use]
pub fn await_(arg: Expr) -> Expr {
  Expr::Await(Await { arg: Box::new(arg), origin: None })
}

#[must_use]
pub fn arrow(params: Vec<Ident>, body: Expr, is_async: bool) -> Expr {
  Expr::ArrowFn(ArrowFn {
    params,
    body: ArrowBody::Expr(Box::new(body)),
    is_async,
    origin: None,
  })
}

#[must_use]
pub fn arrow_block(
  params: Vec<Ident>,
  body: Vec<Stmt>,
  is_async: bool,
) -> Expr {
  Expr::ArrowFn(ArrowFn {
    params,
    body: ArrowBody::Block(body),
    is_async,
    origin: None,
  })
}

#[must_use]
pub fn expr_stmt(expr: Expr) -> Stmt {
  Stmt::Expr(ExprStmt { expr, origin: None })
}

#[must_use]
pub fn ret(arg: Expr) -> Stmt {
  Stmt::Return(Return { arg: Some(arg), origin: None })
}

#[must_use]
pub fn const_(name: Ident, init: Expr) -> Stmt {
  Stmt::Var(VarDecl {
    kind: VarKind::Const,
    name,
    init: Some(init),
    exported: false,
    origin: None,
  })
}

#[must_use]
pub fn let_(name: Ident) -> Stmt {
  Stmt::Var(VarDecl {
    kind: VarKind::Let,
    name,
    init: None,
    exported: false,
    origin: None,
  })
}

#[must_use]
pub fn if_(
  test: Expr,
  consequent: Vec<Stmt>,
  alternate: Option<Vec<Stmt>>,
) -> Stmt {
  Stmt::If(If { test, consequent, alternate, origin: None })
}

#[must_use]
pub fn block(body: Vec<Stmt>) -> Stmt {
  Stmt::Block(Block { body, origin: None })
}

#[must_use]
pub fn throw(arg: Expr) -> Stmt {
  Stmt::Throw(Throw { arg, origin: None })
}

#[must_use]
pub fn try_(block: Vec<Stmt>, param: Ident, handler: Vec<Stmt>) -> Stmt {
  Stmt::Try(Try { block, param, handler, origin: None })
}

#[must_use]
pub fn break_() -> Stmt {
  Stmt::Break(Break { origin: None })
}

#[must_use]
pub fn while_(test: Expr, body: Vec<Stmt>) -> Stmt {
  Stmt::While(While { test, body, origin: None })
}

#[must_use]
pub fn continue_() -> Stmt {
  Stmt::Continue(Continue { origin: None })
}

#[must_use]
pub fn function(
  name: Ident,
  params: Vec<Ident>,
  body: Vec<Stmt>,
  is_async: bool,
) -> FunctionDecl {
  FunctionDecl { name, params, body, is_async, exported: false, origin: None }
}
