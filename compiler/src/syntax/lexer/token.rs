use crate::shared::source::Span;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
  pub kind: TokenKind,
  pub span: Span,
  pub newline_before: bool,
  pub value: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommentKind {
  Line,
  Doc,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Comment {
  pub kind: CommentKind,
  pub span: Span,
}

macro_rules! token_kinds {
  (
    plain { $($p_name:ident => $p_display:literal),+ $(,)? }
    keywords { $($kw_name:ident => $kw_text:literal),+ $(,)? }
    punct { $($pu_name:ident => $pu_text:literal),+ $(,)? }
    eof { $eof_name:ident => $eof_display:literal }
  ) => {
       #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
          pub enum TokenKind {
            $($p_name,)+
            $($kw_name,)+
            $($pu_name,)+
            $eof_name,
          }

        impl TokenKind {
          pub const ALL: &[TokenKind] = &[
                $(TokenKind::$p_name,)+
                $(TokenKind::$kw_name,)+
                $(TokenKind::$pu_name,)+
                TokenKind::$eof_name,
              ];

          pub fn display_name(self) -> &'static str {
                match self {
                  $(TokenKind::$p_name => $p_display,)+
                  $(TokenKind::$kw_name => concat!("keyword `", $kw_text, "`"),)+
                  $(TokenKind::$pu_name => concat!("`", $pu_text, "`"),)+
                  TokenKind::$eof_name => $eof_display,
                }
          }
        }

        pub fn keyword(ident: &str) -> Option<TokenKind> {
              match ident {
                $($kw_text => Some(TokenKind::$kw_name),)+
                _ => None,
              }
        }
     };
}

token_kinds! {
  plain {
    Lower => "identifier",
    Upper => "type or constructor name",
    Underscore => "`_`",
    Int => "integer literal",
    Float => "float literal",
    StringStart => "string",
    StringPart => "string",
    InterpStart => "`${`",
    InterpEnd => "`}`",
    StringEnd => "string",
    Unknown => "text",
  }
  keywords {
    KwModule => "module",
    KwUses => "uses",
    KwHosts => "hosts",
    KwTraits => "traits",
    KwTypes => "types",
    KwConstants => "constants",
    KwEffects => "effects",
    KwExterns => "externs",
    KwBinds => "binds",
    KwFunctions => "functions",
    KwImpls => "impls",
    KwExports => "exports",
    KwFunction => "function",
    KwLet => "let",
    KwDerive => "derive",
    KwMatch => "match",
    KwIf => "if",
    KwElse => "else",
    KwAs => "as",
    KwTrue => "true",
    KwFalse => "false",
    KwType => "type",
    KwImport => "import",
    KwPub => "pub",
    KwExport => "export",
    KwFor => "for",
    KwWhere => "where",
    KwTrait => "trait",
    KwImpl => "impl",
    KwIn => "in",
    KwNative => "native",
    KwForce => "force",
    KwThrow => "throw",
    KwReturn => "return",
    KwTry => "try",
    KwCatch => "catch",
    KwHost => "host",
    KwEffect => "effect",
    KwBind => "bind",
    KwExtern => "extern",
  }
  punct {
    LParen => "(", RParen => ")",
    LBrace => "{", RBrace => "}",
    LBracket => "[", RBracket => "]",
    Comma => ",", Colon => ":", Dot => ".",
    Arrow => "->", PipeOp => "|>",
    Eq => "=", EqEq => "==", BangEq => "!=",
    Lt => "<", Le => "<=", Gt => ">", Ge => ">=",
    Plus => "+", Minus => "-", Star => "*", Slash => "/", Percent => "%",
    AndAnd => "&&", OrOr => "||", Bang => "!", Bar => "|", DotDot => "..",
    Amp => "&", Caret => "^", Tilde => "~",
  }
  eof {
    Eof => "end of file"
  }
}
