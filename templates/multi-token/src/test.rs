use super::*;
use soroban_sdk::testutils::storage::Instance as _;
use soroban_sdk::testutils::{Address as _, Ledger};
use soroban_sdk::{vec, Address, Env, String};

fn setup(env: &Env) -> (MultiTokenContractClient<'_>, Address) {
    env.mock_all_auths();
    let admin = Address::generate(env);
    let contract_id = env.register(
        MultiTokenContract,
        (admin.clone(), String::from_str(env, "Forge Multi-Token"), String::from_str(env, "FMT")),
    );
    (MultiTokenContractClient::new(env, &contract_id), admin)
}

#[test]
fn metadata() {
    let env = Env::default();
    let (mt, admin) = setup(&env);
    assert_eq!(mt.name(), String::from_str(&env, "Forge Multi-Token"));
    assert_eq!(mt.symbol(), String::from_str(&env, "FMT"));
    assert_eq!(mt.admin(), admin);
}

#[test]
fn mint_across_multiple_ids() {
    let env = Env::default();
    let (mt, _admin) = setup(&env);
    let alice = Address::generate(&env);
    mt.mint(&alice, &1, &100);
    mt.mint(&alice, &2, &50);
    mt.mint(&alice, &3, &1);
    assert_eq!(mt.balance_of(&alice, &1), 100);
    assert_eq!(mt.balance_of(&alice, &2), 50);
    assert_eq!(mt.balance_of(&alice, &3), 1);
    assert_eq!(mt.balance_of(&alice, &4), 0);
}

#[test]
fn transfer_across_multiple_ids() {
    let env = Env::default();
    let (mt, _admin) = setup(&env);
    let alice = Address::generate(&env);
    let bob = Address::generate(&env);
    mt.mint(&alice, &1, &100);
    mt.mint(&alice, &2, &50);
    mt.transfer(&alice, &bob, &1, &40);
    mt.transfer(&alice, &bob, &2, &50);
    assert_eq!(mt.balance_of(&alice, &1), 60);
    assert_eq!(mt.balance_of(&alice, &2), 0);
    assert_eq!(mt.balance_of(&bob, &1), 40);
    assert_eq!(mt.balance_of(&bob, &2), 50);
}

#[test]
fn burn_across_multiple_ids() {
    let env = Env::default();
    let (mt, _admin) = setup(&env);
    let alice = Address::generate(&env);
    mt.mint(&alice, &1, &100);
    mt.mint(&alice, &2, &50);
    mt.burn(&alice, &1, &30);
    mt.burn(&alice, &2, &50);
    assert_eq!(mt.balance_of(&alice, &1), 70);
    assert_eq!(mt.balance_of(&alice, &2), 0);
}

#[test]
fn balance_of_batch() {
    let env = Env::default();
    let (mt, _admin) = setup(&env);
    let alice = Address::generate(&env);
    let bob = Address::generate(&env);
    mt.mint(&alice, &1, &10);
    mt.mint(&bob, &2, &20);
    let owners = vec![&env, alice.clone(), bob.clone()];
    let ids = vec![&env, 1u32, 2u32];
    let balances = mt.balance_of_batch(&owners, &ids);
    assert_eq!(balances, vec![&env, 10, 20]);
}

#[test]
fn mint_batch_across_multiple_ids() {
    let env = Env::default();
    let (mt, _admin) = setup(&env);
    let alice = Address::generate(&env);
    let ids = vec![&env, 1u32, 2u32, 3u32];
    let amounts = vec![&env, 10i128, 20i128, 30i128];
    mt.mint_batch(&alice, &ids, &amounts);
    assert_eq!(mt.balance_of(&alice, &1), 10);
    assert_eq!(mt.balance_of(&alice, &2), 20);
    assert_eq!(mt.balance_of(&alice, &3), 30);
}

