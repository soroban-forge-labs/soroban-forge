use super::*;
use soroban_sdk::{String, Env};

#[test]
fn test_init_v2_state() {
    let env = Env::default();
    let client = StorageMigrationClient::new(&env, &env.register(StorageMigration, ()));

    let initial_counter = 42;
    let name = String::from_slice(&env, "Alice");

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
    let name = String::from_slice(&env, "Bob");

    client.init(&initial_counter, &name);
    client.increment();
    client.increment();

    assert_eq!(client.get_counter(), 12);
}

#[test]
fn test_set_name() {
    let env = Env::default();
    let client = StorageMigrationClient::new(&env, &env.register(StorageMigration, ()));

    client.init(&0, &String::from_slice(&env, "original"));
    assert_eq!(client.get_name(), String::from_slice(&env, "original"));

    let new_name = String::from_slice(&env, "updated");
    client.set_name(&new_name);

    assert_eq!(client.get_name(), new_name);
}

#[test]
fn test_migration_from_v1_to_v2() {
    let env = Env::default();
    let contract_id = env.register(StorageMigration, ());
    let client = StorageMigrationClient::new(&env, &contract_id);

    // Simulate v1 state: directly write to storage using v1 key
    {
        let v1_counter = 100u32;
        env.storage()
            .instance()
            .set(&DataKey::CounterV1, &v1_counter);
        env.storage().instance().set(&DataKey::Version, &1u32);
    }

    // Call a v2 function which should trigger migration
    let name = String::from_slice(&env, "migrated");
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
    let v1_counter = 50u32;
    env.storage()
        .instance()
        .set(&DataKey::CounterV1, &v1_counter);
    env.storage().instance().set(&DataKey::Version, &1u32);

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
    let original_counter = 999u32;
    env.storage()
        .instance()
        .set(&DataKey::CounterV1, &original_counter);
    env.storage().instance().set(&DataKey::Version, &1u32);

    // Migrate to v2
    client.increment();

    // The v1 counter should be preserved and incremented by 1
    assert_eq!(client.get_counter(), original_counter + 1);
}

#[test]
fn test_v2_state_functions_without_v1_data() {
    let env = Env::default();
    let contract_id = env.register(StorageMigration, ());
    let client = StorageMigrationClient::new(&env, &contract_id);

    // Initialize with v2 state (no v1 migration)
    let initial_counter = 5;
    let name = String::from_slice(&env, "v2_init");

    client.init(&initial_counter, &name);

    // All operations should work without issues
    assert_eq!(client.get_counter(), 5);
    assert_eq!(client.get_name(), name);

    client.increment();
    assert_eq!(client.get_counter(), 6);

    let new_name = String::from_slice(&env, "v2_updated");
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
    assert_eq!(client.get_name(), String::from_slice(&env, "unknown"));
}

#[test]
fn test_migration_with_multiple_data_points() {
    let env = Env::default();
    let contract_id = env.register(StorageMigration, ());
    let client = StorageMigrationClient::new(&env, &contract_id);

    // Simulate complex v1 state
    env.storage()
        .instance()
        .set(&DataKey::CounterV1, &42u32);
    env.storage().instance().set(&DataKey::Version, &1u32);

    // Perform multiple v2 operations
    client.set_name(&String::from_slice(&env, "first"));
    assert_eq!(client.get_counter(), 42);
    assert_eq!(client.get_name(), String::from_slice(&env, "first"));

    client.increment();
    assert_eq!(client.get_counter(), 43);

    client.set_name(&String::from_slice(&env, "second"));
    assert_eq!(client.get_name(), String::from_slice(&env, "second"));

    // Verify version is still 2 (no re-migration)
    assert_eq!(client.version(), 2);
}

#[test]
#[should_panic]
fn test_init_twice_fails() {
    let env = Env::default();
    let client = StorageMigrationClient::new(&env, &env.register(StorageMigration, ()));

    client.init(&1, &String::from_slice(&env, "first"));
    client.init(&999, &String::from_slice(&env, "overwrite"));
}

#[test]
fn test_second_init_leaves_state_untouched() {
    let env = Env::default();
    let client = StorageMigrationClient::new(&env, &env.register(StorageMigration, ()));

    let name = String::from_slice(&env, "first");
    client.init(&1, &name);
    client.increment();

    assert!(client
        .try_init(&999, &String::from_slice(&env, "overwrite"))
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
    assert!(client.try_init(&0, &String::from_slice(&env, "reset")).is_err());

    // The migration example still works end to end.
    assert_eq!(client.get_counter(), 77);
    assert_eq!(client.get_name(), String::from_slice(&env, "migrated"));
    assert_eq!(client.version(), 2);
}
