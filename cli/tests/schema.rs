use polar_compiler::{
  CompileOptions, Stage, compile, dump_stage_with, format_plugins,
  shared::codes::DiagnosticCode::{MissingField, PluginError, TypeMismatch},
  shared::diagnostic::Diagnostic,
};
use std::{
  fs,
  path::{Path, PathBuf},
  process::Command,
  sync::OnceLock,
};

const POLAR: &str = env!("CARGO_BIN_EXE_polar");

fn framework() -> PathBuf {
  PathBuf::from(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../projects/simple_framework"
  ))
}

fn plugin() -> Vec<String> {
  static PATH: OnceLock<String> = OnceLock::new();

  let path = PATH.get_or_init(|| {
    let dir = framework().canonicalize().unwrap();

    polar_cli::plugin::prepare(
      &dir,
      Path::new("projects/simple_framework"),
      "simple_framework",
      &["schema".to_string(), "routes".to_string(), "views".to_string()],
      &std::collections::BTreeMap::new(),
      "polar.toml",
    )
    .unwrap_or_else(|e| panic!("{e:?}"))
  });

  vec![path.clone()]
}

const USES: &str = "uses\n  Std.Id\n  Std.Json\n  Std.Option\n\n";

const BLOG: &str = "schema
  users
    id    Id<User>  primary
    name  String

  posts
    id         Id<Post>  primary
    title      String
    author_id  Id<User>  references users
";

fn options() -> CompileOptions {
  CompileOptions { plugins: plugin(), ..CompileOptions::default() }
}

fn diagnostics(src: &str) -> Vec<Diagnostic> {
  compile(src, "test.px", &options()).diagnostics
}

fn with_schema(schema: &str) -> String {
  format!("{USES}{schema}")
}

fn dump(src: &str, stage: Stage) -> String {
  let out = dump_stage_with(src, "test.px", stage, &options());

  assert!(out.diagnostics.is_empty(), "{:#?}", out.diagnostics);
  out.output.expect("a dump")
}

#[track_caller]
fn only(src: &str, message: &str, at: &str) {
  let found = diagnostics(src);

  assert_eq!(found.len(), 1, "{found:#?}");
  assert_eq!(found[0].code, PluginError, "{found:#?}");
  assert!(found[0].message.contains(message), "{found:#?}");

  let start = src.rfind(at).unwrap_or_else(|| panic!("`{at}` not in source"));
  let span = &found[0].primary.span;

  assert!(
    span.start >= start && span.end <= start + at.len(),
    "should point at `{at}`: {found:#?}"
  );
}

mod id {
  use super::*;

  #[test]
  fn ids_of_different_records_do_not_unify() {
    let src = "uses\n  Std.Id\n\ntypes\n  Post = { title: String } derive(Eq)\n\n  User = { name: String } derive(Eq)\n\nfunctions\n  f() {\n    let a: Id<Post> = Id(1)\n    let b: Id<User> = a\n\n    b\n  }\n";
    let found = compile(src, "test.px", &CompileOptions::default()).diagnostics;

    assert_eq!(found.len(), 1, "{found:#?}");
    assert_eq!(found[0].code, TypeMismatch, "{found:#?}");
  }
}

mod grammar {
  use super::*;

  #[test]
  fn ast_dump_keeps_the_zone_and_shows_what_it_made() {
    let dump = dump(&with_schema(BLOG), Stage::Ast);

    assert!(dump.contains("(Zone schema"), "{dump}");
    assert!(
      dump.contains("(PluginEntry text=\"users\\nid Id<User> primary"),
      "{dump}"
    );
    assert!(dump.contains("(TypeDecl NewPost"), "{dump}");
    assert!(dump.contains("(EffectDecl native=false Posts"), "{dump}");
  }

  #[test]
  fn alias_names_the_record() {
    let types = dump(
      &with_schema("schema\n  people as Person\n    id  Id<Person>  primary\n"),
      Stage::Types,
    );

    assert!(
      types.contains("People.insert : function({} as NewPerson)"),
      "{types}"
    );
  }

  #[test]
  fn names_follow_the_table() {
    let types = dump(
      &with_schema(
        "schema\n  categories\n    id  Id<Category>  primary\n\n  blog_posts\n    id  Id<BlogPost>  primary\n",
      ),
      Stage::Types,
    );

    assert!(
      types.contains("Categories.find : function(Id<Category>)"),
      "{types}"
    );
    assert!(
      types.contains("BlogPosts.insert : function({} as NewBlogPost)"),
      "{types}"
    );
  }

  #[test]
  fn formatter_aligns_columns_and_keeps_comments() {
    let src = with_schema(
      "schema\n  posts\n    id Id<Post> primary\n    // the headline\n    title   String // short\n    author_id Id<User>  references  users\n\n  users\n    id Id<User> primary\n",
    );
    let once =
      format_plugins(&src, "test.px", &plugin()).output.expect("formats");

    assert!(
      once.contains(
        "  posts\n    id         Id<Post>  primary\n    // the headline\n    title      String // short\n    author_id  Id<User>  references users\n"
      ),
      "{once}"
    );
    assert_eq!(
      format_plugins(&once, "test.px", &plugin()).output.unwrap(),
      once
    );
  }

