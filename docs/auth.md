# Authentication

Soroban uses an invocation tree authorisation model.

```rust
fn transfer(env: Env, from: Address, to: Address, amount: i128) {
    from.require_auth();
    // ...
}
```

The caller must include an `auth_entry` for `from` in the transaction.
