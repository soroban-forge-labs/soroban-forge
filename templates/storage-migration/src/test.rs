use super::*;
use soroban_sdk::{
    testutils::storage::{Instance as _, Persistent as _},
    testutils::Ledger as _,
    Env, String,
};

#[test]
fn test_init_v2_state() {
    let env = Env::default();
    let client = StorageMigrationClient::new(&env, &env.register(StorageMigration, ()));

    let initial_counter = 42;
    let name = String::from_str(&env, "Alice");

    client.init(&initial_counter, &name);

    assert_eq!(client.get_counter(), 42);
    assert_eq!(client.get_name(), name);
    assert_eq!(client.version(), 2);
}

#[test]
fn test_increment() {
    let env = Env::default();
    let client = StorageMigrationClient::new(&env, &env.register(StorageMigration, ()));

    let initial_counter = 10;
    let name = String::from_str(&env, "Bob");

    client.init(&initial_counter, &name);
    client.increment();
    client.increment();

    assert_eq!(client.get_counter(), 12);
}

#[test]
fn test_set_name() {
    let env = Env::default();
    let client = StorageMigrationClient::new(&env, &env.register(StorageMigration, ()));

    client.init(&0, &String::from_str(&env, "original"));
    assert_eq!(client.get_name(), String::from_str(&env, "original"));

    let new_name = String::from_str(&env, "updated");
    client.set_name(&new_name);

    assert_eq!(client.get_name(), new_name);
}

#[test]
fn test_migration_from_v1_to_v2() {
    let env = Env::default();
    let contract_id = env.register(StorageMigration, ());
    let client = StorageMigrationClient::new(&env, &contract_id);

    // Simulate v1 state: directly write to storage using v1 key
    env.as_contract(&contract_id, || {
        let v1_counter = 100u32;
        env.storage()
            .instance()
            .set(&DataKey::CounterV1, &v1_counter);
        env.storage().instance().set(&DataKey::Version, &1u32);
    });

    // Call a v2 function which should trigger migration
    let name = String::from_str(&env, "migrated");
    client.set_name(&name);

    // Verify migration occurred
    assert_eq!(client.version(), 2);
    assert_eq!(client.get_counter(), 100); // v1 counter preserved
    assert_eq!(client.get_name(), name);
}

#[test]
fn test_migration_idempotency() {
    let env = Env::default();
    let contract_id = env.register(StorageMigration, ());
    let client = StorageMigrationClient::new(&env, &contract_id);

    // Set initial v1 state
    env.as_contract(&contract_id, || {
        let v1_counter = 50u32;
        env.storage()
            .instance()
            .set(&DataKey::CounterV1, &v1_counter);
        env.storage().instance().set(&DataKey::Version, &1u32);
    });

    // Trigger migration
    client.increment();
    let first_counter = client.get_counter();
    assert_eq!(first_counter, 51);

    // Call functions multiple times to ensure migration doesn't run again
    client.increment();
    client.increment();
    let second_counter = client.get_counter();
    assert_eq!(second_counter, 53);

    // Version should still be 2 (not re-migrated)
    assert_eq!(client.version(), 2);
}

#[test]
fn test_migration_preserves_v1_counter() {
    let env = Env::default();
    let contract_id = env.register(StorageMigration, ());
    let client = StorageMigrationClient::new(&env, &contract_id);

    // Write v1 state with a specific counter value
    env.as_contract(&contract_id, || {
        let original_counter = 999u32;
        env.storage()
            .instance()
            .set(&DataKey::CounterV1, &original_counter);
        env.storage().instance().set(&DataKey::Version, &1u32);
    });

    // Migrate to v2
    client.increment();

    // The v1 counter should be preserved and incremented by 1
    assert_eq!(client.get_counter(), 999 + 1);
}

#[test]
fn test_v2_state_functions_without_v1_data() {
    let env = Env::default();
    let contract_id = env.register(StorageMigration, ());
    let client = StorageMigrationClient::new(&env, &contract_id);

    // Initialize with v2 state (no v1 migration)
    let initial_counter = 5;
    let name = String::from_str(&env, "v2_init");

    client.init(&initial_counter, &name);

    // All operations should work without issues
    assert_eq!(client.get_counter(), 5);
    assert_eq!(client.get_name(), name);

    client.increment();
    assert_eq!(client.get_counter(), 6);

    let new_name = String::from_str(&env, "v2_updated");
    client.set_name(&new_name);
    assert_eq!(client.get_name(), new_name);
}

#[test]
fn test_default_values_when_no_state() {
    let env = Env::default();
    let contract_id = env.register(StorageMigration, ());
    let client = StorageMigrationClient::new(&env, &contract_id);

    // Query without initialization
    // Should return defaults due to unwrap_or() in the implementation
    assert_eq!(client.get_counter(), 0);
    assert_eq!(client.get_name(), String::from_str(&env, "unknown"));
}