  #[test]
  fn corpus_round_trips() {
    let src = include_str!("fixtures/plugins/schema.px");

    assert_eq!(
      format_plugins(src, "schema.px", &plugin()).output.as_deref(),
      Some(src)
    );
    assert!(diagnostics(src).is_empty(), "{:#?}", diagnostics(src));
  }

  #[test]
  fn an_unknown_modifier_is_an_error_at_it() {
    only(
      &with_schema("schema\n  posts\n    id  Id<Post>  primary  unique\n"),
      "unknown column modifier `unique`",
      "unique",
    );
  }
}

mod validation {
  use super::*;

  #[test]
  fn no_primary() {
    only(
      &with_schema("schema\n  posts\n    title  String\n"),
      "has no primary key",
      "posts",
    );
  }

  #[test]
  fn two_primaries() {
    only(
      &with_schema(
        "schema\n  posts\n    id     Id<Post>  primary\n    other  Id<Post>  primary\n",
      ),
      "more than one primary key",
      "other",
    );
  }

  #[test]
  fn primary_is_not_an_id_of_itself() {
    only(
      &with_schema("schema\n  posts\n    id  Int  primary\n"),
      "must have type `Id<Post>`",
      "Int",
    );
  }

  #[test]
  fn unknown_reference() {
    only(
      &with_schema(
        "schema\n  posts\n    id         Id<Post>  primary\n    author_id  Id<Person>  references people\n",
      ),
      "no table `people`",
      "people",
    );
  }

  #[test]
  fn a_table_from_another_module_needs_its_type_in_scope() {
    let src = with_schema(
      "schema\n  posts\n    id         Id<Post>  primary\n    author_id  Id<User>  references users\n",
    );
    let found = diagnostics(&src);

    assert_eq!(found.len(), 1, "{found:#?}");
    assert!(found[0].message.contains("cannot find type `User`"), "{found:#?}");
    assert_eq!(
      found[0].primary.span.start,
      src.find("Id<User>  references").unwrap()
    );
  }

  #[test]
  fn reference_type_mismatch() {
    only(
      &with_schema(
        "schema\n  users\n    id  Id<User>  primary\n\n  posts\n    id         Id<Post>  primary\n    author_id  Int       references users\n",
      ),
      "its type must be `Id<User>`",
      "Int",
    );
  }

  #[test]
  fn an_optional_reference_is_fine() {
    let src = with_schema(
      "schema\n  users\n    id  Id<User>  primary\n\n  posts\n    id         Id<Post>          primary\n    author_id  Option<Id<User>>  references users\n",
    );

    assert!(diagnostics(&src).is_empty(), "{:#?}", diagnostics(&src));
  }

  #[test]
  fn people_needs_as() {
    only(
      &with_schema("schema\n  people\n    id  Id<Person>  primary\n"),
      "doesn't end in `s`",
      "people",
    );
  }

  #[test]
  fn duplicate_column() {
    only(
      &with_schema(
        "schema\n  posts\n    id     Id<Post>  primary\n    title  String\n    title  String\n",
      ),
      "appears more than once",
      "title",
    );
  }

  #[test]
  fn duplicate_table() {
    only(
      &with_schema(
        "schema\n  posts\n    id  Id<Post>  primary\n\n  posts\n    id  Id<Post>  primary\n",
      ),
      "declared more than once",
      "posts",
    );
  }

  #[test]
  fn a_column_type_outside_the_set() {
    only(
      &with_schema(
        "schema\n  posts\n    id    Id<Post>   primary\n    tags  List<Int>\n",
      ),
      "can't have this type",
      "List<Int>",
    );
  }

  #[test]
  fn missing_json_import() {
    let src = format!("uses\n  Std.Id\n\n{BLOG}");

    only(&src, "needs `Std.Json`", "schema");
  }

  #[test]
  fn missing_both_imports() {
    only(BLOG, "needs `Std.Id` and `Std.Json`", "schema");
  }

  #[test]
  fn a_table_in_a_host_needs_std_table() {
    let src = with_schema(&BLOG.replace("  posts\n", "  posts in Node\n"));

    only(&src, "needs `Std.Table`", "schema");
  }
}

mod expansion {
  use super::*;

  #[test]
  fn the_blog_type_checks() {
    let types = dump(&with_schema(BLOG), Stage::Types);

    assert!(
      types.contains(
        "Posts.find : function(Id<Post>) -> Option<{ author_id: Id<User>, id: Id<Post>, title: String } as Post>"
      ),
      "{types}"
    );
    assert!(
      types.contains("Users.delete : function(Id<User>) -> Bool"),
      "{types}"
    );
    assert!(types.contains("schema_posts : String"), "{types}");
  }

