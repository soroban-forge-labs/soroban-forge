use super::*;
use soroban_sdk::testutils::{Address as _, MockAuth, MockAuthInvoke};
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
