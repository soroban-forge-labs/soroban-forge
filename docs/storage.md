# Storage

## Durability Types

- **Persistent** – survives ledger close, requires TTL bumps
- **Temporary** – auto-deleted after TTL, no bump needed for ephemeral data
- **Instance** – scoped to the contract instance

## TTL Bumping

```rust
env.storage().persistent().extend_ttl(&key, MIN_TTL, MAX_TTL);
```
