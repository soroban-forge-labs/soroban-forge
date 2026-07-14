# Tutorial: zero to a deployed testnet contract

This walkthrough takes you from an empty machine to a Soroban smart contract
deployed on the Stellar **testnet**, using `soroban-forge`. No prior Soroban
knowledge needed. (~20 minutes)

## 0. What you're building

Soroban is Stellar's smart-contract platform; contracts are Rust compiled to
WebAssembly. You'll scaffold a token contract, run its tests, wire up CI, and
deploy it to the free public testnet with the official `stellar` CLI.

## 1. Install the toolchain

```sh
# Rust (includes cargo)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# the WebAssembly target Soroban contracts compile to
rustup target add wasm32v1-none

# the official Stellar CLI (macOS/Linux; see docs for Windows)
brew install stellar-cli   # or: cargo install --locked stellar-cli

# soroban-forge itself
git clone https://github.com/soroban-forge-labs/soroban-forge
cd soroban-forge && cargo install --path . && cd ..
```

Verify everything at once:

```sh
soroban-forge doctor
```

Every line should show `✓`; anything else prints the exact fix command.

## 2. Scaffold a project

```sh
soroban-forge new my-token --template token
cd my-token
```

You get a complete cargo project: a fungible token contract implementing the
standard Soroban token interface (SEP-41), unit tests, a README and a
`forge.toml`. Prove it works before touching anything:

```sh
cargo test
```

## 3. Look around

- `src/lib.rs` — the contract. Note `#[contract]`, `#[contractimpl]` and the
  `__constructor` that receives the admin and token metadata at deploy time.
- `src/test.rs` — unit tests using the SDK's `testutils` (mocked auth,
  generated accounts).

Add a generated test harness with reusable fixtures and a snapshot helper:

```sh
soroban-forge test-init --force   # --force: the template already ships tests/
cargo test
```

## 4. Build the wasm

```sh
stellar contract build
```

The deployable file lands in `target/wasm32v1-none/release/my_token.wasm`.

## 5. Create and fund a testnet identity

Testnet is free; friendbot funds new accounts:

```sh
stellar keys generate alice --network testnet --fund
stellar keys address alice     # your public key, G...
```

## 6. Deploy

The token's constructor needs an admin plus metadata — pass constructor args
after `--`:

```sh
stellar contract deploy \
  --wasm target/wasm32v1-none/release/my_token.wasm \
  --source-account alice \
  --network testnet \
  --alias my_token \
  -- \
  --admin "$(stellar keys address alice)" \
  --decimals 7 \
  --name "My Token" \
  --symbol MYT
```

The printed `C...` address is your live contract.

## 7. Invoke it

```sh
# mint 100.0000000 MYT (7 decimals) to alice
stellar contract invoke --id my_token --source-account alice --network testnet \
  -- mint --to "$(stellar keys address alice)" --amount 1000000000

# check the balance
stellar contract invoke --id my_token --source-account alice --network testnet \
  -- balance --id "$(stellar keys address alice)"
```

## 8. Add CI

```sh
soroban-forge ci-init --deploy
git init && git add -A && git commit -m "my first Soroban contract"
```

Push to GitHub and you have build+test and contract-size checks on every PR.
For the manual deploy workflow, add a repository secret named
`STELLAR_DEPLOYER_SECRET` (Settings → Secrets and variables → Actions)
containing a funded testnet secret key — the workflow only ever references
the secret, it is never stored.

## Where next

- Try the other templates: `soroban-forge new demo --template crowdfund`
- [Official Soroban docs](https://developers.stellar.org/docs/build/smart-contracts)
- [Example contracts](https://github.com/stellar/soroban-examples)
- Contribute a template or preset: see [CONTRIBUTING.md](../CONTRIBUTING.md)
  and [ISSUES.md](../ISSUES.md)
