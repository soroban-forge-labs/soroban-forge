use super::*;
use soroban_sdk::testutils::Address as _; // minor: keep import explicit
use soroban_sdk::testutils::storage::Instance as _;
use soroban_sdk::testutils::Ledger;
use soroban_sdk::{Address, Env, String};

fn setup(env: &Env) -> (TokenContractClient<'_>, Address) {
    env.mock_all_auths();
    let admin = Address::generate(env);
    let contract_id = env.register(
        TokenContract,
        (
            admin.clone(),
            7u32,
            String::from_str(env, "Forge Token"),
            String::from_str(env, "FORGE"),
        ),
    );
    (TokenContractClient::new(env, &contract_id), admin)
}

#[test]
fn metadata() {
    let env = Env::default();
    let (token, admin) = setup(&env);
    assert_eq!(token.decimals(), 7);
    assert_eq!(token.name(), String::from_str(&env, "Forge Token"));
    assert_eq!(token.symbol(), String::from_str(&env, "FORGE"));
    assert_eq!(token.admin(), admin);
}

#[test]
fn mint_and_transfer() {
    let env = Env::default();
    let (token, _admin) = setup(&env);
    let alice = Address::generate(&env);
    let bob = Address::generate(&env);

    token.mint(&alice, &1000);
    assert_eq!(token.balance(&alice), 1000);
    assert_eq!(token.balance(&bob), 0);

    token.transfer(&alice, &bob, &400);
    assert_eq!(token.balance(&alice), 600);
    assert_eq!(token.balance(&bob), 400);
}

#[test]
fn approve_and_transfer_from() {
    let env = Env::default();
    let (token, _admin) = setup(&env);
    let alice = Address::generate(&env);
    let bob = Address::generate(&env);
    let carol = Address::generate(&env);

    token.mint(&alice, &1000);
    token.approve(&alice, &bob, &500, &200);
    assert_eq!(token.allowance(&alice, &bob), 500);

    token.transfer_from(&bob, &alice, &carol, &300);
    assert_eq!(token.balance(&alice), 700);
    assert_eq!(token.balance(&carol), 300);
    assert_eq!(token.allowance(&alice, &bob), 200);
}

#[test]
fn burn() {
    let env = Env::default();
    let (token, _admin) = setup(&env);
    let alice = Address::generate(&env);

    token.mint(&alice, &1000);
    token.burn(&alice, &250);
    assert_eq!(token.balance(&alice), 750);
}

#[test]
#[should_panic]
fn transfer_more_than_balance_panics() {
    let env = Env::default();
    let (token, _admin) = setup(&env);
    let alice = Address::generate(&env);
    let bob = Address::generate(&env);

    token.mint(&alice, &10);
    token.transfer(&alice, &bob, &11);
}

#[test]
#[should_panic]
fn transfer_from_beyond_allowance_panics() {
    let env = Env::default();
    let (token, _admin) = setup(&env);
    let alice = Address::generate(&env);
    let bob = Address::generate(&env);

    token.mint(&alice, &1000);
    token.approve(&alice, &bob, &100, &200);
    token.transfer_from(&bob, &alice, &bob, &101);
}

#[test]
fn admin_handover_propose_and_accept() {
    let env = Env::default();
    let (token, admin) = setup(&env);
    let new_admin = Address::generate(&env);

    token.propose_admin(&new_admin);
    // Admin is unchanged until the proposal is accepted.
    assert_eq!(token.admin(), admin);

    token.accept_admin();
    assert_eq!(token.admin(), new_admin);

    // The new admin can mint; the proposal is cleared.
    let alice = Address::generate(&env);
    token.mint(&alice, &5);
    assert_eq!(token.balance(&alice), 5);
}

#[test]
#[should_panic]
fn accept_without_proposal_panics() {
    let env = Env::default();
    let (token, _admin) = setup(&env);
    token.accept_admin();
}

#[test]
#[should_panic]
fn accept_by_non_pending_address_fails() {
    let env = Env::default();
    let (token, _admin) = setup(&env);
    let new_admin = Address::generate(&env);
    token.propose_admin(&new_admin);

    // Only the pending admin's auth satisfies accept_admin.
    env.set_auths(&[]);
    token.accept_admin();
}

#[test]
#[should_panic]
fn cancel_clears_proposal() {
    let env = Env::default();
    let (token, admin) = setup(&env);
    let new_admin = Address::generate(&env);
    token.propose_admin(&new_admin);
    token.cancel_admin_proposal();
    assert_eq!(token.admin(), admin);

    // Nothing left to accept.
    token.accept_admin();
}

#[test]
fn instance_storage_outlives_the_default_ttl() {
    let env = Env::default();
    let (token, _admin) = setup(&env);

    // Well past the default instance TTL, but within the extended one.
    env.ledger().with_mut(|li| li.sequence_number += 100_000);

    let alice = Address::generate(&env);
    token.mint(&alice, &1_000);
    assert_eq!(token.balance(&alice), 1_000);

    let ttl = env.as_contract(&token.address, || env.storage().instance().get_ttl());
    assert!(ttl >= INSTANCE_TTL_THRESHOLD, "instance TTL was not extended: {}", ttl);
}
