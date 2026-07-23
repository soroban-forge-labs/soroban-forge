# Interoperability

Call another contract:

```rust
let client = OtherContractClient::new(&env, &other_address);
client.some_function(&arg);
```

Pass `auth_entries` for any addresses that need authorisation inside the sub-call.
