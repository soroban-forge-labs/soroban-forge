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

fn setup(env: &Env) -> (VestingContractClient<'_>, Tok<'_>, Address, Address, u64) {
    env.mock_all_auths();

    // Pin a known ledger timestamp for deterministic tests.
    let start_ts = 1_000_000;
    env.ledger().with_mut(|li| li.timestamp = start_ts);

    let token = make_token(env);
    let admin = Address::generate(env);
    let beneficiary = Address::generate(env);
    let cliff_duration = 1000_u64; // 1000 seconds
    let vesting_duration = 4000_u64; // 4000 seconds

    let contract_id = env.register(VestingContract, ());
    let contract = VestingContractClient::new(env, &contract_id);

    contract.initialize(
        &admin,
        &token.address,
        &beneficiary,
        &cliff_duration,
        &vesting_duration,
        &100_000,
    );

    (contract, token, admin, beneficiary, start_ts)
}

#[test]
fn pre_cliff_nothing_claimable() {
    let env = Env::default();
    let (contract, token, admin, _beneficiary, start_ts) = setup(&env);

    // Fund the contract.
    token.admin.mint(&admin, &100_000);
    contract.fund();

    // Jump to just before the cliff — nothing should be vested.
    env.ledger().with_mut(|li| li.timestamp = start_ts + 999);
    assert_eq!(contract.get_vested_amount(), 0);
    assert_eq!(contract.get_claimable_amount(), 0);

    // The claim should revert.
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(std::boxed::Box::new(|_| {}));
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        contract.claim();
    }));
    std::panic::set_hook(prev_hook);
    assert!(result.is_err());
}

#[test]
fn mid_schedule_partial_claim() {
    let env = Env::default();
    let (contract, token, admin, beneficiary, start_ts) = setup(&env);

    token.admin.mint(&admin, &100_000);
    contract.fund();

    // Jump to halfway through the vesting period.
    // Cliff ends at start_ts + 1000, duration is 4000, so end = start_ts + 5000.
    // Halfway through vesting = start_ts + 1000 + 2000 = start_ts + 3000.
    env.ledger().with_mut(|li| li.timestamp = start_ts + 3000);
    let vested = contract.get_vested_amount();
    let claimable = contract.get_claimable_amount();

    // Should be exactly 50% vested (2000 / 4000 * 100_000 = 50_000).
    assert_eq!(vested, 50_000);
    assert_eq!(claimable, 50_000);

    // Claim and verify transfer.
    let claimed = contract.claim();
    assert_eq!(claimed, 50_000);

    let balance = token.client.balance(&beneficiary);
    assert_eq!(balance, 50_000);

    // After claiming, claimable should be 0.
    assert_eq!(contract.get_claimable_amount(), 0);
}

#[test]
fn full_release_after_end() {
    let env = Env::default();
    let (contract, token, admin, beneficiary, start_ts) = setup(&env);

    token.admin.mint(&admin, &100_000);
    contract.fund();

    // Jump past the full vesting end.
    // End = start_ts + 1000 + 4000 = start_ts + 5000.
    env.ledger().with_mut(|li| li.timestamp = start_ts + 6000);
    assert_eq!(contract.get_vested_amount(), 100_000);
    assert_eq!(contract.get_claimable_amount(), 100_000);

    let claimed = contract.claim();
    assert_eq!(claimed, 100_000);
    assert_eq!(token.client.balance(&beneficiary), 100_000);
}

#[test]
fn multiple_partial_claims() {
    let env = Env::default();
    let (contract, token, admin, beneficiary, start_ts) = setup(&env);

    token.admin.mint(&admin, &100_000);
    contract.fund();

    // Claim at 25% vesting.
    env.ledger().with_mut(|li| li.timestamp = start_ts + 2000);
    // 1000 / 4000 * 100_000 = 25_000
    assert_eq!(contract.claim(), 25_000);
    assert_eq!(token.client.balance(&beneficiary), 25_000);

    // Claim at 75% vesting (cumulative).
    env.ledger().with_mut(|li| li.timestamp = start_ts + 4000);
    // 3000 / 4000 * 100_000 = 75_000, minus 25_000 claimed = 50_000
    assert_eq!(contract.claim(), 50_000);
    assert_eq!(token.client.balance(&beneficiary), 75_000);

    // Claim final 25%.
    env.ledger().with_mut(|li| li.timestamp = start_ts + 6000);
    assert_eq!(contract.claim(), 25_000);
    assert_eq!(token.client.balance(&beneficiary), 100_000);
}

#[test]
#[should_panic(expected = "nothing to claim")]
fn double_claim_fails() {
    let env = Env::default();
    let (contract, token, admin, _beneficiary, start_ts) = setup(&env);

    token.admin.mint(&admin, &100_000);
    contract.fund();

    // Claim everything after full release.
    env.ledger().with_mut(|li| li.timestamp = start_ts + 6000);
    contract.claim();

    // Second claim should fail.
    contract.claim();
}

#[test]
#[should_panic(expected = "nothing to claim")]
fn claim_pre_cliff_fails() {
    let env = Env::default();
    let (contract, _token, _admin, _beneficiary, start_ts) = setup(&env);

    // Stay pre-cliff — nothing is claimable.
    env.ledger().with_mut(|li| li.timestamp = start_ts + 500);
    contract.claim();
}
