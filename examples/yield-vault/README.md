# yield-vault

An ERC4626-style yield-bearing vault for Soroban smart contracts.

Users deposit an underlying SEP-41 token to receive proportional vault shares. As yield is generated and deposited into the vault, the exchange rate of assets per share increases, allowing shareholders to withdraw more underlying tokens than they originally deposited.

## Rounding Policy

The vault strictly implements a **vault-favoured rounding policy** (rounding down during integer division) for all share and asset conversions:

1. **Deposits (`convert_to_shares`)**:
   $$\text{shares} = \left\lfloor \frac{\text{assets} \times \text{total\_shares}}{\text{total\_assets}} \right\rfloor$$
   Shares are rounded down to the nearest integer. Depositors receive at most their exact proportional share count, ensuring new depositors cannot dilute existing shareholders through fractional rounding.

2. **Withdrawals (`convert_to_assets`)**:
   $$\text{assets} = \left\lfloor \frac{\text{shares} \times \text{total\_assets}}{\text{total\_shares}} \right\rfloor$$
   Assets are rounded down to the nearest integer. Withdrawing users receive at most their exact entitlement, leaving any sub-unit fractional asset surplus in the vault for remaining shareholders.

Under no circumstances does integer truncation favour an individual user over the pool.

## Contract Interface

### Entrypoints

- `__constructor(env, asset)`: Sets the underlying SEP-41 token address at deploy time, atomically with deployment.
- `asset(env)`: Returns the address of the underlying asset token.
- `total_assets(env)`: Returns the total amount of underlying assets managed by the vault.
- `total_shares(env)`: Returns the total number of vault shares currently minted.
- `balance_of(env, account)`: Returns the share balance of a given account.
- `convert_to_shares(env, assets)`: Previews the number of shares minted for a given asset amount.
- `convert_to_assets(env, shares)`: Previews the number of assets returned for a given share amount.
- `deposit(env, from, assets)`: Transfers `assets` from the caller to the vault and mints proportional shares.
- `withdraw(env, from, shares)`: Burns `shares` from the caller and transfers proportional underlying assets.
- `add_yield(env, from, yield_amount)`: Deposits additional underlying assets as yield without minting new shares.
- `transfer(env, from, to, amount)`: Transfers vault shares from the caller to `to`.
- `approve(env, from, spender, amount)`: Authorizes `spender` to transfer up to `amount` of the caller's shares.
- `allowance(env, from, spender)`: Returns the amount `spender` may transfer from `from`'s shares.
- `transfer_from(env, spender, from, to, amount)`: Transfers shares from `from` to `to` using a previously approved allowance.

## Testing

```sh
cargo test
```

## Deployment

1. Build the contract wasm:
   ```sh
   stellar contract build
   ```
2. Deploy to Testnet, passing the constructor argument (there is no separate
   initialize step — it runs atomically with deployment):
   ```sh
   stellar contract deploy \
     --wasm target/wasm32-unknown-unknown/release/yield_vault.wasm \
     --source <IDENTITY> \
     --network testnet \
     -- --asset <ASSET_TOKEN_ADDRESS>
   ```

## Security

This template is a starting point, not audited code. See [security-considerations.md](https://github.com/soroban-forge-labs/soroban-forge/blob/main/docs/security-considerations.md) for what it does and does not protect against.
