# soroban-forge-testgen (Module 3)

Test harness generator. **Owner: Person C.**

Implements the `soroban-forge test-init` subcommand: point it at an existing
Soroban contract project and it generates

| file                   | contents                                                             |
|------------------------|----------------------------------------------------------------------|
| `tests/common/mod.rs`  | fixtures: mocked-auth `Env`, account generator, ledger-time control, token (SAC) setup + funding, snapshot assertion helper |
| `tests/forge_smoke.rs` | smoke test registering the detected `#[contract]` type and constructing its client |
| `tests/forge_invariant.rs` | proptest-based invariant testing harness asserting state properties across random call sequences |
| `tests/forge_init_once.rs` | (when an initialize-style entrypoint is detected) asserts a second call to it is rejected |
| `tests/forge_budget.rs` | (with `--budget`) benchmark measuring one entrypoint's CPU instructions and memory via `env.cost_estimate().budget()`, asserting an upper bound |
| `tests/forge_upgrade.rs` | (when an upgrade entrypoint is detected) writes state, upgrades the contract, and asserts the state survived |
| `tests/forge_roundtrip.rs` | (when an entrypoint takes an `Option`, `Vec` or `Map`) passes empty, single- and multi-element values through it |
| `benches/forge_bench.rs` | (with `--bench`) criterion benchmarks, one per entrypoint, for tracking cost over time |
| `fuzz/Cargo.toml`      | (with `--fuzz`) cargo-fuzz workspace manifest |
| `fuzz/fuzz_targets/fuzz_target_1.rs` | (with `--fuzz`) property-based fuzzer feeding arbitrary values into detected contract methods |
| `tests/forge_storage_isolation.rs` | (when 2+ of instance/persistent/temporary storage are used) asserts the same key written to each storage type stays independent |

Pass `--prop` (or `--invariant`/`--property`) to `test-init` to generate the
property-based invariant harness, and `--fuzz` to emit a cargo-fuzz target.

`--budget [ENTRYPOINT]` emits the budget test, measuring the named entrypoint or
the first one detected. It starts at Soroban's per-transaction ceilings (100M
CPU instructions, 40 MiB); run `cargo test --test forge_budget -- --nocapture`
to see the real cost and tighten the constants so a regression fails the test.

