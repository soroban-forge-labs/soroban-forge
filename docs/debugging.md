# Debugging Soroban Contracts

## Local Simulation

Use `forge simulate` to dry-run a transaction without broadcasting.

## Logs

Soroban events appear in the simulation result under `events[]`.

## Common Errors

- `WasmVm`: contract panicked — check your `#[cfg(test)]` coverage
- `InvalidInput`: serialisation mismatch — verify argument types
