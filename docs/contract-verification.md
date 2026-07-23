# Contract Verification

Forge integrates with the Stellar contract verification service to make your source code publicly auditable.

## Verify a deployed contract

```sh
forge verify \
  --contract distribution \
  --contract-id C... \
  --network mainnet
```

This uploads your source code and build reproducibility metadata to the verification registry.

## What Gets Verified

- Contract ID on-chain matches the WASM hash of the submitted source
- Build flags match the canonical release profile
- Source is pinned to a specific git commit

## Benefits

- Explorers (Stellar Expert, Lobstr) show a ✅ Verified badge
- Users can audit the exact code their funds interact with
