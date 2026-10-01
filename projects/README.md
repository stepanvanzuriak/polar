# Projects

Each directory here is a Polar project: a `polar.toml` plus its sources.

```toml
[project]
name = "todo"
src = "src"            # default "src"
out = "dist"           # default "dist"
main = "src/main.px"   # default "src/main.px", what `polar run` runs
hosts = ["Browser", "Node"]  # optional: the hosts `polar build` builds for
```

With `hosts`, `polar build` builds every listed host into its own directory,
`dist/Browser/` and `dist/Node/`, each with only that host's bindings linked in.
The JavaScript files a host's externs use are copied next to its output, so each
directory runs on its own.
`--host Node` builds just one, into `dist/`. `polar run` uses the only listed
host, or needs `--host` when there are several. Without `hosts`, a program that
declares one host builds for it, and one that declares several needs `--host`.

Paths are relative to the project directory, so each project builds into its own
`dist/` (ignored by git):

```sh
polar build projects/todo              # → projects/todo/dist/
polar build projects/todo projects/x   # each into its own dist/
polar run projects/todo                # runs src/main.px
polar check projects/todo --watch
polar fmt projects/todo

cd projects/todo && polar run          # inside a project, no arguments needed
```

A project may have a `main.expected.txt`: `cargo test` runs every project here and
compares its output with it. A project that lists `hosts` is run once with each
`--host`, all against `main.expected.txt`; give it a `main.<Host>.expected.txt`
per host instead when the output differs between hosts.

A project with an `e2e.mjs` is a full-stack one, so the `main.expected.txt`
runs skip it. `cargo test` runs `polar build --out <temp>/dist` in place (so path
dependencies resolve), then `node e2e.mjs <temp>/dist`, and compares stdout with
`e2e.expected.txt`.
`bridge_posts/` is the example: its `e2e.mjs` serves `dist/Node/_polar/bridges.js`
over HTTP and runs the browser bundle's `start` against it.

To start a new one, copy `todo/`, rename it in `polar.toml`, and replace
`src/main.px`.

## Packages and dependencies