  #[test]
  fn a_missing_column_points_at_the_call_and_the_column() {
    let src = format!(
      "{USES}hosts\n  Node\n\n{BLOG}\nfunctions\n  main() -> {{}} / {{Posts}} {{\n    let _ = Posts.insert({{ title: \"x\" }})\n\n    {{}}\n  }}\n"
    );
    let found: Vec<Diagnostic> = diagnostics(&src)
      .into_iter()
      .filter(|d| d.code == MissingField)
      .collect();

    assert_eq!(found.len(), 1, "{found:#?}");
    assert_eq!(
      found[0].primary.span.start,
      src.find("{ title: \"x\" }").unwrap()
    );
    assert_eq!(
      found[0].secondary[0].span.start,
      src.find("author_id  Id<User>").unwrap(),
      "{found:#?}"
    );
  }

  const POSTS_JS: &str = "const rows = new Map();
let next = 1;

export function insert(row) {
  const post = { ...row, id: { $: \"Id\", _0: next++ } };

  rows.set(post.id._0, post);
  return post;
}

export function find(id) {
  return rows.get(id._0) ?? null;
}

export function all() {
  return [...rows.values()];
}

export function update(row) {
  const known = rows.has(row.id._0);

  if (known) rows.set(row.id._0, row);
  return known;
}

function remove(id) {
  return rows.delete(id._0);
}

export { remove as delete };
";

  #[test]
  fn insert_then_find_runs_under_polar_run() {
    let dir = tempfile::tempdir().unwrap();
    let app = dir.path().join("app");
    let main = format!(
      "module Main\n\n{USES}hosts\n  Node\n\n{BLOG}\nbinds\n  Posts in Node = \"./posts.js\"\n\nfunctions\n  main() -> {{}} / {{Posts}} {{\n    let post = Posts.insert({{ title: \"hello\", author_id: Id(7) }})\n\n    match Posts.find(post.id) {{\n      Some(found) -> Log.info(\"#{{found.title}} by #{{Id.value(found.author_id)}}: #{{found == post}}\"),\n      None -> Log.info(\"missing\"),\n    }}\n\n    Log.info(\"#{{Posts.delete(post.id)}} #{{Posts.delete(post.id)}} #{{schema_users}}\")\n  }}\n\nexports\n  main\n"
    );

    fs::create_dir_all(app.join("src")).unwrap();
    fs::write(
      app.join("polar.toml"),
      format!(
        "[project]\nname = \"app\"\n\n[dependencies]\nsimple_framework = {{ path = \"{}\" }}\n",
        framework().canonicalize().unwrap().display()
      ),
    )
    .unwrap();
    fs::write(app.join("src/main.px"), main).unwrap();
    fs::write(app.join("src/posts.js"), POSTS_JS).unwrap();

    let out =
      Command::new(POLAR).arg("run").current_dir(&app).output().unwrap();

    assert_eq!(
      String::from_utf8_lossy(&out.stdout),
      "hello by 7: true\ntrue false {\"table\":\"users\",\"record\":\"User\",\"columns\":[{\"name\":\"id\",\"type\":\"Id<User>\",\"primary\":true},{\"name\":\"name\",\"type\":\"String\"}]}\n",
      "{}",
      String::from_utf8_lossy(&out.stderr)
    );
  }

  #[test]
  fn a_table_in_a_host_is_kept_in_memory() {
    let dir = tempfile::tempdir().unwrap();
    let app = dir.path().join("app");
    let blog = BLOG.replace("  posts\n", "  posts in Node\n");
    let main = format!(
      "module Main\n\nuses\n  Std.Id\n  Std.Json\n  Std.List\n  Std.Option\n  Std.Table\n\nhosts\n  Node\n\n{blog}\nfunctions\n  main() -> {{}} / {{Posts}} {{\n    let post = Posts.insert({{ title: \"hello\", author_id: Id(7) }})\n    let other = Posts.insert({{ title: \"other\", author_id: Id(8) }})\n\n    match Posts.find(post.id) {{\n      Some(found) -> Log.info(\"#{{found.title}} #{{Id.value(found.id)}} #{{found == post}}\"),\n      None -> Log.info(\"missing\"),\n    }}\n\n    Log.info(\"#{{Posts.update({{ ..other, title: \"edited\" }})}} #{{Posts.delete(post.id)}} #{{Posts.delete(post.id)}}\")\n    Log.info(List.join(List.map(Posts.all(), function(p) {{ \"#{{Id.value(p.id)}}:#{{p.title}}\" }}), \",\"))\n  }}\n\nexports\n  main\n"
    );

    fs::create_dir_all(app.join("src")).unwrap();
    fs::write(
      app.join("polar.toml"),
      format!(
        "[project]\nname = \"app\"\n\n[dependencies]\nsimple_framework = {{ path = \"{}\" }}\n",
        framework().canonicalize().unwrap().display()
      ),
    )
    .unwrap();
    fs::write(app.join("src/main.px"), main).unwrap();

    let out =
      Command::new(POLAR).arg("run").current_dir(&app).output().unwrap();

    assert_eq!(
      String::from_utf8_lossy(&out.stdout),
      "hello 1 true\ntrue true false\n2:edited\n",
      "{}",
      String::from_utf8_lossy(&out.stderr)
    );
  }
}
