# Merkle Airdrop Example

Distribute tokens to a large allowlist using a Merkle proof, storing only the root on-chain.

```sh
forge init my-airdrop --template merkle-airdrop
```

## How It Works

1. Off-chain: build a Merkle tree of `(address, amount)` pairs
2. Store only the Merkle root in the contract
3. Claimants submit a proof — the contract verifies and transfers

## Key Functions

| Function | Description |
|----------|-------------|
| `set_root(root)` | Admin sets the Merkle root |
| `claim(proof, amount)` | Claimant proves eligibility |
