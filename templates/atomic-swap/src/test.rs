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

fn setup(env: &Env) -> (AtomicSwapClient<'_>, Tok<'_>, Tok<'_>, Address, Address) {
    env.mock_all_auths();
    let token_a = make_token(env);
    let token_b = make_token(env);
    let swap_id = env.register(AtomicSwap, ());
    let swap = AtomicSwapClient::new(env, &swap_id);
    let alice = Address::generate(env);
    let bob = Address::generate(env);
    (swap, token_a, token_b, alice, bob)
}

#[test]
fn successful_two_party_swap() {
    let env = Env::default();
    let (swap, token_a, token_b, alice, bob) = setup(&env);

    // Fund both parties.
    token_a.admin.mint(&alice, &1000);
    token_b.admin.mint(&bob, &500);

    // Alice sends 100 of token_a to Bob; Bob sends 50 of token_b to Alice.
    swap.swap(
        &alice,
        &bob,
        &token_a.address,
        &token_b.address,
        &100,
        &50,
    );

    // Alice: 900 A, 50 B. Bob: 100 A, 450 B.
    assert_eq!(token_a.client.balance(&alice), 900);
    assert_eq!(token_b.client.balance(&alice), 50);
    assert_eq!(token_a.client.balance(&bob), 100);
    assert_eq!(token_b.client.balance(&bob), 450);
}

#[test]
fn swap_full_balances() {
    let env = Env::default();
    let (swap, token_a, token_b, alice, bob) = setup(&env);

    token_a.admin.mint(&alice, &1000);
    token_b.admin.mint(&bob, &2000);

    swap.swap(
        &alice,
        &bob,
        &token_a.address,
        &token_b.address,
        &1000,
        &2000,
    );

    assert_eq!(token_a.client.balance(&alice), 0);
    assert_eq!(token_b.client.balance(&alice), 2000);
    assert_eq!(token_a.client.balance(&bob), 1000);
    assert_eq!(token_b.client.balance(&bob), 0);
}

#[test]
#[should_panic]
fn swap_fails_with_insufficient_balance() {
    let env = Env::default();
    let (swap, token_a, token_b, alice, bob) = setup(&env);

    token_a.admin.mint(&alice, &10);
    token_b.admin.mint(&bob, &500);

    // Alice only has 10 but tries to swap 100.
    swap.swap(
        &alice,
        &bob,
        &token_a.address,
        &token_b.address,
        &100,
        &50,
    );
}

#[test]
#[should_panic]
fn swap_fails_with_zero_amount() {
    let env = Env::default();
    let (swap, token_a, token_b, alice, bob) = setup(&env);

    token_a.admin.mint(&alice, &1000);
    token_b.admin.mint(&bob, &500);

    swap.swap(
        &alice,
        &bob,
        &token_a.address,
        &token_b.address,
        &0,
        &50,
    );
}

#[test]
#[should_panic]
fn swap_fails_when_party_a_equals_party_b() {
    let env = Env::default();
    let (swap, token_a, token_b, alice, _bob) = setup(&env);

    token_a.admin.mint(&alice, &1000);
    token_b.admin.mint(&alice, &500);

    swap.swap(
        &alice,
        &alice,
        &token_a.address,
        &token_b.address,
        &100,
        &50,
    );
}

#[test]
#[should_panic]
fn swap_fails_when_token_a_equals_token_b() {
    let env = Env::default();
    let (swap, token_a, _token_b, alice, bob) = setup(&env);

    token_a.admin.mint(&alice, &1000);
    token_a.admin.mint(&bob, &500);

    swap.swap(
        &alice,
        &bob,
        &token_a.address,
        &token_a.address,
        &100,
        &50,
    );
}