#[test]
fn transfer_batch_across_multiple_ids() {
    let env = Env::default();
    let (mt, _admin) = setup(&env);
    let alice = Address::generate(&env);
    let bob = Address::generate(&env);
    let ids = vec![&env, 1u32, 2u32];
    mt.mint_batch(&alice, &ids, &vec![&env, 10i128, 20i128]);
    mt.transfer_batch(&alice, &bob, &ids, &vec![&env, 4i128, 20i128]);
    assert_eq!(mt.balance_of(&alice, &1), 6);
    assert_eq!(mt.balance_of(&alice, &2), 0);
    assert_eq!(mt.balance_of(&bob, &1), 4);
    assert_eq!(mt.balance_of(&bob, &2), 20);
}

#[test]
fn burn_batch_across_multiple_ids() {
    let env = Env::default();
    let (mt, _admin) = setup(&env);
    let alice = Address::generate(&env);
    let ids = vec![&env, 1u32, 2u32];
    mt.mint_batch(&alice, &ids, &vec![&env, 10i128, 20i128]);
    mt.burn_batch(&alice, &ids, &vec![&env, 10i128, 5i128]);
    assert_eq!(mt.balance_of(&alice, &1), 0);
    assert_eq!(mt.balance_of(&alice, &2), 15);
}

#[test]
#[should_panic]
fn unauthorized_transfer_panics() {
    let env = Env::default();
    let (mt, _admin) = setup(&env);
    let alice = Address::generate(&env);
    let bob = Address::generate(&env);
    let carol = Address::generate(&env);
    mt.mint(&alice, &1, &10);
    mt.transfer(&bob, &carol, &1, &10);
}

#[test]
#[should_panic]
fn transfer_more_than_balance_panics() {
    let env = Env::default();
    let (mt, _admin) = setup(&env);
    let alice = Address::generate(&env);
    let bob = Address::generate(&env);
    mt.mint(&alice, &1, &10);
    mt.transfer(&alice, &bob, &1, &11);
}

#[test]
#[should_panic]
fn burn_more_than_balance_panics() {
    let env = Env::default();
    let (mt, _admin) = setup(&env);
    let alice = Address::generate(&env);
    mt.mint(&alice, &1, &10);
    mt.burn(&alice, &1, &11);
}

#[test]
#[should_panic]
fn mint_batch_mismatched_lengths_panics() {
    let env = Env::default();
    let (mt, _admin) = setup(&env);
    let alice = Address::generate(&env);
    let ids = vec![&env, 1u32, 2u32];
    let amounts = vec![&env, 10i128];
    mt.mint_batch(&alice, &ids, &amounts);
}

#[test]
#[should_panic]
fn transfer_negative_amount_panics() {
    let env = Env::default();
    let (mt, _admin) = setup(&env);
    let alice = Address::generate(&env);
    let bob = Address::generate(&env);
    mt.mint(&alice, &1, &10);
    mt.transfer(&alice, &bob, &1, &-1);
}

#[test]
fn balances_across_ids_are_independent() {
    let env = Env::default();
    let (mt, _admin) = setup(&env);
    let alice = Address::generate(&env);
    let bob = Address::generate(&env);
    mt.mint(&alice, &1, &100);
    mt.mint(&bob, &2, &200);
    assert_eq!(mt.balance_of(&alice, &2), 0);
    assert_eq!(mt.balance_of(&bob, &1), 0);
    assert_eq!(mt.balance_of(&alice, &1), 100);
    assert_eq!(mt.balance_of(&bob, &2), 200);
}

#[test]
fn instance_storage_outlives_the_default_ttl() {
    let env = Env::default();
    let (mt, _admin) = setup(&env);

    // Well past the default instance TTL, but within the extended one.
    env.ledger().with_mut(|li| li.sequence_number += 100_000);

    let alice = Address::generate(&env);
    mt.mint(&alice, &1, &10);
    assert_eq!(mt.balance_of(&alice, &1), 10);

    let ttl = env.as_contract(&mt.address, || env.storage().instance().get_ttl());
    assert!(ttl >= INSTANCE_TTL_THRESHOLD, "instance TTL was not extended: {}", ttl);
}
