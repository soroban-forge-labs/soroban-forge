use super::*;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Bytes, BytesN, Env, IntoVal, Symbol, TryIntoVal};

fn setup(env: &Env) -> (UpgradeableContractClient<'_>, Address) {
    env.mock_all_auths();
    let admin = Address::generate(env);
    let contract_id = env.register(UpgradeableContract, (admin.clone(),));
    (UpgradeableContractClient::new(env, &contract_id), admin)
}

/// Upload empty WASM to get a valid hash for testing.
/// In test mode the host accepts empty WASM blobs (see `soroban-env-host`'s
/// `upload_contract_wasm` — the `testutils` feature allows zero-byte payloads).
fn test_wasm_hash(env: &Env) -> BytesN<32> {
    env.deployer().upload_contract_wasm(Bytes::new(env))
}

#[test]
fn constructor_sets_admin() {
    let env = Env::default();
    let (client, admin) = setup(&env);
    assert_eq!(client.admin(), admin);
}

#[test]
fn version_starts_at_one() {
    let env = Env::default();
    let (client, _admin) = setup(&env);
    assert_eq!(client.version(), 1);
}

#[test]
fn admin_can_upgrade() {
    let env = Env::default();
    let (client, _admin) = setup(&env);
    let wasm_hash = test_wasm_hash(&env);
    client.upgrade(&wasm_hash);
}

#[test]
fn upgrade_emits_event_with_new_wasm_hash() {
    let env = Env::default();
    let (client, _admin) = setup(&env);
    let wasm_hash = test_wasm_hash(&env);

    client.upgrade(&wasm_hash);

    let events = env.events().all();
    let mut found = false;
    for event in events.iter() {
        let (_contract, topics, data) = event;
        // The first topic is the event name symbol.
        if let Some(topic) = topics.get(0) {
            let name: Result<Symbol, _> = topic.try_into_val(&env);
            if let Ok(name) = name {
                if name == Symbol::new(&env, "Upgraded") {
                    let emitted_hash: BytesN<32> = data.try_into_val(&env).unwrap();
                    assert_eq!(emitted_hash, wasm_hash);
                    found = true;
                }
            }
        }
    }
    assert!(found, "expected an Upgraded event to be published");
}

#[test]
fn upgrade_bumps_version() {
    let env = Env::default();
    let (client, _admin) = setup(&env);
    let wasm_hash = test_wasm_hash(&env);

    assert_eq!(client.version(), 1);
    client.upgrade(&wasm_hash);
    assert_eq!(client.version(), 2);
}

#[test]
#[should_panic(expected = "HostError")]
fn non_admin_cannot_upgrade() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(UpgradeableContract, (admin.clone(),));
    let client = UpgradeableContractClient::new(&env, &contract_id);
    let wasm_hash = BytesN::from_array(&env, &[0; 32]);
    client.upgrade(&wasm_hash);
}

#[test]
fn propose_sets_pending_admin() {
    let env = Env::default();
    let (client, admin) = setup(&env);
    let new_admin = Address::generate(&env);
    client.propose_admin(&new_admin);
    assert_eq!(client.pending_admin(), Some(new_admin));
    assert_eq!(client.admin(), admin);
}

#[test]
fn pending_admin_can_accept() {
    let env = Env::default();
    let (client, _admin) = setup(&env);
    let new_admin = Address::generate(&env);
    client.propose_admin(&new_admin);
    client.accept_admin();
    assert_eq!(client.admin(), new_admin);
    assert_eq!(client.pending_admin(), None);
}

#[test]
fn admin_can_cancel_proposal() {
    let env = Env::default();
    let (client, admin) = setup(&env);
    let new_admin = Address::generate(&env);
    client.propose_admin(&new_admin);
    client.cancel_admin_proposal();
    assert_eq!(client.pending_admin(), None);
    assert_eq!(client.admin(), admin);
    assert_eq!(
        client.try_accept_admin().unwrap_err(),
        Ok(Error::NoPendingAdmin)
    );
}

#[test]
#[should_panic(expected = "HostError")]
fn non_pending_address_cannot_accept() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(UpgradeableContract, (admin.clone(),));
    let client = UpgradeableContractClient::new(&env, &contract_id);
    let new_admin = Address::generate(&env);
    // Authorize only the proposal; the pending admin's auth is never granted.
    env.mock_auths(&[soroban_sdk::testutils::MockAuth {
        address: &admin,
        invoke: &soroban_sdk::testutils::MockAuthInvoke {
            contract: &contract_id,
            fn_name: "propose_admin",
            args: (new_admin.clone(),).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    client.propose_admin(&new_admin);
    client.accept_admin();
}
