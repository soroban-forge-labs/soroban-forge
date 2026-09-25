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

## Publishing the Package

`soroban-forge bindings ts` writes a `package.json` that is ready to
publish: a conditional `exports` map with `types`, a `files` list, a
`prepack` build, and `@stellar/stellar-sdk` as a **peer dependency**. The
peer range follows the SDK the installed `stellar-cli` targets (`^16` for
stellar-cli 28), so consumers install the SDK alongside the client:

```sh
npm install my-token @stellar/stellar-sdk@^16
```

Type declarations resolve under both `node16`/`nodenext` and `bundler`
module resolution. See `crates/binding-ts/README.md` for the full field list.

## Options

- `--out-dir <dir>` (alias `--output`) - target directory for generated bindings. Defaults to `bindings/<contract_name>`.
- `--package-name <name>` - npm package name in `package.json`. Validated according to npm package naming rules. Defaults to `@soroban-contracts/<contract_name>`.
- `--contract <name>` - contract name when running in a multi-contract workspace.
- `--overwrite` - overwrite existing output directory.

### Example with Custom Output and Package Name

```sh
forge bindings ts --out-dir src/clients/distribution --package-name @my-org/distribution-client
```
