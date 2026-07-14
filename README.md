# soroban-forge

**Scaffolding, test-harness and CI toolkit for [Soroban](https://developers.stellar.org/docs/build/smart-contracts) smart contracts on Stellar** — think `create-react-app` for Soroban development.

`soroban-forge` wraps and complements the official [stellar-cli](https://github.com/stellar/stellar-cli); it never reimplements it. Building and deploying always go through `stellar contract build` / `stellar contract deploy` — forge gets you to that point faster:

- `soroban-forge new` — start from a working, tested contract template
- `soroban-forge test-init` — generate fixtures, a smoke test and a snapshot helper for an existing contract
- `soroban-forge ci-init` — add GitHub Actions workflows (build+test, contract-size check, optional testnet deploy)
- `soroban-forge doctor` — verify your toolchain and get fix instructions

## Quickstart

```sh
# 1. install (from source, v0.1)
git clone https://github.com/soroban-forge-labs/soroban-forge
cd soroban-forge && cargo install --path .

# 2. check your environment
soroban-forge doctor

# 3. create a project (templates: hello-world, token, crowdfund)
soroban-forge new my-token --template token
cd my-token

# 4. it builds and passes tests out of the box
cargo test
stellar contract build

# 5. add a generated test harness and CI
soroban-forge test-init --force
soroban-forge ci-init --deploy
```

New to Soroban entirely? Follow the full walkthrough:
[docs/tutorial-zero-to-testnet.md](docs/tutorial-zero-to-testnet.md).

## Commands

| command                          | what it does                                              |
|----------------------------------|-----------------------------------------------------------|
| `new <name> --template <t>`      | scaffold a project (`--list-templates` to see options)    |
| `test-init`                      | generate `tests/` fixtures + smoke test for a contract    |
| `ci-init --provider github`      | write CI workflows; `--deploy` adds manual testnet deploy |
| `doctor`                         | check rustc/cargo, `wasm32v1-none` target, stellar-cli    |

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

See [CONTRIBUTING.md](CONTRIBUTING.md) for the ownership map and how to pick
up an issue — [ISSUES.md](ISSUES.md) lists well-scoped starter work.

## Requirements

- Rust ≥ 1.84 with the `wasm32v1-none` target
- [stellar-cli](https://developers.stellar.org/docs/tools/cli/stellar-cli) for building/deploying contracts
- Generated contracts use [soroban-sdk](https://crates.io/crates/soroban-sdk) 26.x

`soroban-forge doctor` checks all of this for you.

## License

[Apache-2.0](LICENSE)
