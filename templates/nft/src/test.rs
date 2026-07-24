use super::*;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env, String};

fn setup(env: &Env) -> (NftContractClient<'_>, Address) {
    env.mock_all_auths();
    let admin = Address::generate(env);
    let contract_id = env.register(
        NftContract,
        (
            admin.clone(),
            String::from_str(env, "Forge NFT"),
            String::from_str(env, "FNFT"),
        ),
    );
    (NftContractClient::new(env, &contract_id), admin)
}

#[test]
fn metadata() {
    let env = Env::default();
    let (nft, admin) = setup(&env);
    assert_eq!(nft.name(), String::from_str(&env, "Forge NFT"));
    assert_eq!(nft.symbol(), String::from_str(&env, "FNFT"));
    assert_eq!(nft.admin(), admin);
}

#[test]
fn mint_and_owner_of() {
    let env = Env::default();
    let (nft, _admin) = setup(&env);
    let alice = Address::generate(&env);
    let uri = String::from_str(&env, "https://example.com/nft/1");

    nft.mint(&alice, &101, &uri);

    assert_eq!(nft.owner_of(&101), alice);
    assert_eq!(nft.balance_of(&alice), 1);
    assert_eq!(nft.token_uri(&101), uri);
}

#[test]
fn transfer() {
    let env = Env::default();
    let (nft, _admin) = setup(&env);
    let alice = Address::generate(&env);
    let bob = Address::generate(&env);
    let uri = String::from_str(&env, "https://example.com/nft/1");

    nft.mint(&alice, &1, &uri);
    assert_eq!(nft.owner_of(&1), alice);
    assert_eq!(nft.balance_of(&alice), 1);
    assert_eq!(nft.balance_of(&bob), 0);

    nft.transfer(&alice, &bob, &1);

    assert_eq!(nft.owner_of(&1), bob);
    assert_eq!(nft.balance_of(&alice), 0);
    assert_eq!(nft.balance_of(&bob), 1);
}

#[test]
#[should_panic]
fn unauthorized_transfer() {
    let env = Env::default();
    let (nft, _admin) = setup(&env);
    let alice = Address::generate(&env);
    let bob = Address::generate(&env);
    let carol = Address::generate(&env);
    let uri = String::from_str(&env, "https://example.com/nft/1");

    nft.mint(&alice, &1, &uri);

    // Bob tries to transfer Alice's token to Carol
    nft.transfer(&bob, &carol, &1);
}

#[test]
fn burn() {
    let env = Env::default();
    let (nft, _admin) = setup(&env);
    let alice = Address::generate(&env);
    let uri = String::from_str(&env, "https://example.com/nft/1");

    nft.mint(&alice, &1, &uri);
    assert_eq!(nft.balance_of(&alice), 1);

    nft.burn(&alice, &1);
    assert_eq!(nft.balance_of(&alice), 0);
}

#[test]
#[should_panic]
fn query_burned_token_panics() {
    let env = Env::default();
    let (nft, _admin) = setup(&env);
    let alice = Address::generate(&env);
    let uri = String::from_str(&env, "https://example.com/nft/1");

    nft.mint(&alice, &1, &uri);
    nft.burn(&alice, &1);

    // Token should no longer exist
    let _owner = nft.owner_of(&1);
}

#[test]
#[should_panic]
fn mint_duplicate_token_panics() {
    let env = Env::default();
    let (nft, _admin) = setup(&env);
    let alice = Address::generate(&env);
    let uri = String::from_str(&env, "https://example.com/nft/1");

    nft.mint(&alice, &1, &uri);
    nft.mint(&alice, &1, &uri);
}
