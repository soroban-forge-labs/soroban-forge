# Contract Events

Publish events with `env.events().publish()`.

```rust
env.events().publish((Symbol::new(&env, "transfer"),), (from, to, amount));
```

Events are queryable via Horizon or RPC `getEvents`.
