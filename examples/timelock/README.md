# timelock

A timelock controller contract for delayed governance call execution on Soroban.

The timelock enforces a minimum delay between proposing an on-chain operation and its execution, allowing community members and stakeholders to inspect, prepare for, or cancel scheduled actions (e.g., protocol upgrades, parameter changes, or treasury allocations).

## Workflow

1. **Queueing**: A designated `proposer` calls `queue` with target contract, function name, arguments, delay, and a unique salt. The delay must be greater than or equal to `min_delay`.
2. **Waiting**: During the delay period, the operation status is `Waiting`. Anyone can inspect the queued call details and execution timestamp.
3. **Execution**: Once the delay has passed (`timestamp >= ready_at`), an `executor` (or any authorized caller) calls `execute` to trigger the call via `env.invoke_contract`.
4. **Cancellation**: If a proposed operation is malicious, flawed, or superseded, a `proposer` or `admin` can call `cancel` before it is executed.

## Contract Interface

### Entrypoints

- `initialize(env, admin, proposers, executors, min_delay)`: Initializes roles and minimum delay.
- `get_min_delay(env)`: Returns the current minimum execution delay in seconds.
- `set_min_delay(env, new_delay)`: Updates the minimum execution delay (admin only).
- `hash_operation(env, target, function_name, args, salt)`: Computes the unique 32-byte operation ID.
- `get_operation_state(env, id)`: Returns the current status (`Unset`, `Waiting`, `Ready`, `Executed`, `Cancelled`).
- `get_operation(env, id)`: Returns full metadata for a queued operation.
- `queue(env, caller, target, function_name, args, delay, salt)`: Queues an operation for delayed execution.
- `execute(env, caller, target, function_name, args, salt)`: Executes a ready queued operation.
- `cancel(env, caller, id)`: Cancels a queued operation.
- `grant_role(env, admin, role, account)`: Grants a role (`Proposer`, `Executor`, `Admin`).
- `revoke_role(env, admin, role, account)`: Revokes a role.

## Testing

```sh
cargo test
```

## Deployment

1. Build the contract wasm:
   ```sh
   stellar contract build
   ```
2. Deploy to Testnet:
   ```sh
   stellar contract deploy \
     --wasm target/wasm32-unknown-unknown/release/timelock.wasm \
     --source <IDENTITY> \
     --network testnet
   ```
3. Initialize the timelock controller:
   ```sh
   stellar contract invoke \
     --id <TIMELOCK_CONTRACT_ID> \
     --source <IDENTITY> \
     --network testnet \
     -- initialize \
     --admin <ADMIN_ADDRESS> \
     --proposers '["<PROPOSER_ADDRESS>"]' \
     --executors '["<EXECUTOR_ADDRESS>"]' \
     --min_delay 86400
   ```