#[test]
fn test_migration_with_multiple_data_points() {
    let env = Env::default();
    let contract_id = env.register(StorageMigration, ());
    let client = StorageMigrationClient::new(&env, &contract_id);

    // Simulate complex v1 state
    env.as_contract(&contract_id, || {
        env.storage()
            .instance()
            .set(&DataKey::CounterV1, &42u32);
        env.storage().instance().set(&DataKey::Version, &1u32);
    });

    // Perform multiple v2 operations
    client.set_name(&String::from_str(&env, "first"));
    assert_eq!(client.get_counter(), 42);
    assert_eq!(client.get_name(), String::from_str(&env, "first"));

    client.increment();
    assert_eq!(client.get_counter(), 43);

    client.set_name(&String::from_str(&env, "second"));
    assert_eq!(client.get_name(), String::from_str(&env, "second"));

    // Verify version is still 2 (no re-migration)
    assert_eq!(client.version(), 2);
}

#[test]
#[should_panic(expected = "already initialized")]
fn test_init_twice_fails() {
    let env = Env::default();
    let client = StorageMigrationClient::new(&env, &env.register(StorageMigration, ()));

    client.init(&1, &String::from_str(&env, "first"));
    client.init(&999, &String::from_str(&env, "overwrite"));
}

#[test]
fn test_second_init_leaves_state_untouched() {
    let env = Env::default();
    let client = StorageMigrationClient::new(&env, &env.register(StorageMigration, ()));

    let name = String::from_str(&env, "first");
    client.init(&1, &name);
    client.increment();

    assert!(client
        .try_init(&999, &String::from_str(&env, "overwrite"))
        .is_err());
    assert_eq!(client.get_counter(), 2);
    assert_eq!(client.get_name(), name);
}

#[test]
fn test_init_cannot_clobber_migrated_v1_state() {
    let env = Env::default();
    let contract_id = env.register(StorageMigration, ());
    let client = StorageMigrationClient::new(&env, &contract_id);

    // A contract upgraded from v1 with live data.
    env.as_contract(&contract_id, || {
        env.storage().instance().set(&DataKey::CounterV1, &77u32);
        env.storage().instance().set(&DataKey::Version, &1u32);
    });

    // init would otherwise migrate and then overwrite the migrated counter.
    assert!(client.try_init(&0, &String::from_str(&env, "reset")).is_err());

    // The migration example still works end to end.
    assert_eq!(client.get_counter(), 77);
    assert_eq!(client.get_name(), String::from_str(&env, "migrated"));
    assert_eq!(client.version(), 2);
}

#[test]
fn test_storage_survives_ledger_advance_past_default_ttl() {
    let env = Env::default();
    let contract_id = env.register(StorageMigration, ());
    let client = StorageMigrationClient::new(&env, &contract_id);

    let initial_counter = 42;
    let name = String::from_str(&env, "Alice");
    client.init(&initial_counter, &name);

    // Verify initial state and TTL extensions
    assert_eq!(client.get_counter(), 42);
    assert_eq!(client.get_name(), name);
    assert_eq!(client.version(), 2);

    env.as_contract(&contract_id, || {
        assert!(env.storage().instance().get_ttl() >= INSTANCE_TTL_THRESHOLD);
        assert!(
            env.storage().persistent().get_ttl(&DataKey::CounterV2) >= PERSISTENT_TTL_THRESHOLD
        );
        assert!(
            env.storage().persistent().get_ttl(&DataKey::NameV2) >= PERSISTENT_TTL_THRESHOLD
        );
    });

    // Advance ledger sequence past the default TTL (in test env, default TTL is 4096 ledgers).
    // Our extended TTL is 30 days (518,400 ledgers).
    env.ledger().with_mut(|li| li.sequence_number += 10_000);

    // Assert that the contract and persistent data continue to function
    assert_eq!(client.get_counter(), 42);
    assert_eq!(client.get_name(), name);

    // State changes extend TTL again and succeed
    client.increment();
    assert_eq!(client.get_counter(), 43);

    let updated_name = String::from_str(&env, "Bob");
    client.set_name(&updated_name);
    assert_eq!(client.get_name(), updated_name);

    // Advance ledger again by 20,000 ledgers
    env.ledger().with_mut(|li| li.sequence_number += 20_000);
    assert_eq!(client.get_counter(), 43);
    assert_eq!(client.get_name(), updated_name);
}

#[test]
fn test_migrated_storage_survives_ledger_advance() {
    let env = Env::default();
    let contract_id = env.register(StorageMigration, ());
    let client = StorageMigrationClient::new(&env, &contract_id);

    // Simulate v1 persistent state
    env.as_contract(&contract_id, || {
        env.storage()
            .persistent()
            .set(&DataKey::CounterV1, &100u32);
        env.storage().instance().set(&DataKey::Version, &1u32);
    });

    // Trigger migration
    assert_eq!(client.get_counter(), 100);
    assert_eq!(client.get_name(), String::from_str(&env, "migrated"));
    assert_eq!(client.version(), 2);

    // Advance past default TTL
    env.ledger().with_mut(|li| li.sequence_number += 10_000);

    // Assert preserved data survives
    assert_eq!(client.get_counter(), 100);
    assert_eq!(client.get_name(), String::from_str(&env, "migrated"));

    client.increment();
    assert_eq!(client.get_counter(), 101);
}
