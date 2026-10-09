#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuiltinInfo {
  pub module: &'static str,
  pub member: &'static str,
  pub arity: usize,
  pub signature: &'static str,
}

pub const BUILTIN_MODULES: &[&str] =
  &["Bool", "Debug", "Float", "Int", "JsonRaw", "Log", "RefRaw", "String"];

pub const STD_ONLY: &[(&str, &str)] = &[("JsonRaw", "Json"), ("RefRaw", "Ref")];

pub const BUILTINS: &[BuiltinInfo] = &[
  BuiltinInfo {
    module: "RefRaw",
    member: "new",
    arity: 1,
    signature: "function(a) -> Ref<a>",
  },
  BuiltinInfo {
    module: "RefRaw",
    member: "get",
    arity: 1,
    signature: "function(Ref<a>) -> a / {Mut}",
  },
  BuiltinInfo {
    module: "RefRaw",
    member: "set",
    arity: 2,
    signature: "function(Ref<a>, a) -> {} / {Mut}",
  },
  BuiltinInfo {
    module: "JsonRaw",
    member: "parse",
    arity: 1,
    signature: "function(String) -> Result<String, JsonValue>",
  },
  BuiltinInfo {
    module: "JsonRaw",
    member: "print",
    arity: 1,
    signature: "function(JsonValue) -> String",
  },
  BuiltinInfo {
    module: "JsonRaw",
    member: "lookup",
    arity: 2,
    signature: "function(List<{ key: String, value: JsonValue }>, String) -> Option<JsonValue>",
  },
  BuiltinInfo {
    module: "Int",
    member: "to_float",
    arity: 1,
    signature: "function(Int) -> Float",
  },
  BuiltinInfo {
    module: "Int",
    member: "parse",
    arity: 1,
    signature: "function(String) -> Option<Int>",
  },
  BuiltinInfo {
    module: "String",
    member: "split",
    arity: 2,
    signature: "function(String, String) -> List<String>",
  },
  BuiltinInfo {
    module: "String",
    member: "split_once",
    arity: 2,
    signature: "function(String, String) -> Option<{ before: String, after: String }>",
  },
  BuiltinInfo {
    module: "String",
    member: "chars",
    arity: 1,
    signature: "function(String) -> List<String>",
  },
  BuiltinInfo {
    module: "Int",
    member: "to_base",
    arity: 2,
    signature: "function(Int, Int) -> String",
  },
  BuiltinInfo {
    module: "Float",
    member: "to_int",
    arity: 1,
    signature: "function(Float) -> Int",
  },
  BuiltinInfo {
    module: "Int",
    member: "eq",
    arity: 2,
    signature: "function(Int, Int) -> Bool",
  },
  BuiltinInfo {
    module: "Float",
    member: "eq",
    arity: 2,
    signature: "function(Float, Float) -> Bool",
  },
  BuiltinInfo {
    module: "Float",
    member: "to_string",
    arity: 1,
    signature: "function(Float) -> String",
  },
  BuiltinInfo {
    module: "String",
    member: "eq",
    arity: 2,
    signature: "function(String, String) -> Bool",
  },
  BuiltinInfo {
    module: "Bool",
    member: "eq",
    arity: 2,
    signature: "function(Bool, Bool) -> Bool",
  },
  BuiltinInfo {
    module: "Bool",
    member: "to_string",
    arity: 1,
    signature: "function(Bool) -> String",
  },
  BuiltinInfo {
    module: "Int",
    member: "to_string",
    arity: 1,
    signature: "function(Int) -> String",
  },
  BuiltinInfo {
    module: "Log",
    member: "info",
    arity: 1,
    signature: "function(String) -> {}",
  },
  BuiltinInfo {
    module: "Log",
    member: "error",
    arity: 1,
    signature: "function(String) -> {}",
  },
  BuiltinInfo {
    module: "String",
    member: "concat",
    arity: 2,
    signature: "function(String, String) -> String",
  },
  BuiltinInfo {
    module: "String",
    member: "contains",
    arity: 2,
    signature: "function(String, String) -> Bool",
  },
  BuiltinInfo {
    module: "String",
    member: "ends_with",
    arity: 2,
    signature: "function(String, String) -> Bool",
  },
  BuiltinInfo {
    module: "String",
    member: "length",
    arity: 1,
    signature: "function(String) -> Int",
  },
  BuiltinInfo {
    module: "String",
    member: "lowercase",
    arity: 1,
    signature: "function(String) -> String",
  },
  BuiltinInfo {
    module: "String",
    member: "repeat",
    arity: 2,
    signature: "function(String, Int) -> String",
  },
  BuiltinInfo {
    module: "String",
    member: "replace",
    arity: 3,
    signature: "function(String, String, String) -> String",
  },
  BuiltinInfo {
    module: "String",
    member: "slice",
    arity: 3,
    signature: "function(String, Int, Int) -> String",
  },
  BuiltinInfo {
    module: "String",
    member: "starts_with",
    arity: 2,
    signature: "function(String, String) -> Bool",
  },
  BuiltinInfo {
    module: "String",
    member: "trim",
    arity: 1,
    signature: "function(String) -> String",
  },
  BuiltinInfo {
    module: "String",
    member: "uppercase",
    arity: 1,
    signature: "function(String) -> String",
  },
  BuiltinInfo {
    module: "String",
    member: "charCodeAt",
    arity: 2,
    signature: "function(String, Int) -> Int",
  },
  BuiltinInfo {
    module: "Float",
    member: "sqrt",
    arity: 1,
    signature: "function(Float) -> Float",
  },
  BuiltinInfo {
    module: "Float",
    member: "abs",
    arity: 1,
    signature: "function(Float) -> Float",
  },
  BuiltinInfo {
    module: "Float",
    member: "floor",
    arity: 1,
    signature: "function(Float) -> Float",
  },
  BuiltinInfo {
    module: "Float",
    member: "sin",
    arity: 1,
    signature: "function(Float) -> Float",
  },
  BuiltinInfo {
    module: "Float",
    member: "cos",
    arity: 1,
    signature: "function(Float) -> Float",
  },
  BuiltinInfo {
    module: "Float",
    member: "atan2",
    arity: 2,
    signature: "function(Float, Float) -> Float",
  },
];

#[must_use]
pub fn std_owner(module: &str) -> Option<&'static str> {
  STD_ONLY.iter().find(|(m, _)| *m == module).map(|(_, owner)| *owner)
}

#[must_use]
pub fn is_module(name: &str) -> bool {
  BUILTIN_MODULES.binary_search(&name).is_ok()
}

#[must_use]
pub fn lookup(module: &str, member: &str) -> Option<&'static BuiltinInfo> {
  BUILTINS.iter().find(|b| b.module == module && b.member == member)
}

pub fn members(module: &str) -> impl Iterator<Item = &'static str> + '_ {
  BUILTINS.iter().filter(move |b| b.module == module).map(|b| b.member)
}
