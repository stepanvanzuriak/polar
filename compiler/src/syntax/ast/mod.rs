pub mod dump;
pub mod eq;
pub mod fields;

use crate::shared::source::Span;
use fields::{AsField, AsNode, Field, Fields, ast};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Name {
  pub text: String,
  pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Builtin {
  Uses,
  Hosts,
  Traits,
  Types,
  Constants,
  Effects,
  Externs,
  Binds,
  Functions,
  Impls,
  Exports,
}

impl Builtin {
  pub const ALL: [Builtin; 11] = [
    Self::Uses,
    Self::Hosts,
    Self::Traits,
    Self::Types,
    Self::Constants,
    Self::Effects,
    Self::Externs,
    Self::Binds,
    Self::Functions,
    Self::Impls,
    Self::Exports,
  ];

  #[must_use]
  pub fn as_str(self) -> &'static str {
    match self {
      Self::Uses => "uses",
      Self::Hosts => "hosts",
      Self::Traits => "traits",
      Self::Types => "types",
      Self::Constants => "constants",
      Self::Effects => "effects",
      Self::Externs => "externs",
      Self::Binds => "binds",
      Self::Functions => "functions",
      Self::Impls => "impls",
      Self::Exports => "exports",
    }
  }

  #[must_use]
  pub fn index(self) -> u8 {
    self as u8
  }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PluginId(pub u16);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ZoneKind {
  Builtin(Builtin),
  Plugin(PluginId),
}

impl ZoneKind {
  pub const USES: Self = Self::Builtin(Builtin::Uses);
  pub const HOSTS: Self = Self::Builtin(Builtin::Hosts);
  pub const TRAITS: Self = Self::Builtin(Builtin::Traits);
  pub const TYPES: Self = Self::Builtin(Builtin::Types);
  pub const CONSTANTS: Self = Self::Builtin(Builtin::Constants);
  pub const EFFECTS: Self = Self::Builtin(Builtin::Effects);
  pub const EXTERNS: Self = Self::Builtin(Builtin::Externs);
  pub const BINDS: Self = Self::Builtin(Builtin::Binds);
  pub const FUNCTIONS: Self = Self::Builtin(Builtin::Functions);
  pub const IMPLS: Self = Self::Builtin(Builtin::Impls);
  pub const EXPORTS: Self = Self::Builtin(Builtin::Exports);

  #[must_use]
  pub fn rank(self) -> (u8, u16) {
    match self {
      Self::Builtin(builtin) => (builtin.index(), 0),
      Self::Plugin(id) => {
        (crate::syntax::plugins::after(id).index(), id.0.saturating_add(1))
      }
    }
  }

  #[must_use]
  pub fn as_str(self) -> &'static str {
    match self {
      Self::Builtin(builtin) => builtin.as_str(),
      Self::Plugin(id) => crate::syntax::plugins::keyword(id),
    }
  }

  #[must_use]
  pub fn builtin(self) -> Option<Builtin> {
    match self {
      Self::Builtin(builtin) => Some(builtin),
      Self::Plugin(_) => None,
    }
  }
}

impl PartialOrd for ZoneKind {
  fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
    Some(self.cmp(other))
  }
}

impl Ord for ZoneKind {
  fn cmp(&self, other: &Self) -> std::cmp::Ordering {
    self.rank().cmp(&other.rank())
  }
}

impl From<Builtin> for ZoneKind {
  fn from(builtin: Builtin) -> Self {
    Self::Builtin(builtin)
  }
}

