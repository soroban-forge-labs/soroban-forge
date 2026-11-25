use super::*;
use soroban_sdk::testutils::storage::Instance as _;
use soroban_sdk::testutils::{Address as _, Ledger};
use soroban_sdk::{Address, Env, String};

fn setup(env: &Env) -> (SoulboundContractClient<'_>, Address) {
    env.mock_all_auths();
    let admin = Address::generate(env);
    let contract_id = env.register(
        SoulboundContract,
        (
            admin.clone(),
            String::from_str(env, "Forge Soulbound"),
            String::from_str(env, "FSBT"),
        ),
    );
    (SoulboundContractClient::new(env, &contract_id), admin)
}

#[test]
fn metadata() {
    let env = Env::default();
    let (sbt, admin) = setup(&env);
    assert_eq!(sbt.name(), String::from_str(&env, "Forge Soulbound"));
    assert_eq!(sbt.symbol(), String::from_str(&env, "FSBT"));
    assert_eq!(sbt.admin(), admin);
}

#[test]
fn mint_and_owner_of() {
    let env = Env::default();
    let (sbt, _admin) = setup(&env);
    let alice = Address::generate(&env);
    let uri = String::from_str(&env, "https://example.com/sbt/1");

    sbt.mint(&alice, &101, &uri);

    assert_eq!(sbt.owner_of(&101), alice);
    assert_eq!(sbt.balance_of(&alice), 1);
    assert_eq!(sbt.token_uri(&101), uri);
}

#[test]
#[should_panic]
fn transfer_is_rejected() {
    let env = Env::default();
    let (sbt, _admin) = setup(&env);
    let alice = Address::generate(&env);
    let bob = Address::generate(&env);
    let uri = String::from_str(&env, "https://example.com/sbt/1");

    sbt.mint(&alice, &1, &uri);

    // Soulbound tokens can never be transferred, even by their owner.
    sbt.transfer(&alice, &bob, &1);
}

#[test]
#[should_panic]
fn mint_duplicate_token_panics() {
    let env = Env::default();
    let (sbt, _admin) = setup(&env);
    let alice = Address::generate(&env);
    let uri = String::from_str(&env, "https://example.com/sbt/1");

    sbt.mint(&alice, &1, &uri);
    sbt.mint(&alice, &1, &uri);
}

#[test]
fn burn() {
    let env = Env::default();
    let (sbt, _admin) = setup(&env);
    let alice = Address::generate(&env);
    let uri = String::from_str(&env, "https://example.com/sbt/1");

    sbt.mint(&alice, &1, &uri);
    assert_eq!(sbt.balance_of(&alice), 1);

    sbt.burn(&alice, &1);
    assert_eq!(sbt.balance_of(&alice), 0);
}

#[test]
#[should_panic]
fn query_burned_token_panics() {
    let env = Env::default();
    let (sbt, _admin) = setup(&env);
    let alice = Address::generate(&env);
    let uri = String::from_str(&env, "https://example.com/sbt/1");

    sbt.mint(&alice, &1, &uri);
    sbt.burn(&alice, &1);

    // Token should no longer exist
    let _owner = sbt.owner_of(&1);
}

#[test]
fn instance_storage_outlives_the_default_ttl() {
    let env = Env::default();
    let (sbt, _admin) = setup(&env);

    // Well past the default instance TTL, but within the extended one.
    env.ledger().with_mut(|li| li.sequence_number += 100_000);

    let alice = Address::generate(&env);
    sbt.mint(&alice, &1, &String::from_str(&env, "ipfs://badge-1"));
    assert_eq!(sbt.owner_of(&1), alice);

    let ttl = env.as_contract(&sbt.address, || env.storage().instance().get_ttl());
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
    let contract_id = env.register(SoulboundContract, (admin.clone(), String::from_str(&env, "Forge Soulbound"), String::from_str(&env, "FSBT")));
    let client = SoulboundContractClient::new(&env, &contract_id);
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
