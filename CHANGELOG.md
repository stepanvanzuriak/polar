# Changelog

## 0.4.0

### Major changes

#### Native executables

`polar build --target native` writes one file that runs on a machine without Node.

- The executable is a copy of `polar-rt`, a Rust evaluator for the typed program, with the
  program attached to the end of the file
- `polar run --target native` builds and runs in one step
- `--triple <TRIPLE>` picks the platform (the runtime for it is looked up as
  `polar-rt-<TRIPLE>` in `POLAR_RT_DIR`) and `-o`/`--output <FILE>` names the file
- `--watch` and `--emit` are not supported with `--target native`
- The standard library bindings for `Time`, `Url`, `Regex`, `Crypto`, `Fs` and `Process`
  are built in. `Dom`, the browser host and user-written bindings are not available
- An executable that serves an effect to browser bundles starts with `--serve <PORT>`, with
  the same wire format as the Node bridge server
- The two backends follow one written definition of `Int`, `Float` printing, string length and
  JSON numbers, and one conformance suite runs on both

#### Workspaces

- A `polar.toml` with `[workspace] members = ["tests/*", "examples/blog"]` and
  an optional `exclude`. `polar check`, `polar build`, `polar fmt` and `polar test` at the
  root act on every member. A root with `[workspace]` has no `[project]` or `[package]`
- `polar test -p <NAME>` picks workspace members by project name, and `-j`/`--jobs <N>` runs
  members in parallel. The report ends with a `workspace result:` line

#### Testing

- `polar test --format json` prints one JSON object per test and a final result line
- Tests that expect a compile error: a `// expect-error: POLAR0504 "no action `show`"` line
  applies to the next line of code, and the file passes when the errors match. Such files are
  left out of `polar build`, `polar check` and `--watch`
- `polar test --bless` (or `POLAR_UPDATE=1`) rewrites `expect-error` annotations and snapshots
  from what actually happened
- `polar fmt --expect <FILE>` compares formatted output with a golden file, and
  `polar fmt --check --idempotent` formats twice and fails if the second pass changes anything
- `[fmt] exclude = ["tests/*/fmt"]` in `polar.toml` leaves paths out of `polar fmt`
- `test_*` functions in a `*_test.px` file are exported automatically, so `exports` there is
  optional
- A test file may define `around_each(run)`; every test in the file runs as `around_each(test)`.
  It must take one parameter (POLAR0823)
- A test's stderr is merged into its stdout, so a passing test prints nothing

### Breaking changes

- **`Int` is exact up to ±(2^53 − 1) on every backend.** `+`, `-` and `*` whose result leaves
  that range are a runtime error (`integer overflow`) instead of a silent rounding, and an
  integer literal beyond it is a compile error. `/` and `%` by `0` are a runtime error that
  names the location (`division by zero (src/main.px:4:9)`); `/` on `Float` is still IEEE
- `polar fmt` skips `X.px` when `X.expected.px` sits next to it, and skips `*.expected.px` itself,
  unless the file is named on the command line

### Language

- Trait constraints on effect operations: `get(key: String) -> Option<a> where Json<a>`. The
  `binds` that implements the operation can then use what the trait gives it
- `Ord` and `Num` are Prelude traits. `where Ord<a>` allows `<`, `<=`, `>` and `>=` on `a`, and
  `where Num<a>` allows `+`, `-`, `*`, `/`, `%` and unary `-`. `Int`, `Float` and `String`
  implement `Ord`; `Int` and `Float` implement `Num`. Code on concrete types compiles to the
  same JavaScript as before
- Inferred effect rows: `f() -> Response / {..}` keeps the written return type and infers the
  row from the body, and `/ {.., Async}` adds labels to what is inferred. A signature with `->`
  and no row is still a promise of purity
- Module groups in `uses`: `uses Ticket.{Conn, Response}` imports each module under its last
  name
- An extern or a `binds` entry can name an implementation per target:
  `= js "./bindings/Crypto.js" digest_sha256, rs "crypto::sha256"`, and
  `Random in Native = rs "crypto"`. The plain `"./file.js" name` form still means JavaScript
- `Native`, a built-in host for the native runtime
- A missing implementation for the target being built is POLAR0824

### Standard library

