# Contributing to soroban-forge

Thanks for helping! This project is deliberately structured so five people (or
five hundred) can work in parallel with minimal merge conflicts.

Everyone taking part is expected to follow the
[Code of Conduct](CODE_OF_CONDUCT.md).

## Module ownership map

| module | paths | owner | subcommand |
|--------|-------|-------|------------|
| 1 — CLI core & command framework | `crates/core`, `src/main.rs` | Person A | routing, `forge.toml`, errors |
| 2 — Project scaffolding & templates | `crates/scaffold`, `templates/` | Person B | `new` |
| 3 — Test harness generator | `crates/testgen` | Person C | `test-init` |
| 4 — CI/CD & DevOps presets | `crates/ci-presets`, `presets/` | Person D | `ci-init` |
| 5 — Docs, examples & DX | `crates/doctor`, `docs/`, `examples/` | Person E | `doctor` |
| 6 — TypeScript bindings generator | `crates/bindings-ts` | Person F | `bindings ts` |
| 7 — Deployment verification | `crates/verify` | Person G | `verify` |
| 8 — Contract interface dump | `crates/spec` | Person H | `spec` |
| 9 — Python bindings generator | `crates/binding-py` | Person I | `bindings-py` |

Modules 6 and 9 share the `module:bindings` GitHub label (see
`.github/labeler.yml`) — they're two crates and two subcommands, but one
conceptual "bindings generators" module for labelling purposes.

Rules of the road:

- **Stay inside your module.** The only shared code is `crates/core`; changes
  there need sign-off from Person A (they affect everyone).
- Each module keeps its **own README** (what it owns, its public surface, how
  to test it) and its **own tests**.
- Modules never depend on each other — only on `soroban-forge-core`. The one
  exception: testgen's *dev*-dependency on scaffold, used to test against
  freshly generated projects.

## How the plugin interface works

A module is a crate exposing one type that implements `ForgePlugin`
(see `crates/core/src/plugin.rs`):

```rust
pub trait ForgePlugin {
    fn name(&self) -> &'static str;      // subcommand name
    fn command(&self) -> clap::Command;  // clap definition
    fn run(&self, matches: &ArgMatches, ctx: &ForgeContext) -> Result<()>;
}
```

Adding a whole new module = new crate + one `Box::new(...)` line in
`src/main.rs`.

## Development

```sh
cargo build --workspace
cargo test --workspace
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings
```

End-to-end check (what CI runs):

```sh
cargo run -- new demo --template token
cd demo && cargo test && cd .. && rm -rf demo
```

Working on templates or presets? They are embedded into the binary at compile
time (`include_dir`), so just edit the files under `templates/` or `presets/`
and rebuild. Do **not** rename `Cargo.toml.hbs` files to `Cargo.toml` — cargo
would treat the template as a real package.

## Ground rules

- **Never invent Soroban/Stellar APIs.** Check the
  [official docs](https://developers.stellar.org) or docs.rs for
  [soroban-sdk](https://docs.rs/soroban-sdk); if unsure, mark it
  `// TODO(verify)` and say so in the PR.
- soroban-forge **wraps official tooling** (stellar-cli), it never
  reimplements building/deploying.
- Generated output must work out of the box: templates compile and pass
  tests, workflows pass `actionlint`.
- No secrets in code or generated files — workflows reference GitHub secrets
  only.
- Plugins report failures by returning `Err(ForgeError::...)`, never by
  calling `std::process::exit` themselves — the binary derives the process
  exit code from the error variant (see
  [docs/exit-codes.md](docs/exit-codes.md)), and a plugin exiting directly
  would bypass that.

## Picking up work

[ISSUES.md](ISSUES.md) lists scoped starter issues per module, tagged
`trivial` / `medium` / `high`. Comment on the GitHub issue to claim it. PRs
should include tests and update the owning module's README when the public
surface changes.

New issues go through the bug report and feature request forms, which ask
for the module and apply its `module:*` label automatically. The PR template
has the checklist reviewers expect: tests, module README updates, docs and a
`CHANGELOG.md` entry under `[Unreleased]`.

## Releasing

Maintainers: the full release sequence (version bump, changelog cut,
tagging and publishing) and which parts are automated is documented in
[docs/releasing.md](docs/releasing.md).

## License

By contributing you agree your contributions are licensed under
[Apache-2.0](LICENSE).
