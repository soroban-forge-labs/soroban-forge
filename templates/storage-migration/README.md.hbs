# Storage Migration Example

Demonstrates safe storage layout migration from v1 to v2 using a version-marker pattern.

```sh
forge new my-storage-migration --template storage-migration
```

## Problem

Smart contracts evolve. When you upgrade a contract, the persistent storage remains unchanged—but your new code might expect a different storage layout:

- Adding new fields
- Removing old fields
- Restructuring how data is organized

**Risk**: If the new code tries to read from a key the old code didn't set, or assumes a different structure, data corruption or panics occur.

## Solution: Version-Marker Pattern

Use a storage version key to track which migration state the contract is in. On upgrade:

1. **Check the version** when any function accesses storage
2. **If an old version**, run the migration exactly once
3. **Update the version** marker to signal migration is done
4. **All subsequent calls** see the updated version and skip migration

This ensures:
- Migration runs automatically on first post-upgrade call
- Migration is idempotent (safe to call multiple times)
- No special deployment step needed—just call the contract normally

## Architecture

### V1 Storage Layout

```rust
#[contracttype]
pub enum DataKey {
    CounterV1,
}
```

Simple: just a counter.

### V2 Storage Layout

```rust
#[contracttype]
pub enum DataKey {
    CounterV2,
    NameV2,
}
```

Extended: counter + name. The migration preserves the v1 counter and adds a default name.

### Migration Logic

```rust
fn migrate_if_needed(env: &Env) {
    let current_version = get_version(env);
    
    if current_version >= CURRENT_VERSION {
        return; // Already migrated
    }
    
    if current_version == VERSION_1 {
        Self::migrate_v1_to_v2(env);
    }
    
    set_version(env, CURRENT_VERSION);
}
```

**Key insight**: Every function that accesses storage calls `migrate_if_needed()` first.

## Code Example

### V1 Contract (before upgrade)

```rust
pub fn init(env: Env, counter: u32) {
    env.storage().instance().set(&DataKey::CounterV1, &counter);
}

pub fn get_counter(env: Env) -> u32 {
    env.storage().instance().get(&DataKey::CounterV1).unwrap_or(0)
}
```

### V2 Contract (after upgrade)

```rust
pub fn init(env: Env, counter: u32, name: String) {
    migrate_if_needed(&env);  // <-- Automatic migration
    env.storage().instance().set(&DataKey::CounterV2, &counter);
    env.storage().instance().set(&DataKey::NameV2, &name);
}

pub fn get_counter(env: Env) -> u32 {
    migrate_if_needed(&env);  // <-- Automatic migration
    env.storage().instance().get(&DataKey::CounterV2).unwrap_or(0)
}

pub fn get_name(env: Env) -> String {
    migrate_if_needed(&env);  // <-- Automatic migration
    env.storage().instance().get(&DataKey::NameV2).unwrap_or(String::from_slice(&env, "unknown"))
}
```

## Testing the Migration

This template includes tests that verify:

1. **Direct v1→v2 migration**: Write v1 state, call a v2 function, assert v2 layout
2. **Idempotency**: Migration runs once and doesn't repeat
3. **Data preservation**: V1 data is carried forward correctly
4. **Clean v2 init**: New deployments starting in v2 work fine

```bash
cargo test
```

Example test:

```rust
#[test]
fn test_migration_from_v1_to_v2() {
    let env = Env::default();
    let contract_id = env.register(StorageMigration, ());
    let client = StorageMigrationClient::new(&env, &contract_id);

    // Simulate v1 state
    env.storage().instance().set(&DataKey::CounterV1, &100u32);
    env.storage().instance().set(&DataKey::Version, &1u32);

    // Call a v2 function → triggers migration
    client.set_name(&String::from_slice(&env, "migrated"));

    // Verify
    assert_eq!(client.version(), 2);
    assert_eq!(client.get_counter(), 100);  // v1 counter preserved
}
```

## Best Practices

### 1. Always Check Version First

Every storage-accessing function should call `migrate_if_needed()` at the start:

```rust
pub fn some_function(env: Env) {
    migrate_if_needed(&env);
    // Now safe to access storage
}
```

### 2. Preserve Data During Migration

When migrating data structures, always copy old data into new keys:

```rust
fn migrate_v1_to_v2(env: &Env) {
    let v1_value = env.storage().instance().get(&DataKey::OldKey).unwrap_or(default);
    env.storage().instance().set(&DataKey::NewKey, &v1_value);
}
```

### 3. Use Predictable Default Values

When a field is removed or a new field is added, use `unwrap_or()` with a sensible default:

```rust
env.storage().instance().get(&DataKey::Field).unwrap_or(default_value)
```

### 4. Test Migration Path

Always test:
- Writing old state, calling a function, verifying new state
- Idempotency (calling functions repeatedly after migration)
- Edge cases (empty state, maximum values, etc.)

### 5. Optional: Clean Up Old Keys

After migration, you may optionally remove old keys to save storage space:

```rust
fn migrate_v1_to_v2(env: &Env) {
    // ... migration logic ...
    env.storage().instance().remove(&DataKey::CounterV1);
}
```

## Multi-Step Migrations

If you have v1 → v2 → v3, handle all steps:

```rust
fn migrate_if_needed(env: &Env) {
    let current_version = get_version(env);
    
    if current_version >= CURRENT_VERSION {
        return;
    }
    
    if current_version == VERSION_1 {
        Self::migrate_v1_to_v2(env);
        // Fall through to v2→v3
    }
    
    if current_version < VERSION_3 {
        Self::migrate_v2_to_v3(env);
    }
    
    set_version(env, CURRENT_VERSION);
}
```

## Deployment Strategy

1. **Deploy v1 contract** to testnet
2. **Mint some data** in production
3. **Build v2 wasm** with migration logic
4. **Upgrade the contract** using the admin's upgrade authority
5. **Call any function** on the upgraded contract
   - Automatically triggers migration
   - Data is preserved in new layout
6. **Verify** with queries to new functions

## Related Patterns

- **[Upgradeable Pattern](../upgradeable/)** — Admin-gated contract upgrades
- **[Storage TTL](../../docs/storage-ttl.md)** — Long-term storage persistence
- **[Storage Design ADR](../../developer_notes/adr/001-storage-key-design.md)** — Storage key conventions

## Limitations

- **Backward compatibility**: Once upgraded, the old contract code is gone
- **No rollback**: If migration has a bug, you'll need another upgrade to fix it
- **Gas cost**: The first call after upgrade pays for the migration (often negligible)
- **Single migration instance**: This pattern works per contract instance; multi-contract deployments need per-contract handling

## Learn More

- [Soroban Storage Docs](https://stellar.org/docs/learn/storing-data)
- [Contract Upgrade Patterns](https://docs.rs/soroban-sdk/latest/soroban_sdk/deployer/)
- [Version Markers in Production](https://en.wikipedia.org/wiki/Schema_migration)

## Security

This template is a starting point, not audited code. See [security-considerations.md](https://github.com/soroban-forge-labs/soroban-forge/blob/main/docs/security-considerations.md) for what it does and does not protect against.
