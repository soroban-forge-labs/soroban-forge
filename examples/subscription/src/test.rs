extern crate std;

use super::*;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{testutils::Ledger, token::StellarAssetClient, token::TokenClient, Address, Env};

/// Amount charged per period.
const AMOUNT: i128 = 100;
/// Billing interval in seconds (30 days).
const INTERVAL: u64 = 2_592_000;
/// Ledger timestamp the tests start from.
const START_TS: u64 = 1_000_000;
/// Balance and allowance the subscriber starts with — five periods' worth.
const FUNDED: i128 = 500;

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

struct Fixture<'a> {
    contract: SubscriptionContractClient<'a>,
    token: Tok<'a>,
    merchant: Address,
    subscriber: Address,
}

/// A configured plan with a funded subscriber who has approved the contract
/// for five periods — the allowance is what makes the recurring pull possible.
fn setup(env: &Env) -> Fixture<'_> {
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = START_TS);

    let token = make_token(env);
    let merchant = Address::generate(env);
    let subscriber = Address::generate(env);
    token.admin.mint(&subscriber, &FUNDED);

    let contract_id = env.register(SubscriptionContract, ());
    let contract = SubscriptionContractClient::new(env, &contract_id);
    contract.initialize(&merchant, &token.address, &AMOUNT, &INTERVAL);

    token
        .client
        .approve(&subscriber, &contract_id, &FUNDED, &10_000);

    Fixture {
        contract,
        token,
        merchant,
        subscriber,
    }
}

#[test]
fn subscribing_charges_the_first_period() {
    let env = Env::default();
    let f = setup(&env);

    f.contract.subscribe(&f.subscriber);

    assert!(f.contract.is_active(&f.subscriber));
    assert_eq!(f.token.client.balance(&f.merchant), AMOUNT);
    assert_eq!(f.token.client.balance(&f.subscriber), FUNDED - AMOUNT);

    let sub = f.contract.subscription(&f.subscriber);
    assert_eq!(sub.charges, 1);
    assert_eq!(sub.next_charge, START_TS + INTERVAL);
    // The next period is not due yet.
    assert!(!f.contract.is_due(&f.subscriber));
}

#[test]
fn charges_once_the_interval_has_elapsed() {
    let env = Env::default();
    let f = setup(&env);
    f.contract.subscribe(&f.subscriber);

    env.ledger()
        .with_mut(|li| li.timestamp = START_TS + INTERVAL);
    assert!(f.contract.is_due(&f.subscriber));

    assert_eq!(f.contract.charge(&f.subscriber), AMOUNT);
    assert_eq!(f.token.client.balance(&f.merchant), AMOUNT * 2);
    assert_eq!(f.token.client.balance(&f.subscriber), FUNDED - AMOUNT * 2);

    let sub = f.contract.subscription(&f.subscriber);
    assert_eq!(sub.charges, 2);
    assert_eq!(sub.next_charge, START_TS + INTERVAL * 2);
    assert!(!f.contract.is_due(&f.subscriber));
}

#[test]
#[should_panic(expected = "interval has not elapsed")]
fn charging_before_the_interval_elapses_fails() {
    let env = Env::default();
    let f = setup(&env);
    f.contract.subscribe(&f.subscriber);

    // One second short of the due date.
    env.ledger()
        .with_mut(|li| li.timestamp = START_TS + INTERVAL - 1);
    f.contract.charge(&f.subscriber);
}

#[test]
#[should_panic(expected = "interval has not elapsed")]
fn charging_twice_in_the_same_period_fails() {
    let env = Env::default();
    let f = setup(&env);
    f.contract.subscribe(&f.subscriber);

    env.ledger()
        .with_mut(|li| li.timestamp = START_TS + INTERVAL);
    f.contract.charge(&f.subscriber);
    f.contract.charge(&f.subscriber);
}

#[test]
fn a_late_charge_does_not_skip_a_period() {
    let env = Env::default();
    let f = setup(&env);
    f.contract.subscribe(&f.subscriber);

    // The merchant is late: two and a half intervals have passed.
    env.ledger()
        .with_mut(|li| li.timestamp = START_TS + INTERVAL * 2 + INTERVAL / 2);

    // Each charge advances by exactly one interval, so both owed periods can
    // still be collected — one call each.
    assert_eq!(f.contract.charge(&f.subscriber), AMOUNT);
    assert!(f.contract.is_due(&f.subscriber));
    assert_eq!(f.contract.charge(&f.subscriber), AMOUNT);
    assert!(!f.contract.is_due(&f.subscriber));

    assert_eq!(f.token.client.balance(&f.merchant), AMOUNT * 3);
    assert_eq!(f.contract.subscription(&f.subscriber).charges, 3);
}

#[test]
fn cancelling_deactivates_the_subscription() {
    let env = Env::default();
    let f = setup(&env);
    f.contract.subscribe(&f.subscriber);

    f.contract.cancel(&f.subscriber);

    assert!(!f.contract.is_active(&f.subscriber));
    // Even once the interval has elapsed, nothing is due any more.
    env.ledger()
        .with_mut(|li| li.timestamp = START_TS + INTERVAL);
    assert!(!f.contract.is_due(&f.subscriber));
    // Only the first period was ever paid.
    assert_eq!(f.token.client.balance(&f.merchant), AMOUNT);
}

#[test]
#[should_panic(expected = "subscription is not active")]
fn charging_a_cancelled_subscription_fails() {
    let env = Env::default();
    let f = setup(&env);
    f.contract.subscribe(&f.subscriber);
    f.contract.cancel(&f.subscriber);

    env.ledger()
        .with_mut(|li| li.timestamp = START_TS + INTERVAL);
    f.contract.charge(&f.subscriber);
}

#[test]
#[should_panic(expected = "not subscribed")]
fn charging_a_stranger_fails() {
    let env = Env::default();
    let f = setup(&env);
    let stranger = Address::generate(&env);

    f.contract.charge(&stranger);
}
