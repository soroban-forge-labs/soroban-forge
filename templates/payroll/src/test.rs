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

fn setup(env: &Env) -> (StreamingContractClient<'_>, Tok<'_>, Address, Address, u64) {
    env.mock_all_auths();

    let start_ts = 1_000_000;
    env.ledger().with_mut(|li| li.timestamp = start_ts);

    let token = make_token(env);
    let sender = Address::generate(env);
    let recipient = Address::generate(env);
    let duration_seconds = 4000_u64;

    let contract_id = env.register(StreamingContract, ());
    let contract = StreamingContractClient::new(env, &contract_id);

    contract.initialize(&sender, &token.address, &recipient, &duration_seconds, &100_000);

    token.admin.mint(&sender, &100_000);
    contract.fund();

    (contract, token, sender, recipient, start_ts)
}

#[test]
fn nothing_claimable_at_start() {
    let env = Env::default();
    let (contract, _token, _sender, _recipient, _start_ts) = setup(&env);

    assert_eq!(contract.get_streamed_amount(), 0);
    assert_eq!(contract.get_claimable_amount(), 0);
}

#[test]
fn mid_stream_partial_claim() {
    let env = Env::default();
    let (contract, token, _sender, recipient, start_ts) = setup(&env);

    // Halfway through the 4000-second stream.
    env.ledger().with_mut(|li| li.timestamp = start_ts + 2000);
    assert_eq!(contract.get_streamed_amount(), 50_000);
    assert_eq!(contract.get_claimable_amount(), 50_000);

    let claimed = contract.claim();
    assert_eq!(claimed, 50_000);
    assert_eq!(token.client.balance(&recipient), 50_000);
    assert_eq!(contract.get_claimable_amount(), 0);
}

#[test]
fn post_stream_claims_full_amount() {
    let env = Env::default();
    let (contract, token, _sender, recipient, start_ts) = setup(&env);

    // Jump well past the stream end.
    env.ledger().with_mut(|li| li.timestamp = start_ts + 10_000);
    assert_eq!(contract.get_streamed_amount(), 100_000);

    let claimed = contract.claim();
    assert_eq!(claimed, 100_000);
    assert_eq!(token.client.balance(&recipient), 100_000);
}

#[test]
#[should_panic(expected = "nothing to claim")]
fn double_claim_fails() {
    let env = Env::default();
    let (contract, _token, _sender, _recipient, start_ts) = setup(&env);

    env.ledger().with_mut(|li| li.timestamp = start_ts + 10_000);
    contract.claim();
    contract.claim();
}

#[test]
#[should_panic(expected = "already initialized")]
fn initialize_twice_is_rejected() {
    let env = Env::default();
    let (contract, token, sender, recipient, _start_ts) = setup(&env);
    contract.initialize(&sender, &token.address, &recipient, &4000, &100_000);
}

#[test]
fn an_attacker_cannot_reinitialize_to_redirect_the_stream() {
    let env = Env::default();
    let (contract, token, _sender, recipient, start_ts) = setup(&env);

    // Part of the stream has already been claimed.
    env.ledger().with_mut(|li| li.timestamp = start_ts + 1000);
    assert_eq!(contract.claim(), 25_000);

    let attacker = Address::generate(&env);
    let result = contract.try_initialize(&attacker, &token.address, &attacker, &1, &100_000);
    assert!(result.is_err(), "re-initialization must fail");

    // The schedule and the claimed amount are untouched: the rest streams to
    // the original recipient, and nothing goes to the attacker.
    env.ledger().with_mut(|li| li.timestamp = start_ts + 4000);
    assert_eq!(contract.get_claimable_amount(), 75_000);
    assert_eq!(contract.claim(), 75_000);
    assert_eq!(token.client.balance(&recipient), 100_000);
    assert_eq!(token.client.balance(&attacker), 0);
}

#[test]
#[should_panic(expected = "already funded")]
fn fund_twice_is_rejected() {
    let env = Env::default();
    let (contract, token, sender, _recipient, _start_ts) = setup(&env);
    token.admin.mint(&sender, &100_000);
    contract.fund();
}

#[test]
fn sender_can_cancel_stream() {
    let env = Env::default();
    let (contract, token, sender, recipient, start_ts) = setup(&env);

    env.ledger().with_mut(|li| li.timestamp = start_ts + 2000);
    let (recipient_paid, sender_refunded) = contract.cancel();
    assert_eq!(recipient_paid, 50_000);
    assert_eq!(sender_refunded, 50_000);
    assert_eq!(token.client.balance(&recipient), 50_000);
    assert_eq!(token.client.balance(&sender), 50_000);
    assert_eq!(contract.get_claimable_amount(), 0);
}
