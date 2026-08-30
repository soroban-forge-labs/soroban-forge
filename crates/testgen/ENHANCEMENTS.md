# Testgen Enhancements

This document tracks enhancements to the Soroban Forge test generation system.

## Implemented Features

### #225: --no-tests CLI Flag

**Status**: In Progress

Allows users to scaffold a project without the `tests/` directory for projects bringing their own test harness.

```bash
soroban-forge new mycontract --template token --no-tests
```

**Implementation Notes**:
- Added `--no-tests` CLI argument to scaffold command
- Filters out `tests/` directory during template rendering
- Updates `Cargo.toml` to remove test dependencies when flag is set
- Ensures project still builds with `cargo build`

### #226: Cross-Contract Call Test Generation

**Status**: Planned

When a contract stores or takes another contract's address as a dependency, automatically generate:
- A minimal mock contract stub
- Wire-up code to inject the mock into tests
- A passing integration test demonstrating the contract interaction

**Expected Behavior**:
```rust
// Generated mock contract (if dependency detected)
pub struct MockOracleContract;

// Generated test
#[test]
fn test_with_oracle_integration() {
    let env = Env::default();
    let contract = ContractClient::new(&env, &contract_id);
    let mock_oracle = deploy_mock_oracle(&env);
    
    // Test the contract with the mock
    contract.execute_with_oracle(&mock_oracle);
    assert!(/* assertions */);
}
```

**Implementation Hooks**:
- Detect contract methods with `Address` parameter names matching common patterns (e.g., `*_contract`, `*_address`)
- Generate mock implementation in `tests/mock/mod.rs`
- Update `Cargo.toml` to expose mock in test module

### #227: Authorization Tree Assertion Generation

**Status**: Planned

For entrypoints calling `caller.require_auth()`, generate explicit assertions over the recorded authorization tree instead of just success/failure checks.

**Expected Behavior**:
```rust
// For a contract like:
pub fn transfer(env: Env, caller: Address, amount: i128) {
    caller.require_auth();
    // ... transfer logic
}

// Generate tests that assert:
#[test]
fn test_transfer_requires_auth() {
    let env = Env::default();
    let contract = ContractClient::new(&env, &contract_id);
    let caller = Address::random(&env);
    
    contract.transfer(&caller, &100);
    
    // Assert authorization tree contains the expected entry
    assert_auth_tree_contains(&env, &caller, "transfer");
}
```

**Implementation Hooks**:
- Detect `require_auth()` calls in contract methods
- Generate `AuthEntry` assertions using Soroban SDK auth recording
- Skip silently for contracts with no authenticated entrypoints

### #228: Ledger Time Helper

**Status**: Planned

Generate a reusable `advance_ledger(&env, n)` helper function that atomically updates both sequence number and timestamp, enabling consistent time-dependent test patterns.

**Expected Behavior**:
```rust
// Generated helper (once per project)
pub fn advance_ledger(env: &Env, ledgers: u32) {
    let new_sequence = env.ledger().sequence() + ledgers as u32;
    let new_timestamp = env.ledger().timestamp() + (ledgers * 6) as u64; // ~6s per ledger
    // Internal env manipulation to set both atomically
}

// Used in time-dependent tests
#[test]
fn test_vesting_schedule() {
    let env = Env::default();
    let contract = VestingClient::new(&env, &contract_id);
    
    // Fast-forward 100 ledgers (≈ 10 minutes)
    advance_ledger(&env, 100);
    
    // Verify vesting has progressed
    assert_eq!(contract.available_balance(), expected_amount);
}
```

**Benefits**:
- Consistent time advancement across all tests
- Readable test code that's easy to audit
- Aligns with common testing patterns in other blockchains
- Reduces magic numbers in test files

**Implementation Hooks**:
- Generate `pub mod helpers { pub fn advance_ledger(...) {...} }` in tests
- Detect templates using time-dependent logic (vesting, streaming, subscriptions)
- Inject `use helpers::advance_ledger` in generated test files
- Document in `docs/testing-guide.md`

## Integration Points

These enhancements integrate with:

1. **Template System**: Metadata in `template.toml` can declare which enhancements to use
2. **Test Harness**: Uses standard Soroban test harness and SDK
3. **CLI Args**: New flags to control generation behavior
4. **Documentation**: Updated in CLI help and `docs/testing-guide.md`

## Future Enhancements

- [ ] Property-based testing generation (proptest/quickcheck)
- [ ] Fuzzing harness generation
- [ ] Gas cost profiling test generation
- [ ] Error case test generation
