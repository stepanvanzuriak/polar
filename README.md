# Polar

A statically typed, row-polymorphic language that compiles to JavaScript.

> Experimental (0.2.0): expect breaking changes before 1.0.

Requires Node.js.

```sh
curl -fsSL https://polar-lang.vercel.app/install.sh | sh   # latest release
```

To build from source you need Rust 1.85+:

```sh
cargo build                     # builds target/debug/polar
cargo install --path cli        # or put `polar` on your PATH
cargo test
```

## Using the CLI

```sh
polar run hello.px              # compile a file and run its exported `main`
polar run tool.px -- a b        # args after `--` reach the program as `Process.args()`
polar check src                 # report errors, write nothing
polar build src --out dist      # write .js, .js.map and .d.ts into dist/
polar fmt src                   # format in place (`--check` to only verify)
```

`--watch` rebuilds on every change (`build` and `check`), `--host <Host>` picks
which host to build for, and `--emit=tokens|ast|core|types|hosts|js` prints one
compiler stage for a single file.

### Projects

A directory with a `polar.toml` is a project:

```toml
[project]
name = "app"
src = "src"             # default
out = "dist"            # default
main = "src/main.px"    # default
hosts = ["Browser", "Node"]   # optional: build one bundle per host
```

Inside a project the path arguments are optional:

```sh
polar build                     # → dist/ (or dist/<Host>/ per host)
polar run                       # compile and run `main`
polar start                     # run the existing build, without compiling
polar start --host Node -- arg  # pick a host's build; args after `--` go to the app
```

`dist/` is self-contained: `node dist/start.mjs` runs it anywhere. See
[`projects/`](projects/) for examples.

### Dependencies

A project or package lists what it needs under `[dependencies]`, either from disk or
straight from a git repository (any host; no registry):

```toml
[dependencies]
local = { path = "../local" }
ticket = { git = "github.com/owner/ticket", version = "v0.2.0" }   # a tag
tool = { git = "github.com/owner/tool", branch = "main" }          # locked to a commit
pinned = { git = "github.com/owner/pinned", rev = "<40-char sha>" }
```

```sh
polar add github.com/owner/ticket       # newest tag; edits polar.toml, writes polar.lock
polar add github.com/owner/tool@v1.4.1  # or @branch, or @<commit>
polar fetch                             # resolve and download; --verify re-checks checksums
polar update [name]                     # move to the newest tags
polar remove name
polar --offline check                   # never touch the network (or POLAR_OFFLINE=1)
```

`polar.lock` pins every package in the graph to a commit and a sha-256 checksum; commit it.
Builds use the lock and fetch what it names when the cache is cold. Where two packages
want different versions of one dependency, the highest requested wins (Go's minimal
version selection); a `v2+` major lives at an address ending `/v2`. Packages are cached
once per commit under `~/.polar/pkg/` (`POLAR_HOME` overrides). Needs `git` on `PATH`.

### Command-line programs

A `Node` program reads its arguments and environment, sets its exit code and
runs other programs through `Std.Process`:

```polar
uses
  Std.List
  Std.Process

hosts
  Node

functions
  main() -> {} / {Process} {
    match Process.args() {
      ["build", ..rest] -> {
        let code = Process.run("polar", ["build", ..rest], Process.inherit())

        Process.set_exit_code(code)
      }
      _ -> Process.set_exit_code(2),
    }
  }

exports
  main
```

`polar run -- a b`, `polar start -- a b` and `node dist/start.mjs -- a b` all
pass `["a", "b"]`. A launcher sees them after a `--` in its own `process.argv`.
`set_exit_code` sets the code the process exits with once `main` returns; an
uncaught error exits 1. `run` streams the child's output and returns its exit
code, `output` captures `{ code, stdout, stderr }`, and a command that can't be
started gives 127. `Process.inherit() |> Process.in_dir("sub") |>
Process.with_env("NAME", "value")` adjusts where and with what it runs.

`Std.Fs` reads and writes text files. Operations that can fail return
`Result<FsError, _>`, where `FsError` is `{ code, path, message }` and `code` is
Node's (`ENOENT`, `EEXIST`, …). `Fs.list` and `Fs.walk` return sorted names.
`Std.Path` (`join`, `dirname`, `basename`, `extension`, `normalize`,
`relative`) is pure Polar and works on any host.

## Standard library

`uses Std.<Name>` imports a module from [`std/`](std/):

| Module | What |
|---|---|
| `Assert` | `assert` |
| `Dom` | the `Browser` host and its `Dom` effect |
| `Fs` | the `Node` host's `Fs` effect: `read`, `write`, `append`, `exists`, `is_dir`, `mkdir_all`, `list`, `walk`, `remove`, `remove_all` |
| `Http` | `Request`, `Response`, `Header`, `header` lookup |
| `Id` | typed ids, `Id<a>` |
| `Json` | the `Json` trait, `encode`/`decode` |
| `List`, `Map`, `Option`, `Result` | collections and their combinators |
| `Math` | `pi` |
| `Path` | POSIX path functions: `join`, `dirname`, `basename`, `extension`, `normalize`, `is_absolute`, `relative` |
| `Prelude` | `Eq` and `Show` (always in scope) |
| `Process` | the `Node` host's `Process` effect: `args`, `env`, `cwd`, `set_exit_code`, `run`, `output` |
| `Ref` | mutable cells under the `Mut` effect |
| `Table` | in-memory tables |
| `Url` | `decode`/`encode` (percent-encoding, never throws), `parse_query`/`build_query` |

## License

[MIT](LICENSE)
