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

### Example with Custom Output and Package Name

```sh
forge bindings ts --out-dir src/clients/distribution --package-name @my-org/distribution-client
```

## React Hooks

`soroban-forge bindings ts --react` additionally emits `src/hooks.ts`: one
hook per entrypoint, typed against the generated client. It is strictly
opt-in — omit the flag and nothing react-related is generated or declared.

```tsx
import { Client } from "my-token";
import { useBalance, useMintMutation } from "my-token/hooks";

const client = new Client({ contractId: "C...", networkPassphrase: "...", rpcUrl: "..." });

function Balance({ id }: { id: string }) {
  const { data, loading, error, refetch } = useBalance(client, { id });
  if (loading) return <p>Loading…</p>;
  if (error) return <p>{error.message}</p>;
  return <p>{String(data)} <button onClick={refetch}>Refresh</button></p>;
}

function MintButton({ to }: { to: string }) {
  const { mutate, loading } = useMintMutation(client);
  return <button disabled={loading} onClick={() => mutate({ to, amount: 100n })}>Mint</button>;
}
```

Read entrypoints (`get_*`, `balance`, `owner`, `admin`, …) get a query-style
hook that fetches on mount; everything else gets a mutation hook that only
runs when you call `mutate(...)`. See `crates/binding-ts/README.md` for the
full classification rule and the `package.json` changes (`react` as an
optional peer dependency, a `./hooks` export subpath).