A `polar.toml` with `[package]` instead of `[project]` is a library: it isn't
built or run on its own, only `polar check`ed or depended on. It owns one
top-level module name, and its files are named without it:
`simple_framework/src/html.px` is `module Html`, imported as `Framework.Html`
(also from the package's own files).

```toml
[package]
name = "simple_framework"
module = "Framework"   # claims every `uses Framework.…`

[dependencies]         # on [project] and [package] alike
shapes = { path = "../shapes" }  # a package on disk, relative to this file
```

A `uses` path whose first segment is a direct dependency's `module` resolves in
that package; everything else resolves in the project's own `src`. Dependencies
compile into `dist/_deps/<package>/`.

## Zone plugins

A package can define new zones with a plugin written in Rust:

```toml
[plugin]
zones = ["schema"]                 # the zones the plugin defines
dependencies = { regex = "1" }     # optional: crates the plugin uses
```

The plugin's code is the package's `plugin/lib.rs` (plus any modules next to it).
If it doesn't exist, polar writes a stub for each listed zone. Everything needed
to build it — a generated `Cargo.toml`, the `polar-plugin` API crate, the build
output — goes in `.polar/`, which polar regenerates and git ignores. Building
needs `cargo`.

A project gets a package's zones by depending on it, and so does the package's
own code.

## `simple_framework` and `framework_demo`

`simple_framework` (module `Framework`) is a small web framework as a package:

- the `schema` zone (`plugin/schema.rs`): each table becomes a record type, an
  insert type without the primary key, and a CRUD effect (`find`, `all`, `insert`,
  `update`, `delete`). A table named with a host, `posts in Node`, is also bound
  there in memory, on a `Std.Table` constant (the module needs `uses Std.Table`);
  otherwise the project binds the effect itself;
- the `routes` zone (`plugin/routes.rs`): `GET  /posts/:id  -> show` lines become
  an exported `router(request: Request) -> Option<Response>`. A `:name` segment
  fills the handler parameter of that name (`Int` or `String`; an `Int` that
  doesn't parse doesn't match), and a `Request` parameter gets the request;
- the `views` zone (`plugin/views.rs`): JSX-style markup. Each entry is a
  header like `card(post: Post)` with markup indented below it, and becomes a
  function returning `Html`. `{expr}` holes go through the `Render` trait
  (`String` is escaped; `Int`, `Id<a>`, `Html`, `List<a>` and `Option<a>` render
  too), so the type checker, not the plugin, picks how a value prints. `<Card
  post={p} />` calls the view or function `card` with arguments by parameter
  name, and a `children: Html` parameter receives the markup between its tags.
  Text follows JSX's whitespace rules and may hold any character except `<`,
  `{`, an unbalanced `"` or `//` (which starts a comment); write those as
  `{"…"}`. The module needs `Framework.Html` in `uses`;
- `Framework.Html`: the `Html` type, the `Render` trait, `escape`, `raw`,
  `to_string` and `document`;
- `Framework.Html`'s `scripts()`, which boots the client: put `{Html.scripts()}`
  in a layout and the page imports `/_client/main.js` and calls its `start`;
- `Framework.Response`: `text`, `html`, `view`, `json`, `empty`, `redirect` and
  `not_found`, building `Std.Http` responses;
- a launcher, `launcher/serve.mjs` (see below).

`framework_demo` depends on it, declares an `Accounts` module with a `users`
table in its own `src/`, and a `todos` table in `main.px`. Both are in memory in
Node, so the project has no JavaScript of its own.

## Launchers

A package can provide a launcher, a Node script that runs a project built for
several hosts. A project picks one with `[run]`, and then `polar run` (without
`--host`) builds every host and hands the build to it:

```toml
# the package
[launcher]
script = "launcher/serve.mjs"   # relative to the package

# the project
[run]
launcher = "simple_framework"   # a direct dependency with a [launcher]
options = { port = 3000 }       # passed to the launcher as is
```

`polar run` runs `node <script> <manifest.json>`, from the current directory,
and exits with the launcher's exit code. The manifest is:

```json
{ "version": 1, "project": "web_posts", "root": "/abs/projects/web_posts",
  "out": "/tmp/polar-run-…/dist", "main": "main.js",
  "hosts": { "Browser": "/tmp/…/dist/Browser", "Node": "/tmp/…/dist/Node" },
  "options": { "port": 3000 } }
```

`main` is the project's `main` relative to `src`, as JavaScript, and each host
directory is a complete `polar build` output for that host. `polar run --host
<Host>` still runs that host's `main` as before.

`simple_framework`'s launcher serves, on `options.port` (default 3000) and
`options.hostname` (default `127.0.0.1`):

- the client host's build (`options.client`, default `Browser`) at `/_client/`,
  with the runtime's `http.files`;
- the server host's bridges (`options.server`, default `Node`) at `/_polar/`;
- the server's exported `router` for everything else.

It also exports `serve(manifest)`, which `web_posts/e2e.mjs` uses to start the
same server on port 0.

`polar build` bundles the launcher too: it copies the script to
`dist/_polar/launcher/` and writes `dist/start.mjs`, which builds the manifest
with paths resolved against `dist/` and runs the launcher with it. `polar start`
(or `node dist/start.mjs`) then serves the built app without compiling again.
The script is copied alone, so a launcher must not import files beside it.

## `web_posts`

The M6 full-stack example, split by concern:

- `user.px` declares the hosts and the `users` table, kept in memory in Node
  (`users in Node`);
- `post.px` imports `User` and declares the `posts` table (its `author_id`
  `references users` across modules, by its `Id<User>` type). It keeps the table
  in memory in Node (`posts in Node`) and bridges it to the browser with
  `Posts in Browser from Node`;
- `routes.px` holds the JSON `routes` and their handlers, plus `GET /`, which
  renders `Views.page` on the server;
- `views.px` holds the `views`: a layout, a post card, a list and the page. They
  are pure, so the browser's `start` renders `Views.list` with the same code;
- `main.px` re-exports `Routes.router` and the browser's `start`, which reaches the
  generated `Posts` effect over the bridge and renders `Views.list` into
  `#app`.

Its `polar.toml` picks `simple_framework`'s launcher, so `polar run` in
`web_posts/` serves the app on <http://127.0.0.1:3000>: the page is rendered on
the server and `start` runs in the browser. Its `e2e.mjs` starts the same
server with `serve()`, then lists, creates, fetches, rejects and deletes posts
over HTTP, fetches the client bundle and runs the browser's `start`.
