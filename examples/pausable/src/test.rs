use super::*;
use soroban_sdk::testutils::storage::Instance as _;
use soroban_sdk::testutils::{Address as _, Ledger as _, MockAuth, MockAuthInvoke};
use soroban_sdk::{Address, Env, IntoVal};

fn setup(env: &Env) -> (PausableContractClient<'_>, Address) {
    env.mock_all_auths();
    let admin = Address::generate(env);
    let contract_id = env.register(PausableContract, (admin.clone(),));
    (PausableContractClient::new(env, &contract_id), admin)
}

#[test]
fn constructor_sets_admin_and_starts_unpaused() {
    let env = Env::default();
    let (client, admin) = setup(&env);
    assert_eq!(client.admin(), admin);
    assert!(!client.is_paused());
}

#[test]
fn admin_can_pause() {
    let env = Env::default();
    let (client, _admin) = setup(&env);

    client.pause();

    assert!(client.is_paused());
}

#[test]
fn guarded_entrypoints_are_rejected_while_paused() {
    let env = Env::default();
    let (client, _admin) = setup(&env);
    assert_eq!(client.increment(&5), 5);

    client.pause();

    assert_eq!(client.try_increment(&1), Err(Ok(Error::Paused)));
    assert_eq!(client.try_reset(), Err(Ok(Error::Paused)));
    // Reads stay available, and the rejected calls changed nothing.
    assert_eq!(client.count(), 5);
}

#[test]
fn admin_can_unpause_and_guarded_entrypoints_work_again() {
    let env = Env::default();
    let (client, _admin) = setup(&env);
    client.pause();

    client.unpause();

    assert!(!client.is_paused());
    assert_eq!(client.increment(&2), 2);
    client.reset();
    assert_eq!(client.count(), 0);
}

#[test]
fn non_admin_cannot_pause() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let mallory = Address::generate(&env);
    let contract_id = env.register(PausableContract, (admin.clone(),));
    let client = PausableContractClient::new(&env, &contract_id);

    // Only mallory signs — the contract requires the admin's authorization,
    // which is never mocked here, so the call fails.
    let result = client
        .mock_auths(&[MockAuth {
            address: &mallory,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "pause",
                args: ().into_val(&env),
                sub_invokes: &[],
            },
        }])
        .try_pause();

    assert!(result.is_err(), "a non-admin must not be able to pause");
    assert!(!client.is_paused(), "the contract must still be unpaused");
}

#[test]
fn pausing_twice_is_rejected() {
    let env = Env::default();
    let (client, _admin) = setup(&env);
    client.pause();

    assert_eq!(client.try_pause(), Err(Ok(Error::AlreadyPaused)));
}

#[test]
fn unpausing_an_unpaused_contract_is_rejected() {
    let env = Env::default();
    let (client, _admin) = setup(&env);

    assert_eq!(client.try_unpause(), Err(Ok(Error::NotPaused)));
}

#[test]
fn storage_ttl_extended_on_state_changes() {
    let env = Env::default();
    let (client, _admin) = setup(&env);

    // Constructor extended instance TTL to 30 days
    env.as_contract(&client.address, || {
        assert_eq!(env.storage().instance().get_ttl(), INSTANCE_TTL);
    });

    // Advance ledger past the default TTL (4095 ledgers) to 10,000 ledgers
    let current_ledger = env.ledger().sequence();
    env.ledger().set_sequence_number(current_ledger + 10_000);

    // The contract instance survives and guarded functions still work
    assert_eq!(client.increment(&5), 5);
    assert_eq!(client.count(), 5);

    // Advance ledger past the extension threshold (more than 1 day in ledgers)
    let current_ledger = env.ledger().sequence();
    env.ledger().set_sequence_number(current_ledger + DAY_IN_LEDGERS + 10);

    // Verify remaining TTL dropped below threshold
    env.as_contract(&client.address, || {
        assert!(env.storage().instance().get_ttl() < INSTANCE_TTL_THRESHOLD);
    });

    // Calling pause re-extends instance TTL back to INSTANCE_TTL
    client.pause();
    assert!(client.is_paused());

    env.as_contract(&client.address, || {
        assert_eq!(env.storage().instance().get_ttl(), INSTANCE_TTL);
    });

    // Advance past threshold again and test unpause
    let current_ledger = env.ledger().sequence();
    env.ledger().set_sequence_number(current_ledger + DAY_IN_LEDGERS + 10);

    client.unpause();
    assert!(!client.is_paused());

    env.as_contract(&client.address, || {
        assert_eq!(env.storage().instance().get_ttl(), INSTANCE_TTL);
    });

    // Advance past threshold again and test reset
    let current_ledger = env.ledger().sequence();
    env.ledger().set_sequence_number(current_ledger + DAY_IN_LEDGERS + 10);

    client.reset();
    assert_eq!(client.count(), 0);

    env.as_contract(&client.address, || {
        assert_eq!(env.storage().instance().get_ttl(), INSTANCE_TTL);
    });
}

#[test]
fn increment_overflow_returns_error_and_leaves_count_unchanged() {
    let env = Env::default();
    let (client, _admin) = setup(&env);

    // Drive the counter to u32::MAX.
    assert_eq!(client.increment(&(u32::MAX - 1)), u32::MAX - 1);
    assert_eq!(client.increment(&1), u32::MAX);

    // The next increment would overflow and must fail with a contract error.
    assert_eq!(client.try_increment(&1), Err(Ok(Error::Overflow)));
    assert_eq!(client.count(), u32::MAX);
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
    use soroban_sdk::IntoVal as _;
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(PausableContract, (admin.clone(),));
    let client = PausableContractClient::new(&env, &contract_id);
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
