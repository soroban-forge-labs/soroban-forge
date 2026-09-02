# Contract Verification

## Does the deployed contract match my build?

```sh
soroban-forge verify CDLZ… --network testnet
```

`verify` compares the wasm deployed at a contract ID with your local release
build. A Soroban contract's on-chain wasm hash is the SHA-256 of its deployed
bytes, so the check is a hash comparison:

| side       | what is hashed                                                  |
|------------|-----------------------------------------------------------------|
| local      | `target/wasm32v1-none/release/<crate>.wasm`, or `--wasm <path>` |
| on-chain   | the bytes `stellar contract fetch` returns for the contract ID  |

A match means the deployed contract is byte-for-byte the wasm you have
locally.

```
✓ verified — the deployed contract matches the local build

  contract   CDLZ…
  network    testnet
  local      target/wasm32v1-none/release/my_token.wasm

  sha256     9f2c…
```

A mismatch names both hashes:

```
✗ MISMATCH — the deployed contract was NOT built from this wasm

  contract   CDLZ…
  network    testnet
  local      target/wasm32v1-none/release/my_token.wasm

  local      sha256 9f2c…
  on-chain   sha256 41ab…
```

### Options

| flag                       | meaning                                                     |
|----------------------------|-------------------------------------------------------------|
| `--path <dir>`             | contract project directory [default: current directory]     |
| `--wasm <file>`            | compare this wasm instead of the project's release build     |
| `--network`, `-n <name>`   | configured network to query [default: `testnet`]            |
| `--rpc-url <url>`          | query this RPC endpoint instead of a configured network      |
| `--network-passphrase <p>` | passphrase for `--rpc-url`                                   |
| `--reproducible`           | build inside the pinned image before hashing                 |

`--wasm` and `--reproducible` are mutually exclusive — the former skips
the local build entirely (useful for verifying a downloaded release
artifact), the latter rebuilds from source inside the pinned container
so the result does not depend on the local toolchain.

When the network is not given on the command line it falls back to
`[network]` in `forge.toml`:

```toml
[network]
name = "testnet"
rpc_url = "https://soroban-testnet.stellar.org"
passphrase = "Test SDF Network ; September 2015"
```

CLI flags override the file values; `--network` and `--rpc-url` may be
combined to point a named network at a non-default RPC endpoint.

### Reproducible builds

`--reproducible` runs `stellar contract build` inside the image pinned at
[`REPRODUCIBLE_IMAGE`] in `crates/verify/src/lib.rs` and hashes the
result. The image is pinned by digest (`@sha256:…`), so the bytes
produced for a given source tree do not depend on the host toolchain —
useful when verifying a release against the original sources.

A missing Docker daemon is reported as exit `2` (missing tool) so CI can
distinguish it from a real mismatch:

```
$ soroban-forge verify --reproducible CDLZ…
error: docker not found on PATH (run `soroban-forge doctor` for install instructions)
```

To refresh the pin, update the `REPRODUCIBLE_IMAGE` constant to the new
digest and rebuild; the comparison only succeeds with the exact image
that produced the bytes in the first place.

### Exit codes

| code | meaning                                                        |
|------|-----------------------------------------------------------------|
| `0`  | the hashes match                                                |
| `1`  | the hashes differ, or an argument/local build was wrong         |
| `2`  | the `stellar` CLI is not installed (`soroban-forge doctor`)     |

So a CI job can gate on a deployment being the reviewed code:

```sh
soroban-forge verify "$CONTRACT_ID" --network testnet || exit 1
```

`--json` prints the same report machine-readably, which distinguishes a
mismatch from a bad argument (both exit `1`):

```json
{
  "contract_id": "CDLZ…",
  "network": "testnet",
  "local_wasm": "target/wasm32v1-none/release/my_token.wasm",
  "local_hash": "9f2c…",
  "onchain_hash": "41ab…",
  "match": false
}
```

### When a mismatch is expected

- the deployed contract is an **older build** — redeploy, or check out the
  commit that was deployed and rebuild
- you built with **different flags** than the deployment (wasm is sensitive
  to optimisation settings; always compare a `stellar contract build` release
  artifact, not a `cargo build` one)
- you are pointed at the **wrong network** — `--network mainnet` and
  `--network testnet` hold different deployments of the same project

## Public source verification

Publishing your *source* alongside the deployment — so explorers such as
Stellar Expert can show a verified badge and users can read the code their
funds interact with — is a separate flow run through the Stellar contract
verification service, and is not part of `soroban-forge` today. `verify`
answers the local question only: does this build correspond to that
deployment?
