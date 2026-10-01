pub use crate::syntax::ast::NodeRef;
use crate::{shared::source::Span, syntax::ast::Name};

pub enum Field<'a> {
  Span(&'a Span),
  Spans(&'a [Span]),
  Name(&'a Name),
  Names(&'a [Name]),
  OptName(Option<&'a Name>),
  OptNames(Option<&'a [Name]>),
  Text(&'a str),
  Bool(bool),
  Tag(&'static str),
  Node(NodeRef<'a>),
  OptNode(Option<NodeRef<'a>>),
  Nodes(Vec<NodeRef<'a>>),
}

pub trait Fields {
  fn fields(&self) -> Vec<(&'static str, Field<'_>)>;
}

pub trait AsNode {
  fn as_node(&self) -> NodeRef<'_>;
}

impl<T: AsNode> AsNode for Box<T> {
  fn as_node(&self) -> NodeRef<'_> {
    (**self).as_node()
  }
}

pub trait AsField {
  fn as_field(&self) -> Field<'_>;
}

impl AsField for Span {
  fn as_field(&self) -> Field<'_> {
    Field::Span(self)
  }
}

impl AsField for Vec<Span> {
  fn as_field(&self) -> Field<'_> {
    Field::Spans(self)
  }
}

impl AsField for Name {
  fn as_field(&self) -> Field<'_> {
    Field::Name(self)
  }
}

impl AsField for Vec<Name> {
  fn as_field(&self) -> Field<'_> {
    Field::Names(self)
  }
}

impl AsField for Option<Name> {
  fn as_field(&self) -> Field<'_> {
    Field::OptName(self.as_ref())
  }
}

impl AsField for Option<Vec<Name>> {
  fn as_field(&self) -> Field<'_> {
    Field::OptNames(self.as_deref())
  }
}

impl AsField for String {
  fn as_field(&self) -> Field<'_> {
    Field::Text(self)
  }
}

impl AsField for bool {
  fn as_field(&self) -> Field<'_> {
    Field::Bool(*self)
  }
}

macro_rules! ast {
  (
    structs { $($s:ident { $($sf:ident),* $(,)? })* }
    enums   { $($e:ident { $($v:ident),* $(,)? })* }
  ) => {
    #[derive(Clone, Copy)]
    pub enum NodeRef<'a> { $($s(&'a $s),)* }

    impl<'a> NodeRef<'a> {
      #[must_use]
      pub fn kind(&self) -> &'static str {
        match self { $(Self::$s(_) => stringify!($s),)* }
      }

      #[must_use]
      pub fn span(&self) -> &'a Span {
        match self { $(Self::$s(n) => &n.span,)* }
      }

      #[must_use]
      pub fn fields(&self) -> Vec<(&'static str, Field<'a>)> {
        match self { $(Self::$s(n) => Fields::fields(*n),)* }
      }
    }

    $(
      impl AsNode for $s {
        fn as_node(&self) -> NodeRef<'_> { NodeRef::$s(self) }
      }

      impl Fields for $s {
        fn fields(&self) -> Vec<(&'static str, Field<'_>)> {
          let Self { $($sf),* } = self;
          vec![$((stringify!($sf), AsField::as_field($sf)),)*]
        }
      }
    )*

    $(
      impl $e {
        #[must_use]
        pub fn span(&self) -> &Span { AsNode::as_node(self).span() }
      }

      impl AsNode for $e {
        fn as_node(&self) -> NodeRef<'_> {
          match self { $(Self::$v(inner) => AsNode::as_node(inner),)* }
        }
      }

      impl Fields for $e {
        fn fields(&self) -> Vec<(&'static str, Field<'_>)> {
          AsNode::as_node(self).fields()
        }
      }
    )*

    $( ast!(@as_field $s); )*
    $( ast!(@as_field $e); )*
  };

  (@as_field $t:ident) => {
    impl AsField for $t {
      fn as_field(&self) -> Field<'_> { Field::Node(AsNode::as_node(self)) }
    }

    impl AsField for Box<$t> {
      fn as_field(&self) -> Field<'_> { Field::Node(AsNode::as_node(&**self)) }
    }

    impl AsField for Option<$t> {
      fn as_field(&self) -> Field<'_> {
        Field::OptNode(self.as_ref().map(AsNode::as_node))
      }
    }

    impl AsField for Option<Box<$t>> {
      fn as_field(&self) -> Field<'_> {
        Field::OptNode(self.as_deref().map(AsNode::as_node))
      }
    }

    impl AsField for Vec<$t> {
      fn as_field(&self) -> Field<'_> {
        Field::Nodes(self.iter().map(AsNode::as_node).collect())
      }
    }
  };
}

pub(crate) use ast;

#[must_use]
pub fn children(node: NodeRef<'_>) -> Vec<NodeRef<'_>> {
  let mut out = Vec::new();

  for (_, field) in node.fields() {
    match field {
      Field::Node(n) | Field::OptNode(Some(n)) => out.push(n),
      Field::Nodes(ns) => out.extend(ns),
      _ => {}
    }
  }

  out
}