- `Std.Test`: `capture` (what `Log` printed meanwhile), `mock` (replace the operations of an
  effect for the length of a body), `assert_snapshot` and `assert_snapshot_with` (golden files
  under `snapshots/`), `with_app` and `with_app_env` (serve the project's app on a free port),
  `send`, `get` and `request` (an HTTP client), and `session`, `go` and `go_follow` (a client
  that keeps cookies)
- `Std.Assert.equal_text` fails with a line diff of only the lines that changed
- New `Std.Algo` with `diff`, a line diff built on the longest common subsequence
- `Std.Math`: `max`, `min` and `abs` work on `Float` and `String` too, where they have `Ord`
  and `Num`
- `Std.List`: `take` and `drop`
- `Std.Regex`: `captures`
- `Std.Fs`: `mkdtemp` and `with_temp_dir`
- `Std.Process`: `scoped_env`, and `spawn`, `wait_for_line` and `stop` for a child process
- `Std.Json`: numbers are printed with their own text, so `3` stays `3`

## 0.3.0

### Breaking changes

- **Types are private unless listed in `exports`.** Before, every type was exported and listing
  one was an error (POLAR0301). Now listing a type exports it with its constructors, and an
  exported function, constant or effect whose type mentions a private type is an error
  (POLAR0322). Types generated by a zone plugin are still exported automatically. To migrate,
  add the types other modules use to `exports`
- **The `Password` effect is gone from `Std.Crypto`.** `hash_password` is now `/ {Random, Async}`
  and `verify_password` is `/ {Async}`. Remove `Password` from signatures and from any
  `binds` that named it
- **`polar test` no longer runs on `node:test`.** `polar test -- --test-name-pattern=...` stops
  working; use `polar test --filter <TEXT>`. Output now follows the new report, which adds the
  `ERROR` and `TIMEOUT` outcomes
- **`return` is a keyword**, so it can no longer be used as a name
- `Time.from_iso` no longer accepts a tab, newline or other non-space whitespace between the
  date and the time

### Language

- `return` leaves the innermost function or lambda; a bare `return` gives `{}`
- `if` without `else` when the branch is `{}`, such as one that ends in `return` or `throw`
- Match guards: `pattern if condition -> ...`
- Or-patterns: `A | B`, nested too, as long as every alternative binds the same names
- Matching several values at once: `match a, b { p, q -> ... }`
- Calling a function whose effect row ends in the same row variable as the caller's no longer
  needs the rows to be equal; the callee's labels only have to be in the caller's row
- `Async`, a built-in effect label like `Mut`: an extern declared `/ {Async}` is awaited, and a
  function that calls it becomes `async`, with no effect declaration or binding
- An extern can be listed in `exports` directly

### Tooling

- `polar test` is now a Polar test runner: `Std.Test` runs the tests, and `node:test` is no
  longer used. A test can pass, fail, error (an uncaught error of any kind) or time out, and a
  failing test never stops the run
- `polar test --filter <TEXT>` runs the tests whose name contains the text, and
  `--timeout <MS>` sets the per-test deadline (5000 by default). `--test-name-pattern` no longer
  works
- The compiler decides what a test is: a `test_*` function in a `*_test.px` file must take no
  parameters (POLAR0821), and an exported function in a test file that doesn't start with
  `test_` is a warning (POLAR0822). `polar test` also writes `_polar/tests.json` for editors
- Output a test prints is shown only when the test doesn't pass; failures point at the `.px`
  file and line
- `polar run --watch` rebuilds and restarts on every change, and `--debounce <MS>` sets the wait
  after a change (100 by default)
- `polar repl` imports modules that live in folders, such as `Controllers.PagesController`

### Standard library

- New `Std.Test`
- `Std.Crypto`: password hashing with scrypt. `hash_password`, `verify_password` and
  `needs_rehash` use the PHC string format, and `verify_password` never throws
- `Std.Crypto`: the `Password` effect is gone. `hash_password` is `/ {Random, Async}` and
  `verify_password` is `/ {Async}`
- `Std.Time`: formatting and parsing ISO times is written in Polar
- `String.split_once` and `String.chars`
- `Map.from_list`

## 0.2.0

### Tooling

- Git dependencies: `{ git = "github.com/owner/repo", version = "v1.2.3" }` (or `rev` / `branch`)
  in `[dependencies]`, with `polar add`, `remove`, `fetch [--verify]` and `update`
- `polar.lock` pins every package to a commit and a sha-256 checksum; packages are cached
  once per commit under `~/.polar/pkg/` (`POLAR_HOME`), and `--offline` / `POLAR_OFFLINE=1`
  forbid network access
- Go-style minimal version selection: the highest requested version of a package wins; a
  `v2+` major lives at an address ending `/v2`
- `[package] polar = "0.2.0"` declares the Polar version a package was written for; importing
  one for an older or newer Polar warns

## 0.1.0

The first release.

### Language

- Statically typed, row-polymorphic language that compiles to JavaScript
- Records, variants and pattern matching with exhaustiveness checks
- Traits, effects (`/ {Effect}`) and hosts (`Browser`, `Node`)
- Bridge declarations and JavaScript interop with `.d.ts` emission
- Bitwise operators, parameter destructuring, tail calls
- Zones, including zone plugins written in Rust (`migrations`, `routes`, `views`, ...)
- Contextual zone keywords

### Tooling

- `polar run`, `check`, `build`, `fmt`, `start`, `init`, `test` and `repl`
- `--watch`, `--host` and `--emit` for compiler stages
- Projects and packages through `polar.toml`
- Source maps and a formatter that preserves comments
- VS Code grammar in `editor/vscode`
- Hint for misspelled record fields

### Standard library

`Assert`, `Crypto`, `Dom`, `Fs`, `Http`, `Id`, `Json`, `List`, `Map`, `Math`, `Option`,
`Path`, `Prelude`, `Process`, `Ref`, `Regex`, `Result`, `Table`, `Time` and `Url`

### Project

- Website and documentation
- CI and releases for Linux and macOS (x86_64 and aarch64), with an install script
