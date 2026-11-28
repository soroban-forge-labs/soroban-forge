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
    let contract_id = env.register(MultiTokenContract, (admin.clone(), String::from_str(&env, "Forge Multi-Token"), String::from_str(&env, "FMT")));
    let client = MultiTokenContractClient::new(&env, &contract_id);
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

#[test]
fn single_and_batch_ops_publish_contractevents() {
    use soroban_sdk::testutils::Events as _;
    use soroban_sdk::{symbol_short, IntoVal};
    let env = Env::default();
    let (mt, admin) = setup(&env);
    let id = mt.address.clone();
    let alice = Address::generate(&env);
    let bob = Address::generate(&env);
    let ids = vec![&env, 1_u32, 2_u32];
    let amounts = vec![&env, 10_i128, 20_i128];

    mt.mint(&alice, &1, &100);
    let expected = (
        id.clone(),
        (symbol_short!("mint"), admin.clone(), alice.clone(), 1_u32).into_val(&env),
        100_i128.into_val(&env),
    );
    assert_eq!(env.events().all(), vec![&env, expected]);

    mt.mint_batch(&alice, &ids, &amounts);
    let expected = (
        id.clone(),
        (symbol_short!("mintbatch"), admin.clone(), alice.clone()).into_val(&env),
        (ids.clone(), amounts.clone()).into_val(&env),
    );
    assert_eq!(env.events().all(), vec![&env, expected]);

    mt.transfer(&alice, &bob, &1, &40);
    let expected = (
        id.clone(),
        (symbol_short!("transfer"), alice.clone(), bob.clone(), 1_u32).into_val(&env),
        40_i128.into_val(&env),
    );
    assert_eq!(env.events().all(), vec![&env, expected]);

    mt.transfer_batch(&alice, &bob, &ids, &amounts);
    let expected = (
        id.clone(),
        (symbol_short!("xferbatch"), alice.clone(), bob.clone()).into_val(&env),
        (ids.clone(), amounts.clone()).into_val(&env),
    );
    assert_eq!(env.events().all(), vec![&env, expected]);

    mt.burn(&bob, &1, &5);
    let expected = (
        id.clone(),
        (symbol_short!("burn"), bob.clone(), 1_u32).into_val(&env),
        5_i128.into_val(&env),
    );
    assert_eq!(env.events().all(), vec![&env, expected]);

    mt.burn_batch(&bob, &ids, &amounts);
    let expected = (
        id,
        (symbol_short!("burnbatch"), bob.clone()).into_val(&env),
        (ids, amounts).into_val(&env),
    );
    assert_eq!(env.events().all(), vec![&env, expected]);
}
