# Changelog

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
