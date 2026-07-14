# Multi-Contract Workspaces

Forge supports Cargo workspaces with multiple contracts.

## Layout

```
my-project/
  forge.config.ts
  Cargo.toml          ← workspace root
  contracts/
    token/
      Cargo.toml
      src/lib.rs
    staking/
      Cargo.toml
      src/lib.rs
```

## Building All Contracts

```sh
forge build --all
```

## Deploying a Specific Contract

```sh
forge deploy --contract token --network testnet
```

## Cross-Contract Calls

Use the generated TypeScript bindings to call one contract from another in tests:

```typescript
import { TokenClient } from '../contracts/token';
const token = new TokenClient({ contractId, rpcUrl, keypair });
await token.transfer({ from, to, amount });
```
