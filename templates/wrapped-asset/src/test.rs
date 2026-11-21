extern crate std;

use super::*;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{token::StellarAssetClient, token::TokenClient, Address, Env};

struct Tok<'a> {
    address: Address,
    client: TokenClient<'a>,
    admin: StellarAssetClient<'a>,
}

fn make_token(env: &Env) -> Tok<'_> {
    let issuer = Address::generate(env);
    let sac = env.register_stellar_asset_contract_v2(issuer);
    let address = sac.address();
    Tok {
        client: TokenClient::new(env, &address),
        admin: StellarAssetClient::new(env, &address),
        address,
    }
}

fn setup(env: &Env) -> (WrappedAssetContractClient<'_>, Tok<'_>) {
    env.mock_all_auths();

    let token = make_token(env);
    let contract_id = env.register(WrappedAssetContract, (token.address.clone(),));
    let contract = WrappedAssetContractClient::new(env, &contract_id);

    (contract, token)
}

#[test]
fn wrap_mints_1_to_1_and_locks_underlying() {
    let env = Env::default();
    let (contract, token) = setup(&env);

    let user = Address::generate(&env);
    token.admin.mint(&user, &1_000);

    let total = contract.wrap(&user, &400);

    assert_eq!(total, 400);
    assert_eq!(contract.balance_of(&user), 400);
    assert_eq!(contract.total_supply(), 400);
    assert_eq!(token.client.balance(&user), 600);
    assert_eq!(token.client.balance(&contract.address), 400);
}

#[test]
fn unwrap_burns_1_to_1_and_releases_underlying() {
    let env = Env::default();
    let (contract, token) = setup(&env);

    let user = Address::generate(&env);
    token.admin.mint(&user, &1_000);
    contract.wrap(&user, &400);

    let total = contract.unwrap(&user, &150);

    assert_eq!(total, 250);
    assert_eq!(contract.balance_of(&user), 250);
    assert_eq!(contract.total_supply(), 250);
    assert_eq!(token.client.balance(&user), 750);
    assert_eq!(token.client.balance(&contract.address), 250);
}

#[test]
fn full_wrap_and_unwrap_round_trips_conserve_balance() {
    let env = Env::default();
    let (contract, token) = setup(&env);

    let user = Address::generate(&env);
    token.admin.mint(&user, &1_000);

    contract.wrap(&user, &1_000);
    assert_eq!(token.client.balance(&user), 0);
    assert_eq!(contract.total_supply(), 1_000);

    contract.unwrap(&user, &1_000);
    assert_eq!(token.client.balance(&user), 1_000);
    assert_eq!(contract.total_supply(), 0);
    assert_eq!(contract.balance_of(&user), 0);
    assert_eq!(token.client.balance(&contract.address), 0);
}

#[test]
fn total_supply_always_matches_locked_underlying() {
    let env = Env::default();
    let (contract, token) = setup(&env);

    let alice = Address::generate(&env);
    let bob = Address::generate(&env);
    token.admin.mint(&alice, &1_000);
    token.admin.mint(&bob, &1_000);

    contract.wrap(&alice, &300);
    contract.wrap(&bob, &500);
    contract.unwrap(&alice, &100);

    assert_eq!(contract.total_supply(), 700);
    assert_eq!(token.client.balance(&contract.address), 700);
}

#[test]
#[should_panic]
fn unwrap_more_than_balance_fails() {
    let env = Env::default();
    let (contract, token) = setup(&env);

    let user = Address::generate(&env);
    token.admin.mint(&user, &1_000);
    contract.wrap(&user, &100);

    contract.unwrap(&user, &200);
}

#[test]
#[should_panic]
fn initialize_twice_panics() {
    let env = Env::default();
    let (contract, token) = setup(&env);
    contract.initialize(&token.address);
}

#[test]
#[should_panic]
fn initialize_with_different_token_fails_after_setup() {
    let env = Env::default();
    let (contract, _token) = setup(&env);
    let other_token = make_token(&env);
    contract.initialize(&other_token.address);
}

#[test]
fn sep41_transfer_and_allowance_work_on_wrapped_balance() {
    let env = Env::default();
    let (contract, token) = setup(&env);

    let alice = Address::generate(&env);
    let bob = Address::generate(&env);
    let spender = Address::generate(&env);
    token.admin.mint(&alice, &1_000);

    contract.wrap(&alice, &500);
    assert_eq!(contract.balance(&alice), 500);

    let wrapper_token = TokenClient::new(&env, &contract.address);
    wrapper_token.transfer(&alice, &bob, &200);
    assert_eq!(contract.balance(&alice), 300);
    assert_eq!(contract.balance(&bob), 200);

    wrapper_token.approve(&alice, &spender, &150, &1_000);
    assert_eq!(wrapper_token.allowance(&alice, &spender), 150);

    wrapper_token.transfer_from(&spender, &alice, &bob, &100);
    assert_eq!(contract.balance(&alice), 200);
    assert_eq!(contract.balance(&bob), 300);
    assert_eq!(wrapper_token.allowance(&alice, &spender), 50);

    // Bob can unwrap the wrapped tokens he received via transfer.
    contract.unwrap(&bob, &300);
    assert_eq!(contract.balance(&bob), 0);
    assert_eq!(token.client.balance(&bob), 300);
}
