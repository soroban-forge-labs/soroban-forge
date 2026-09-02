# Testgen Enhancements

## #225: --no-tests CLI Flag
**Status**: Implemented

Allows scaffolding without the `tests/` directory:
```bash
soroban-forge new mycontract --template token --no-tests
```

## #226: Cross-Contract Call Test Generation
**Status**: Planned

Automatically generate mock contracts and integration tests for cross-contract calls.

## #227: Authorization Tree Assertion Generation
**Status**: Planned

Generate explicit assertions over authorization tree for `require_auth()` calls.

## #228: Ledger Time Helper
**Status**: Planned

Generate `advance_ledger(&env, n)` helper for time-dependent tests:
```rust
advance_ledger(&env, 100); // Fast-forward 100 ledgers
```
