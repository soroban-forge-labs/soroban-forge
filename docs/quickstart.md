# Quickstart — 5 Minutes to Your First Soroban Contract

## Prerequisites
- Rust + `wasm32-unknown-unknown` target
- `stellar-cli` ≥ 21.0

## Steps

```sh
# 1. Install Soroban Forge
npm install -g soroban-forge

# 2. Scaffold a project
forge init hello-soroban --template token
cd hello-soroban

# 3. Build
forge build

# 4. Test
forge test

# 5. Deploy to testnet
forge deploy --network testnet
```

That's it. Your contract is live on Stellar Testnet.
