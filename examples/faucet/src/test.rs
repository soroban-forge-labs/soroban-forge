extern crate std;

use super::*;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{testutils::Ledger, token::StellarAssetClient, token::TokenClient, Address, Env};

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

fn setup(env: &Env) -> (FaucetContractClient<'_>, Tok<'_>, Address, u64) {
    env.mock_all_auths();

    let start_ts = 1_000_000;
    env.ledger().with_mut(|li| li.timestamp = start_ts);

    let token = make_token(env);
    let admin = Address::generate(env);
    let amount_per_claim = 100_i128;
    let cooldown_seconds = 3600_u64;

    let contract_id = env.register(FaucetContract, ());
    let contract = FaucetContractClient::new(env, &contract_id);

    contract.initialize(&admin, &token.address, &amount_per_claim, &cooldown_seconds);

    token.admin.mint(&admin, &1_000_000);
    contract.fund(&1_000_000);

    (contract, token, admin, start_ts)
}

#[test]
fn dispense_transfers_fixed_amount() {
    let env = Env::default();
    let (contract, token, _admin, _start_ts) = setup(&env);

    let user = Address::generate(&env);
    let claimed = contract.claim(&user);

    assert_eq!(claimed, 100);
    assert_eq!(token.client.balance(&user), 100);
}

#[test]
fn second_claim_after_cooldown_succeeds() {
    let env = Env::default();
    let (contract, token, _admin, start_ts) = setup(&env);

    let user = Address::generate(&env);
    contract.claim(&user);

    env.ledger().with_mut(|li| li.timestamp = start_ts + 3600);
    contract.claim(&user);

    assert_eq!(token.client.balance(&user), 200);
}

#[test]
#[should_panic]
fn claim_before_cooldown_rejected() {
    let env = Env::default();
    let (contract, _token, _admin, start_ts) = setup(&env);

    let user = Address::generate(&env);
    contract.claim(&user);

    env.ledger().with_mut(|li| li.timestamp = start_ts + 1000);
    contract.claim(&user);
}

#[test]
fn cooldown_is_per_address() {
    let env = Env::default();
    let (contract, token, _admin, _start_ts) = setup(&env);

    let alice = Address::generate(&env);
    let bob = Address::generate(&env);

    contract.claim(&alice);
    contract.claim(&bob);

    assert_eq!(token.client.balance(&alice), 100);
    assert_eq!(token.client.balance(&bob), 100);
}

#[test]
#[should_panic]
fn initialize_twice_is_rejected() {
    let env = Env::default();
    let (contract, token, admin, _start_ts) = setup(&env);
    contract.initialize(&admin, &token.address, &100, &3600);
}

#[test]
fn an_attacker_cannot_reinitialize_to_drain_the_faucet() {
    let env = Env::default();
    let (contract, token, _admin, _start_ts) = setup(&env);
    let attacker = Address::generate(&env);

    // Whole balance per claim, no cooldown.
    let result = contract.try_initialize(&attacker, &token.address, &1_000_000, &0);
    assert!(result.is_err(), "re-initialization must fail");

    // The original limits still apply: one claim pays 100, and a second claim
    // before the cooldown is refused.
    assert_eq!(contract.claim(&attacker), 100);
    assert!(contract.try_claim(&attacker).is_err());
    assert_eq!(token.client.balance(&attacker), 100);
}
