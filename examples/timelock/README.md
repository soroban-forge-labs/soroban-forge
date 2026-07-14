# Time-Lock Example

Lock tokens until a future ledger timestamp, then release to a beneficiary.

```sh
forge init my-timelock --template timelock
```

## Features

- Depositor locks tokens with an unlock timestamp
- Beneficiary can claim only after `ledger.timestamp() >= unlock_at`
- Depositor can extend the lock but never shorten it

## Key Functions

| Function | Description |
|----------|-------------|
| `lock(token, amount, unlock_at, beneficiary)` | Create a time-lock |
| `claim(lock_id)` | Release to beneficiary after unlock |
| `extend(lock_id, new_unlock_at)` | Extend the lock period |
