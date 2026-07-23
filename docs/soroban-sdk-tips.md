# Soroban SDK Tips

- Always use `i128` for token amounts — never `u64`
- `Env::current_contract_address()` is free; cache it only for readability
- Storage TTLs must be bumped on every read for persistent entries
- Use `#[contracttype]` for all custom types that cross the host boundary
