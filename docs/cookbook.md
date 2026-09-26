# CLI cookbook

Short, copy-paste recipes for common `soroban-forge` workflows. Each recipe is
a working command sequence — verified against `soroban-forge <subcommand>
--help` and, where it doesn't need a live network or a built wasm, run
end-to-end while writing this page. See [cli-reference.md](cli-reference.md)
for the full flag reference on any command here.

## Scaffold and deploy in one go

```sh
soroban-forge new my-token --template token --yes
cd my-token
soroban-forge identity generate deployer --yes
soroban-forge identity fund deployer
soroban-forge deploy --source deployer --network testnet
```

`deploy` builds the release wasm first if it isn't already built. Add
`--dry-run` to print the `stellar` command it would run without submitting
anything — useful for checking what will happen before you spend a
transaction:

```sh
soroban-forge deploy --source deployer --network testnet --dry-run
```

## Regenerate bindings after a change

After editing a contract's entrypoints, rebuild and regenerate its
TypeScript client so callers pick up the new interface:

```sh
stellar contract build
soroban-forge bindings ts --force
```

`--force` overwrites the existing `bindings/typescript/` output; drop it the
first time there's nothing to overwrite yet. Add `--react` to also emit a
typed React hook per entrypoint, or `--watch` to regenerate automatically on
every source change while you iterate:

```sh
soroban-forge bindings ts --react --watch
```

## Verify a mainnet contract

Confirm a deployed contract's wasm matches your local release build,
byte-for-byte, by comparing SHA-256 hashes:

```sh
soroban-forge network add mainnet --rpc-url <YOUR_MAINNET_RPC_URL>
stellar contract build
soroban-forge verify <CONTRACT_ID> --network mainnet
```

Always pass your own `--rpc-url` for `mainnet` explicitly rather than relying
on whatever default the tool ships — see the security note below. Exit code
is `0` on a match, `1` on a mismatch. Add `--reproducible` to build inside the
pinned reproducible-build container first, so the comparison isn't sensitive
to your local toolchain:

```sh
soroban-forge verify <CONTRACT_ID> --network mainnet --reproducible
```

> **Security note:** as of this writing, `soroban-forge network add mainnet`
> without `--rpc-url` fills in a hardcoded default RPC endpoint containing
> what looks like another party's API key
> (`crates/network/src/lib.rs`). Until that's removed upstream, always pass
> your own `--rpc-url` for `mainnet` rather than accepting the built-in
> default.

## Wire forge into an existing repo

Adding soroban-forge to a contract project you didn't scaffold with it:

```sh
cd my-existing-contract
soroban-forge init --tests --ci --yes
```

`init` writes `forge.toml`, and `--tests`/`--ci` additionally run the
equivalent of `test-init`/`ci-init` (below) in one step. Run `soroban-forge
doctor` afterward to confirm your toolchain matches what the project expects.

## Generate a test harness for an existing contract

```sh
soroban-forge test-init --prop --yes
```

Generates fixtures, a smoke test, event assertions, and (with `--prop`) a
property-based invariant test — regenerate coverage for a specific
contract-count without touching hand-written tests with `--force`:

```sh
soroban-forge test-init --force
```

Add `--localnet` for an ignored integration test that deploys and invokes
against a local Soroban network, or `--fuzz` for a `cargo-fuzz` target.

## Add CI workflows for GitHub Actions

```sh
soroban-forge ci-init --deploy --coverage --yes
```

Writes `build-test.yml`, `contract-size.yml`, and (with `--deploy`) a manual
testnet-deploy workflow that expects a `STELLAR_DEPLOYER_SECRET` repository
secret. Pass `--provider gitlab`, `--provider circleci`, `--provider azure`,
or `--provider bitbucket` for a non-GitHub pipeline instead.

## Check and fix your toolchain before scaffolding

```sh
soroban-forge doctor --fix
```

Checks Rust, the `wasm32v1-none` target, `stellar-cli`, and whether your
project's `soroban-sdk` is current; `--fix` runs the suggested `rustup`/
`cargo install` remedies instead of only reporting them. Add `--build` to
also smoke-build the current project as part of the check:

```sh
soroban-forge doctor --build
```

## Optimize wasm size and enforce a budget

```sh
stellar contract build
soroban-forge optimize --max-size 65536 --check
```

Runs `stellar contract optimize` against the release build and reports the
before/after size; `--check` (combined with `--max-size`, or a budget
declared in `forge.toml`) exits non-zero if the optimized wasm is still too
big — wire this into CI to catch a contract that's grown past its size
budget.

## Diff two contract specs to catch breaking changes

```sh
stellar contract build
soroban-forge spec --format json > new-spec.json
soroban-forge spec diff old-spec.json new-spec.json
```

Compares two contract interfaces (built wasm, a deployed contract ID, or a
saved spec file) and reports which entrypoint changes are additive versus
breaking — run it against your previous release's spec before publishing a
new one.
