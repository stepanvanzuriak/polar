use polar_plugin::{Span, Token};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Type {
  pub name: String,
  pub args: Vec<Type>,
  pub span: Span,
}

impl Type {
  pub fn text(&self) -> String {
    if self.args.is_empty() {
      return self.name.clone();
    }

    let args: Vec<String> = self.args.iter().map(Type::text).collect();

    format!("{}<{}>", self.name, args.join(", "))
  }

  pub fn is(&self, name: &str) -> bool {
    self.name == name && self.args.is_empty()
  }

  pub fn arg_of(&self, name: &str) -> Option<&Type> {
    match self.args.as_slice() {
      [arg] if self.name == name => Some(arg),
      _ => None,
    }
  }
}

pub fn parse(tokens: &[&Token], at: &mut usize) -> Option<Type> {
  let name = tokens.get(*at).filter(|t| t.is("Upper"))?;
  let mut span = name.span;
  let mut args = Vec::new();

  *at += 1;

  if tokens.get(*at).is_some_and(|t| t.is("Lt")) {
    *at += 1;

    loop {
      args.push(parse(tokens, at)?);

      match tokens.get(*at) {
        Some(t) if t.is("Comma") => *at += 1,
        Some(t) if t.is("Gt") => {
          span = span.join(t.span);
          *at += 1;
          break;
        }
        _ => return None,
      }
    }
  }

  Some(Type { name: name.text.clone(), args, span })
}