`--bench` emits criterion benchmarks under `benches/` and adds the `[[bench]]`
target to `Cargo.toml`. Where the budget test gates a ceiling, these track cost
over time — see [docs/testing-guide.md](../../docs/testing-guide.md#benchmarks-criterion).

> `--bench` was previously an alias for `--budget`. It now selects criterion
> benchmarks; use `--budget` for the ceiling test.

`--contract <name>` targets one member of a multi-contract workspace, matched by
package name, crate name or directory. A workspace with more than one contract
and no `--contract` stops and lists the candidates rather than generating a
harness for every member. Single-contract projects are unaffected.

`tests/forge_upgrade.rs` needs no flag. It is written when the contract exposes
an upgrade entrypoint — one named `upgrade`, `upgrade_contract`, `set_wasm` or
`migrate`, or any method taking a `BytesN<32>` argument whose name mentions
wasm. It writes state, upgrades, and asserts the state survived. The test ships
`#[ignore]`d because a real migration test needs a second wasm; the generated
file documents how to point it at one.

`tests/forge_roundtrip.rs` needs no flag either. It is written when an
entrypoint takes an `Option`, `Vec` or `Map`, and covers the empty, single- and
multi-element cases for each — the empty case especially, where the host and the
contract can disagree about the encoding and the call returns a plausible wrong
value instead of failing.

`tests/forge_init_once.rs` needs no flag: it is written whenever the contract
exposes an entrypoint named `initialize`, `initialise`, `init` or `setup`. It
calls the entrypoint, then asserts `try_<entrypoint>` returns `Err` on a second
call. A failure means the contract has no re-initialization guard — a real
finding, not a flaky test.

`tests/forge_storage_isolation.rs` needs no flag either. It is written when
the contract uses 2 or more of instance/persistent/temporary storage, and
writes the same key to each, asserting a value read back from one storage
type never leaks in from another — a common source of subtle bugs.

`--actors <N>` (default `3`) controls the size of the multi-user fixture
generated in `tests/common/mod.rs`: a `pub struct Actors` with one named
`Address` field per identity (`admin`, `alice`, `bob`, ... — see
`ACTOR_NAMES`; past the pool, identities are named `actor_N`) and a
`pub fn actors(env: &Env) -> Actors` that generates them. Like the rest of
`tests/common/mod.rs`, it's written once and reusable across every generated
test file.

`--fail-on-uncovered` turns the coverage summary (printed after every run)
into a non-zero exit when any contract entrypoint has no generated test
touching it. Without the flag, the summary is informational only — it never
fails the run. The summary — covered/uncovered entrypoint names — is also
available under `entrypoint_coverage` in `--json` output.

`--update-snapshots` runs the generated snapshot tests
(`cargo test --test forge_snapshots`) with `FORGE_UPDATE_SNAPSHOTS=1`, so any
changed or missing golden file under `tests/snapshots/` is (re)written, then
reports which files changed — the sanctioned way to accept a snapshot change
instead of hand-editing the `.snap` files. Without it, a snapshot mismatch
still fails `cargo test` as usual. Changed files are also available under
`updated_snapshots` in `--json` output.

The global `--quiet` flag suppresses the generated-file report and follow-up
notes without changing which harness files are written.

## How detection works

`detect.rs` inspects the target without heavy parsing:

- `Cargo.toml` → package name, whether dev-dependencies enable soroban-sdk's
  `testutils` feature (warns if not).
- `src/lib.rs` → the struct annotated with exactly `#[contract]`, and whether
  a `__constructor` exists. Contracts with constructors get an `#[ignore]`d
  smoke test with a TODO, since constructor arguments can't be guessed. Also 
  detects method names and their parameters (ignoring `env: Env`) to construct
  `FuzzInput` enums when generating the fuzzer.

## Snapshot helper

`assert_snapshot(name, &value)` compares `value`'s `Debug` output against
`tests/snapshots/<name>.snap`. First run writes the snapshot; subsequent runs
fail on change; `FORGE_UPDATE_SNAPSHOTS=1 cargo test` accepts changes, or run
`soroban-forge test-init --update-snapshots` to do the same and see a report
of what changed without setting the env var by hand.

## Public surface

```rust
testgen::generate(dir, force, fuzz) -> Result<(ContractInfo, Vec<&str>)>;
testgen::generate_with(dir, &GenerateOptions) -> Result<(ContractInfo, Vec<&str>)>;
testgen::inspect(dir) -> Result<ContractInfo>;
testgen::build_budget_test(&info, entrypoint) -> Result<String>;
testgen::build_init_once_test(&info) -> String;
testgen::build_upgrade_test(&info) -> String;
testgen::build_bench(&info) -> String;
testgen::ensure_bench_target(&manifest) -> Option<String>;
testgen::write_bench_files(dir, &info, force) -> Result<Vec<&str>>;
testgen::build_roundtrip_tests(&info) -> String;
testgen::upgrade::detect_upgrade_entrypoint(&methods) -> Option<UpgradeEntrypoint>;
testgen::containers::find_container_args(&methods) -> Vec<ContainerArg>;
testgen::candidates(root, &members) -> Vec<Candidate>;
testgen::resolve(requested, &candidates) -> Result<Selection>;
testgen::detect::detect_init_method(&methods) -> Option<MethodInfo>;
testgen::build_actors_fixture(count) -> String;
testgen::build_storage_isolation_test(&info) -> String;
testgen::entrypoint_coverage(&info, dir, &written) -> (Vec<String>, Vec<String>);
testgen::update_snapshots(dir) -> Result<Vec<String>>;
```

`GenerateOptions` carries `force`, `fuzz`, `budget`, `budget_entrypoint`,
`actor_count` and `fail_on_uncovered`; `generate` is the two-flag shorthand
for it.

## Tests

`cargo test -p soroban-forge-testgen` — includes end-to-end tests that run the
generator against freshly scaffolded `hello-world` and `token` projects.
