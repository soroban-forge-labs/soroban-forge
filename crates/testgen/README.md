# soroban-forge-testgen (Module 3)

Test harness generator. **Owner: Person C.**

Implements the `soroban-forge test-init` subcommand: point it at an existing
Soroban contract project and it generates

| file                   | contents                                                             |
|------------------------|----------------------------------------------------------------------|
| `tests/common/mod.rs`  | fixtures: mocked-auth `Env`, account generator, ledger-time control, token (SAC) setup + funding, snapshot assertion helper |
| `tests/forge_smoke.rs` | smoke test registering the detected `#[contract]` type and constructing its client |

## How detection works

`detect.rs` inspects the target without heavy parsing:

- `Cargo.toml` → package name, whether dev-dependencies enable soroban-sdk's
  `testutils` feature (warns if not).
- `src/lib.rs` → the struct annotated with exactly `#[contract]`, and whether
  a `__constructor` exists. Contracts with constructors get an `#[ignore]`d
  smoke test with a TODO, since constructor arguments can't be guessed.

## Snapshot helper

`assert_snapshot(name, &value)` compares `value`'s `Debug` output against
`tests/snapshots/<name>.snap`. First run writes the snapshot; subsequent runs
fail on change; `FORGE_UPDATE_SNAPSHOTS=1 cargo test` accepts changes.

## Public surface

```rust
testgen::generate(dir, force) -> Result<(ContractInfo, Vec<&str>)>;
testgen::inspect(dir) -> Result<ContractInfo>;
```

## Tests

`cargo test -p soroban-forge-testgen` — includes end-to-end tests that run the
generator against freshly scaffolded `hello-world` and `token` projects.
