# Error Handling

## Define a typed error enum

```rust
use soroban_sdk::contracterror;

#[contracterror]
#[derive(Copy, Clone)]
pub enum Error {
    NotInitialized       = 1,
    AlreadyInitialized   = 2,
    Unauthorized         = 3,
    InsufficientBalance  = 4,
    InvalidAmount        = 5,
    ContractPaused       = 6,
}
```

## Return Results from entrypoints

```rust
pub fn deposit(env: Env, user: Address, amount: i128) -> Result<(), Error> {
    if !storage::is_initialized(&env) {
        return Err(Error::NotInitialized);
    }
    if amount <= 0 {
        return Err(Error::InvalidAmount);
    }
    // ...
    Ok(())
}
```

## Test error cases

```rust
let err = client.try_deposit(&user, &0);
assert!(err.is_err());
```
