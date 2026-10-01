## Title

Closes:

## What changed

## Language surface

- [ ] Syntax or formatter (`polar fmt` output changes)
- [ ] Type checker, effects or hosts (new or changed diagnostics: list their codes)
- [ ] Codegen or runtime (`runtime/runtime.js`, emitted JS, `.d.ts`)
- [ ] Std (`std/*.px`, `std/bindings/`, `compiler/src/stdlib.rs` `MODULES`)
- [ ] CLI or `polar.toml`
- [ ] Packages, plugins or launchers
- [ ] Editor grammar (`editor/vscode`)

## Checklist

- [ ] `cargo test` passes, including the `projects/` e2e tests
- [ ] `cargo clippy --all-targets` is clean
- [ ] `cargo fmt --check` and `cargo polar-fmt` leave nothing to change
- [ ] Snapshots and `*.expected.txt` changes are deliberate, and explained above
- [ ] New diagnostics have a code in `shared/codes.rs`, a message and a `help` where it helps
- [ ] Docs updated where the surface changed (README, its std table, the ticket)

## Notes for reviewers

<!-- Risks, follow-ups, what you didn't do and why. -->
