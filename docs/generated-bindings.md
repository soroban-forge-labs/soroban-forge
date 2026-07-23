# Generated TypeScript Bindings

Forge auto-generates type-safe TypeScript clients from your contract ABI.

## Generate

```sh
forge bindings --contract distribution --output src/clients/
```

## Usage

```typescript
import { DistributionClient } from './clients/distribution';

const client = new DistributionClient({
  contractId: 'C...',
  rpcUrl: 'https://soroban-testnet.stellar.org',
  keypair: Keypair.fromSecret('S...'),
});

const pending = await client.getPending({ user: 'G...' });
console.log('Pending reward:', pending);
```

## Supported Types

| Rust type | TypeScript type |
|-----------|----------------|
| `i128`   | `bigint` |
| `Address` | `string` |
| `Symbol` | `string` |
| `Vec<T>` | `T[]` |
| `Map<K,V>` | `Map<K,V>` |