impl AsField for ZoneKind {
  fn as_field(&self) -> Field<'_> {
    Field::Tag(self.as_str())
  }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Module {
  pub span: Span,
  pub name: Option<Name>,
  pub zones: Vec<Zone>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Zone {
  pub span: Span,
  pub kind: ZoneKind,
  pub decls: Vec<Decl>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Import {
  pub span: Span,
  pub path: Vec<Name>,
  pub alias: Option<Name>,
  pub methods: Option<Vec<Name>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportDecl {
  pub span: Span,
  pub name: Name,
  pub methods: Option<Vec<Name>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decl {
  Import(Import),
  Trait(TraitDecl),
  Type(TypeDecl),
  Const(ConstDecl),
  Fn(FnDecl),
  Impl(ImplDecl),
  Export(ExportDecl),
  Host(HostDecl),
  Effect(EffectDecl),
  Extern(ExternDecl),
  Bind(BindDecl),
  Plugin(PluginEntry),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginEntry {
  pub span: Span,
  pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostDecl {
  pub span: Span,
  pub docs: Vec<Span>,
  pub name: Name,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectDecl {
  pub span: Span,
  pub docs: Vec<Span>,
  pub native: bool,
  pub name: Name,
  pub host: Option<Name>,
  pub ops: Vec<MethodSig>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternDecl {
  pub span: Span,
  pub docs: Vec<Span>,
  pub name: Name,
  pub params: Vec<Param>,
  pub return_type: TypeExpr,
  pub effects: Option<EffectRow>,
  pub module: StringLit,
  pub export: Name,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindDecl {
  pub span: Span,
  pub docs: Vec<Span>,
  pub force: bool,
  pub effect: Name,
  pub host: Name,
  pub from: Option<Name>,
  pub module: Option<StringLit>,
  pub ops: Vec<FnDecl>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraitDecl {
  pub span: Span,
  pub docs: Vec<Span>,
  pub name: Name,
  pub param: Name,
  pub methods: Vec<MethodSig>,
  pub recipe: Option<Recipe>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MethodSig {
  pub span: Span,
  pub docs: Vec<Span>,
  pub name: Name,
  pub params: Vec<Param>,
  pub return_type: TypeExpr,
  pub effects: Option<EffectRow>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recipe {
  pub span: Span,
  pub cases: Vec<FnDecl>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImplDecl {
  pub span: Span,
  pub docs: Vec<Span>,
  pub trait_name: Name,
  pub target: TypeRef,
  pub bounds: Vec<Bound>,
  pub methods: Vec<FnDecl>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bound {
  pub span: Span,
  pub trait_name: Name,
  pub var: Name,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Derive {
  pub span: Span,
  pub names: Vec<Name>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeDecl {
  pub span: Span,
  pub docs: Vec<Span>,
  pub name: Name,
  pub params: Vec<Name>,
  pub body: TypeBody,
  pub derive: Option<Derive>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeBody {
  Alias(TypeExpr),
  Variants(VariantBody),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VariantBody {
  pub span: Span,
  pub ctors: Vec<CtorDecl>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CtorDecl {
  pub span: Span,
  pub name: Name,
  pub args: Vec<TypeExpr>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConstDecl {
  pub span: Span,
  pub docs: Vec<Span>,
  pub name: Name,
  pub ty: Option<TypeExpr>,
  pub value: Expr,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FnDecl {
  pub span: Span,
  pub docs: Vec<Span>,
  pub name: Name,
  pub params: Vec<Param>,
  pub return_type: Option<TypeExpr>,
  pub effects: Option<EffectRow>,
  pub bounds: Vec<Bound>,
  pub body: Block,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Param {
  pub span: Span,
  pub name: Name,
  pub pattern: Option<Pattern>,
  pub ty: Option<TypeExpr>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeExpr {
  Ref(TypeRef),
  Var(TypeVar),
  Fn(FnType),
  Record(RecordType),
  Invalid(InvalidType),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeRef {
  pub span: Span,
  pub name: Name,
  pub args: Vec<TypeExpr>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeVar {
  pub span: Span,
  pub name: Name,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FnType {
  pub span: Span,
  pub params: Vec<TypeExpr>,
  pub ret: Box<TypeExpr>,
  pub effects: Option<EffectRow>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordType {
  pub span: Span,
  pub fields: Vec<FieldType>,
  pub tail: Option<Name>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldType {
  pub span: Span,
  pub name: Name,
  pub ty: TypeExpr,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectRow {
  pub span: Span,
  pub entries: Vec<TypeRef>,
  pub tail: Option<Name>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidType {
  pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expr {
  Int(IntLit),
  Float(FloatLit),
  Bool(BoolLit),
  String(StringLit),
  Var(Var),
  Field(FieldAccess),
  Call(Call),
  Pipe(Pipe),
  Binary(Binary),
  Unary(Unary),
  Record(RecordLit),
  List(ListLit),
  Lambda(Box<Lambda>),
  Block(Block),
  If(If),
  Match(Match),
  Return(Return),
  Throw(Throw),
  Try(Try),
  Invalid(InvalidExpr),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Throw {
  pub span: Span,
  pub value: Box<Expr>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Try {
  pub span: Span,
  pub body: Box<Block>,
  pub catch_span: Span,
  pub arms: Vec<MatchArm>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntLit {
  pub span: Span,
  pub raw: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FloatLit {
  pub span: Span,
  pub raw: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoolLit {
  pub span: Span,
  pub value: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StringLit {
  pub span: Span,
  pub parts: Vec<StringPart>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StringPart {
  Text(StringText),
  Interp(StringInterp),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StringText {
  pub span: Span,
  pub raw: String,
  pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StringInterp {
  pub span: Span,
  pub expr: Box<Expr>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Var {
  pub span: Span,
  pub name: Name,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldAccess {
  pub span: Span,
  pub target: Box<Expr>,
  pub field: Name,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Call {
  pub span: Span,
  pub callee: Box<Expr>,
  pub args: Vec<Expr>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pipe {
  pub span: Span,
  pub left: Box<Expr>,
  pub right: Box<Expr>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BinaryOp {
  Or,
  And,
  Eq,
  NotEq,
  Lt,
  LtEq,
  Gt,
  GtEq,
  Add,
  Sub,
  Mul,
  Div,
  Rem,
  BitAnd,
  BitOr,
  BitXor,
}

impl BinaryOp {
  #[must_use]
  pub fn as_str(self) -> &'static str {
    match self {
      Self::Or => "||",
      Self::And => "&&",
      Self::Eq => "==",
      Self::NotEq => "!=",
      Self::Lt => "<",
      Self::LtEq => "<=",
      Self::Gt => ">",
      Self::GtEq => ">=",
      Self::Add => "+",
      Self::Sub => "-",
      Self::Mul => "*",
      Self::Div => "/",
      Self::Rem => "%",
      Self::BitAnd => "&",
      Self::BitOr => "|",
      Self::BitXor => "^",
    }
  }
}

impl AsField for BinaryOp {
  fn as_field(&self) -> Field<'_> {
    Field::Tag(self.as_str())
  }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binary {
  pub span: Span,
  pub op: BinaryOp,
  pub op_span: Span,
  pub left: Box<Expr>,
  pub right: Box<Expr>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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

impl AsField for UnaryOp {
  fn as_field(&self) -> Field<'_> {
    Field::Tag(self.as_str())
  }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unary {
  pub span: Span,
  pub op: UnaryOp,
  pub operand: Box<Expr>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordLit {
  pub span: Span,
  pub spread: Option<Box<Expr>>,
  pub fields: Vec<FieldInit>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListLit {
  pub span: Span,
  pub items: Vec<Expr>,
  pub tail: Option<Box<Expr>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldInit {
  pub span: Span,
  pub name: Name,
  pub value: Expr,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lambda {
  pub span: Span,
  pub params: Vec<Param>,
  pub return_type: Option<TypeExpr>,
  pub effects: Option<EffectRow>,
  pub body: Box<Block>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
  pub span: Span,
  pub stmts: Vec<Stmt>,
  pub result: Box<Expr>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(
  clippy::large_enum_variant,
  reason = "lets are the common statement; boxing would allocate per let"
)]
pub enum Stmt {
  Let(LetStmt),
  Expr(ExprStmt),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LetStmt {
  pub span: Span,
  pub pattern: Pattern,
  pub ty: Option<TypeExpr>,
  pub value: Expr,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExprStmt {
  pub span: Span,
  pub expr: Expr,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct If {
  pub span: Span,
  pub cond: Box<Expr>,
  pub then_branch: Box<Block>,
  pub else_branch: Option<Box<Else>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Else {
  Block(Block),
  If(If),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Match {
  pub span: Span,
  pub subjects: Vec<Expr>,
  pub arms: Vec<MatchArm>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchArm {
  pub span: Span,
  pub rows: Vec<Vec<Pattern>>,
  pub guard: Option<Box<Expr>>,
  pub body: Expr,
}

impl Module {
  #[must_use]
  pub fn exported_types(&self) -> Vec<String> {
    let decls: Vec<&Decl> = self.zones.iter().flat_map(|z| &z.decls).collect();

    decls
      .iter()
      .filter_map(|d| match d {
        Decl::Export(e) if e.methods.is_none() => Some(e.name.text.as_str()),
        _ => None,
      })
      .filter(|name| {
        decls.iter().any(|d| matches!(d, Decl::Type(t) if t.name.text == *name))
      })
      .map(str::to_string)
      .collect()
  }
}

impl MatchArm {
  #[must_use]
  pub fn first_pattern(&self) -> &Pattern {
    &self.rows[0][0]
  }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Return {
  pub span: Span,
  pub value: Box<Expr>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidExpr {
  pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pattern {
  Wildcard(PWildcard),
  Var(PVar),
  Lit(PLit),
  Ctor(PCtor),
  Record(PRecord),
  List(PList),
  Or(POr),
  Invalid(InvalidPattern),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct POr {
  pub span: Span,
  pub alternatives: Vec<Pattern>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PWildcard {
  pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PVar {
  pub span: Span,
  pub name: Name,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PLit {
  pub span: Span,
  pub lit: PatLit,
  pub negative: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PatLit {
  Int(IntLit),
  Float(FloatLit),
  Bool(BoolLit),
  String(StringLit),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PCtor {
  pub span: Span,
  pub name: Name,
  pub args: Vec<Pattern>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PRecord {
  pub span: Span,
  pub fields: Vec<PField>,
  pub open: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PList {
  pub span: Span,
  pub items: Vec<Pattern>,
  pub tail: Option<Box<Pattern>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PField {
  pub span: Span,
  pub name: Name,
  pub pattern: Pattern,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidPattern {
  pub span: Span,
}

ast! {
  structs {
    Module      { span, name, zones }
    Zone        { span, kind, decls }
    Import      { span, path, alias, methods }
    ExportDecl  { span, name, methods }
    HostDecl    { span, docs, name }
    EffectDecl  { span, docs, native, name, host, ops }
    ExternDecl  { span, docs, name, params, return_type, effects, module, export }
    BindDecl    { span, docs, force, effect, host, from, module, ops }
    TraitDecl   { span, docs, name, param, methods, recipe }
    MethodSig   { span, docs, name, params, return_type, effects }
    Recipe      { span, cases }
    ImplDecl    { span, docs, trait_name, target, bounds, methods }
    Bound       { span, trait_name, var }
    Derive      { span, names }
    TypeDecl    { span, docs, name, params, body, derive }
    VariantBody { span, ctors }
    CtorDecl    { span, name, args }
    ConstDecl   { span, docs, name, ty, value }
    FnDecl      { span, docs, name, params, return_type, effects, bounds, body }
    Param       { span, name, pattern, ty }
    TypeRef     { span, name, args }
    TypeVar     { span, name }
    FnType      { span, params, ret, effects }
    RecordType  { span, fields, tail }
    FieldType   { span, name, ty }
    EffectRow   { span, entries, tail }
    InvalidType { span }
    IntLit       { span, raw }
    FloatLit     { span, raw }
    BoolLit      { span, value }
    StringLit    { span, parts }
    StringText   { span, raw, value }
    StringInterp { span, expr }
    Var          { span, name }
    FieldAccess  { span, target, field }
    Call         { span, callee, args }
    Pipe         { span, left, right }
    Binary       { span, op, op_span, left, right }
    Unary        { span, op, operand }
    RecordLit    { span, spread, fields }
    ListLit      { span, items, tail }
    FieldInit    { span, name, value }
    Lambda       { span, params, return_type, effects, body }
    Block        { span, stmts, result }
    LetStmt      { span, pattern, ty, value }
    ExprStmt     { span, expr }
    If           { span, cond, then_branch, else_branch }
    Match        { span, subjects, arms }
    MatchArm     { span, rows, guard, body }
    Return       { span, value }
    Throw        { span, value }
    Try          { span, body, catch_span, arms }
    InvalidExpr  { span }
    PWildcard      { span }
    PVar           { span, name }
    PLit           { span, lit, negative }
    PCtor          { span, name, args }
    PRecord        { span, fields, open }
    PList          { span, items, tail }
    POr            { span, alternatives }
    PField         { span, name, pattern }
    InvalidPattern { span }
    PluginEntry    { span, text }
  }
  enums {
    Decl       { Import, Trait, Type, Const, Fn, Impl, Export, Host, Effect,
                 Extern, Bind, Plugin }
    TypeBody   { Alias, Variants }
    TypeExpr   { Ref, Var, Fn, Record, Invalid }
    Expr       { Int, Float, Bool, String, Var, Field, Call, Pipe, Binary,
                 Unary, Record, List, Lambda, Block, If, Match, Return, Throw,
                 Try, Invalid }
    StringPart { Text, Interp }
    Stmt       { Let, Expr }
    Else       { Block, If }
    Pattern    { Wildcard, Var, Lit, Ctor, Record, List, Or, Invalid }
    PatLit     { Int, Float, Bool, String }
  }
}
