use super::*;
use soroban_sdk::testutils::{Address as _, Ledger};
use soroban_sdk::token::{StellarAssetClient, TokenClient};
use soroban_sdk::{Address, Env};

const START: u64 = 100;
const DEADLINE: u64 = 1_000;
const TARGET: i128 = 1_000;

struct Setup<'a> {
    campaign: CrowdfundContractClient<'a>,
    token: TokenClient<'a>,
    token_admin: StellarAssetClient<'a>,
    owner: Address,
}

fn setup(env: &Env) -> Setup<'_> {
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = START);

    let owner = Address::generate(env);
    let token_issuer = Address::generate(env);
    let sac = env.register_stellar_asset_contract_v2(token_issuer);
    let token_address = sac.address();

    let campaign_id = env.register(
        CrowdfundContract,
        (owner.clone(), token_address.clone(), TARGET, DEADLINE),
    );

    Setup {
        campaign: CrowdfundContractClient::new(env, &campaign_id),
        token: TokenClient::new(env, &token_address),
        token_admin: StellarAssetClient::new(env, &token_address),
        owner,
    }
}

fn advance_past_deadline(env: &Env) {
    env.ledger().with_mut(|li| li.timestamp = DEADLINE + 1);
}

#[test]
fn successful_campaign_owner_claims() {
    let env = Env::default();
    let s = setup(&env);
    let alice = Address::generate(&env);
    let bob = Address::generate(&env);
    s.token_admin.mint(&alice, &600);
    s.token_admin.mint(&bob, &500);

    s.campaign.pledge(&alice, &600);
    s.campaign.pledge(&bob, &500);
    assert_eq!(s.campaign.get_total_pledged(), 1_100);
    assert_eq!(s.campaign.get_pledge(&alice), 600);
    assert_eq!(s.token.balance(&alice), 0);

    advance_past_deadline(&env);
    s.campaign.claim();
    assert_eq!(s.token.balance(&s.owner), 1_100);
}

#[test]
fn failed_campaign_refunds_backers() {
    let env = Env::default();
    let s = setup(&env);
    let alice = Address::generate(&env);
    s.token_admin.mint(&alice, &300);

    s.campaign.pledge(&alice, &300);
    advance_past_deadline(&env);

    s.campaign.refund(&alice);
    assert_eq!(s.token.balance(&alice), 300);
    assert_eq!(s.campaign.get_pledge(&alice), 0);
    assert_eq!(s.campaign.get_total_pledged(), 0);
}

#[test]
#[should_panic]
fn pledge_after_deadline_panics() {
    let env = Env::default();
    let s = setup(&env);
    let alice = Address::generate(&env);
    s.token_admin.mint(&alice, &100);

    advance_past_deadline(&env);
    s.campaign.pledge(&alice, &100);
}

#[test]
#[should_panic]
fn claim_before_deadline_panics() {
    let env = Env::default();
    let s = setup(&env);
    let alice = Address::generate(&env);
    s.token_admin.mint(&alice, &TARGET);

    s.campaign.pledge(&alice, &TARGET);
    s.campaign.claim();
}

#[test]
#[should_panic]
fn refund_when_target_reached_panics() {
    let env = Env::default();
    let s = setup(&env);
    let alice = Address::generate(&env);
    s.token_admin.mint(&alice, &TARGET);

    s.campaign.pledge(&alice, &TARGET);
    advance_past_deadline(&env);
    s.campaign.refund(&alice);
}

#[test]
#[should_panic]
fn double_claim_panics() {
    let env = Env::default();
    let s = setup(&env);
    let alice = Address::generate(&env);
    s.token_admin.mint(&alice, &TARGET);

    s.campaign.pledge(&alice, &TARGET);
    advance_past_deadline(&env);
    s.campaign.claim();
    s.campaign.claim();
}
