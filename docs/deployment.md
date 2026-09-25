# Deployment

Deploy to Testnet:

```sh
forge deploy --network testnet
```

Deploy to Mainnet:

```sh
forge deploy --network mainnet
```

## Automatic Friendbot Funding (--fund)

When deploying to Testnet, `soroban-forge deploy` verifies that the source account exists and has a non-zero XLM balance.

- If the source account is unfunded and running interactively, `soroban-forge` offers to fund it using Friendbot.
- In automated/CI environments or when running non-interactively, pass `--fund` to automatically request Friendbot funding:

```sh
forge deploy --network testnet --source alice --fund
```

Friendbot funding is only supported on `testnet`. It is never attempted on `mainnet` or in `--offline` mode.
