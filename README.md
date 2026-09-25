# soroban-forge

**Scaffolding, test-harness and CI toolkit for [Soroban](https://developers.stellar.org/docs/build/smart-contracts) smart contracts on Stellar** — think `create-react-app` for Soroban development.

`soroban-forge` wraps and complements the official [stellar-cli](https://github.com/stellar/stellar-cli); it never reimplements it. Building and deploying always go through `stellar contract build` / `stellar contract deploy` — forge gets you to that point faster:

- `soroban-forge new` — start from a working, tested contract template
- `soroban-forge init` — add forge configuration to an existing contract
- `soroban-forge test-init` — generate fixtures, a smoke test and a snapshot helper for an existing contract
- `soroban-forge ci-init` — add CI workflows for GitHub, GitLab, CircleCI or Bitbucket (build+test, contract-size check, optional testnet deploy)
- `soroban-forge doctor` — verify your toolchain and get fix instructions
- `soroban-forge bindings ts` — generate a TypeScript client package from a built contract
- `soroban-forge verify <contract-id>` — check that a deployed contract matches your local build
- `soroban-forge deploy` — build (if needed) and deploy the contract, printing its contract ID
- `soroban-forge invoke <contract-id> <fn> [args...]` — call a function on a deployed contract and print the result

## Quickstart

[![asciinema cast](https://asciinema.org/a/soroban-forge-zero-to-testnet.svg)](https://asciinema.org/a/soroban-forge-zero-to-testnet)

You need Rust ≥ 1.84 ([rustup](https://rustup.rs)) and `git`. The two remaining
pieces — the `wasm32v1-none` target and `stellar-cli` — are what `doctor --fix`
installs in step 2.

```sh
# 1. install soroban-forge (from source, v0.1)
git clone https://github.com/soroban-forge-labs/soroban-forge
cd soroban-forge && cargo install --path . && cd ..

# 2. install anything missing from your toolchain, then re-check
#    (--fix prompts before running each install; drop it to only report)
soroban-forge doctor --fix

# 3. create a project (`soroban-forge templates` lists all six)
soroban-forge new my-token --template token
cd my-token

# 4. build the deployable wasm -> target/wasm32v1-none/release/my_token.wasm
stellar contract build

# 5. run the tests — the template passes them out of the box
cargo test
```

Step 5 should end in `test result: ok. 6 passed; 0 failed`. From here,
`soroban-forge test-init --force` adds a generated test harness with fixtures
and a snapshot helper, and `soroban-forge ci-init --deploy` writes GitHub
Actions workflows for build+test, contract size and manual testnet deploys.

New to Soroban entirely? Follow the full walkthrough:
[docs/tutorial-zero-to-testnet.md](docs/tutorial-zero-to-testnet.md).

Hitting an error? Check the
[troubleshooting / FAQ](docs/troubleshooting.md) page first.

## Commands

| command                          | what it does                                              |
|----------------------------------|-----------------------------------------------------------|
| `new <name> --template <t>`      | scaffold a project (`--list-templates` to see options)    |
| `init [--tests] [--ci]`         | configure an existing contract without creating a crate  |
| `templates`                      | list the bundled templates with a one-line description    |
| `test-init`                      | generate fixtures + smoke/TTL tests; `--layout inline` puts them in `src/` |
| `ci-init --provider github`      | write CI workflows (`github`, `gitlab`, `circleci`, `bitbucket`); `--deploy` adds manual testnet deploy, `--matrix` a toolchain matrix |
| `doctor`                         | check rustc/cargo, `wasm32v1-none` target, stellar-cli    |
| `bindings ts [--react]`          | generate a TypeScript client package from a built contract wasm; `--react` adds typed hooks |
| `bindings-py`                    | generate a typed Python client (`client.py`) from a built contract wasm |
| `spec`                           | print the contract interface — entrypoints with their argument and return types — from the built wasm (`--json` for machine-readable output) |
| `verify <contract-id>`           | compare a deployed contract's wasm hash with the local release build (exit `1` on mismatch) |
| `deploy --source <identity>`     | build (if needed) and deploy the contract, printing its contract ID |
| `invoke <contract-id> <fn> [args...]` | call a function on a deployed contract and print the result |


Global `--log-file <path>` writes structured JSON-lines diagnostics in addition
to normal output, which is useful when retaining CI debugging artifacts. Pass
`--offline` to prohibit remote template clones, connectivity checks, friendbot,
verification fetches, and any other network-capable operation.

All commands read an optional [`forge.toml`](crates/core/src/config.rs) in the
project directory (name, authors, default template) — generated projects get
one automatically.

## Architecture

Five modules, five owners, minimal merge conflicts. Each module is a crate
with its own README, tests and a small public surface; they meet only at the
`ForgePlugin` trait defined in core:

| module | crate | subcommand |
|--------|-------|------------|
| 1 — CLI core & framework | [`crates/core`](crates/core) | (routing, config, errors) |
| 2 — Scaffolding & templates | [`crates/scaffold`](crates/scaffold) + [`templates/`](templates) | `new` |
| 3 — Test harness generator | [`crates/testgen`](crates/testgen) | `test-init` |
| 4 — CI/CD presets | [`crates/ci-presets`](crates/ci-presets) + [`presets/`](presets) | `ci-init` |
| 5 — Docs & DX | [`crates/doctor`](crates/doctor) + [`docs/`](docs) + [`examples/`](examples) | `doctor` |
| 6 — TypeScript bindings | [`crates/bindings-ts`](crates/bindings-ts) | `bindings ts` |
| 7 — Deployment verification | [`crates/verify`](crates/verify) | `verify` |
| 8 — Contract interface dump | [`crates/spec`](crates/spec) | `spec` |
| 9 — Python bindings | [`crates/binding-py`](crates/binding-py) | `bindings-py` |

> **Note:** See [`examples/README.md`](examples/README.md) for instructions on
> regenerating the checked-in example projects.


See [CONTRIBUTING.md](CONTRIBUTING.md) for the ownership map and how to pick
up an issue — [ISSUES.md](ISSUES.md) lists well-scoped starter work. All
contributors are expected to follow the [Code of Conduct](CODE_OF_CONDUCT.md).

## Requirements

- Rust ≥ 1.84 with the `wasm32v1-none` target
- [stellar-cli](https://developers.stellar.org/docs/tools/cli/stellar-cli) for building/deploying contracts
- Generated contracts use [soroban-sdk](https://crates.io/crates/soroban-sdk) 26.x

`soroban-forge doctor` checks all of this for you.

## Privacy

`soroban-forge` collects **no telemetry**: no usage analytics, crash reports,
identifiers, command arguments, or project contents are sent to the maintainers
or an analytics provider. Network requests made for explicitly requested
features are not telemetry and can be disabled with `--offline`. Any future
telemetry must be explicitly opt-in, disabled by default, fully documented, and
revocable; upgrades will never silently enable it. See [Privacy and
Telemetry](docs/privacy.md).

## Exit codes

`soroban-forge` uses a small set of stable exit codes (`0` success, `1`
user error, `2` missing tool, `3` internal error) so CI/scripts can branch
on outcome — see [docs/exit-codes.md](docs/exit-codes.md).

## Changelog

See [CHANGELOG.md](CHANGELOG.md) for a full history of notable changes and release notes.

## License

[Apache-2.0](LICENSE)