# ADR-002: Contract Event Schema

**Status:** Accepted  
**Date:** 2026-07-05

## Context

Different contracts in the ecosystem publish events in inconsistent formats, making indexer development painful.

## Decision

All Forge-generated contracts follow this event schema:

- **Topic[0]**: `Symbol` — event name in PascalCase (e.g. `"Deposited"`)
- **Topic[1]**: entity ID where applicable (e.g. `pool_id: u32`)
- **Data**: tuple of `(actor: Address, amount: i128)` or relevant payload

## Example

```rust
env.events().publish(
    (Symbol::new(env, "Deposited"), pool_id),
    (user.clone(), amount),
);
```

## Rationale

- Consistent topic[0] lets indexers filter by event type across all Forge contracts
- Keeping amounts in data (not topics) avoids topic bloat
