# soroban-forge-deploy

`soroban-forge deploy` — builds arguments and deploys compiled Soroban contract WASM files to Stellar networks (`testnet`, `mainnet`, or custom RPCs).

## Features

- **Testnet auto-funding**: Detects unfunded testnet source identities before submission, offering friendbot funding interactively or automatically via `--fund`.
- **Offline protection**: Prevents accidental network submissions and friendbot calls under `--offline`.
- **Dry-run support**: Inspect the exact command that would be run via `--dry-run`.

## Usage

```sh
# Deploy to testnet with auto-funding
soroban-forge deploy --network testnet --source alice --fund

# Dry run deployment
soroban-forge deploy --network testnet --source alice --dry-run
```
