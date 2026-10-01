# Polar

A statically typed, row-polymorphic language that compiles to JavaScript.

Requires Rust 1.85+ and Node.js.

```sh
cargo build                     # builds target/debug/polar
cargo install --path cli        # or put `polar` on your PATH
cargo test
```

## Using the CLI

```sh
polar run hello.px              # compile a file and run its exported `main`
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

## License

[MIT](LICENSE)
