# Testing Strategies

## Unit Tests
Test pure functions and storage helpers in isolation.

## Integration Tests
Use `Env::default()` with `mock_all_auths()` for full-contract flows.

## Fuzz Tests
Use property-based testing to find edge cases in math-heavy contracts.

## Snapshot Tests
Soroban SDK records ledger state snapshots for regression detection.
