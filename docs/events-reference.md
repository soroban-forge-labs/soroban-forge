# Events Reference

Soroban events are the primary way off-chain services track contract activity.

## Publishing an Event

```rust
pub fn emit_deposited(env: &Env, user: &Address, pool_id: PoolId, amount: i128) {
    env.events().publish(
        (Symbol::new(env, "Deposited"), pool_id),
        (user.clone(), amount),
    );
}
```

## Event Structure

- **Topics** (up to 4): indexed, used for filtering. Use Symbols and IDs.
- **Data**: unindexed payload. Use structs or tuples.

## Querying Events via RPC

```sh
stellar-cli events \
  --start-ledger 1000000 \
  --contract-id C... \
  --event-type contract
```

## Best Practices

- Always emit an event for every state transition
- Keep topic cardinality low (use pool_id not user address as topic)
- Document every event in your contract's ABI
