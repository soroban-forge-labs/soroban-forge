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

fn advance_ledgers(env: &Env, count: u32) {
    env.ledger().with_mut(|ledger| {
        ledger.sequence_number += count;
    });
}

fn instance_ttl(env: &Env, campaign: &Address) -> u32 {
    use soroban_sdk::testutils::storage::Instance as _;
    env.as_contract(campaign, || env.storage().instance().get_ttl())
}

fn pledge_ttl(env: &Env, campaign: &Address, backer: &Address) -> u32 {
    use soroban_sdk::testutils::storage::Persistent as _;
    env.as_contract(campaign, || {
        env.storage()
            .persistent()
            .get_ttl(&DataKey::Pledge(backer.clone()))
    })
}

#[test]
fn instance_and_pledges_survive_past_default_ttl() {
    let env = Env::default();
    let s = setup(&env);
    let alice = Address::generate(&env);
    s.token_admin.mint(&alice, &TARGET);
    let default_ttl = env.ledger().get().min_persistent_entry_ttl;
    // Inspect BEFORE advancing: recording-mode auto-restore cannot turn an
    // expired entry into a false passing lifetime assertion.
    assert!(instance_ttl(&env, &s.campaign.address) > default_ttl);
    s.campaign.pledge(&alice, &TARGET);
    assert!(pledge_ttl(&env, &s.campaign.address, &alice) > default_ttl);

    advance_ledgers(&env, default_ttl + 1);
    assert!(instance_ttl(&env, &s.campaign.address) > 0);
    assert!(pledge_ttl(&env, &s.campaign.address, &alice) > 0);
    assert_eq!(s.campaign.get_total_pledged(), TARGET);
    assert_eq!(s.campaign.get_pledge(&alice), TARGET);
    advance_past_deadline(&env);
    s.campaign.claim();
    assert_eq!(s.token.balance(&s.owner), TARGET);
}

#[test]
fn every_state_change_renews_the_instance() {
    let env = Env::default();
    let s = setup(&env);
    let alice = Address::generate(&env);
    s.token_admin.mint(&alice, &TARGET);
    assert_eq!(instance_ttl(&env, &s.campaign.address), TTL_EXTEND_TO);
    advance_ledgers(&env, 2 * DAY_IN_LEDGERS);
    s.campaign.pledge(&alice, &TARGET);
    assert_eq!(instance_ttl(&env, &s.campaign.address), TTL_EXTEND_TO);
    advance_ledgers(&env, 2 * DAY_IN_LEDGERS);
    advance_past_deadline(&env);
    s.campaign.claim();
    assert_eq!(instance_ttl(&env, &s.campaign.address), TTL_EXTEND_TO);
    assert_eq!(s.token.balance(&s.owner), TARGET);

    let refund_env = Env::default();
    let r = setup(&refund_env);
    let bob = Address::generate(&refund_env);
    r.token_admin.mint(&bob, &300);
    r.campaign.pledge(&bob, &300);
    advance_ledgers(&refund_env, 2 * DAY_IN_LEDGERS);
    advance_past_deadline(&refund_env);
    refund_env.mock_auths(&[]);
    r.campaign.refund(&bob);
    assert_eq!(
        instance_ttl(&refund_env, &r.campaign.address),
        TTL_EXTEND_TO
    );
    assert_eq!(r.token.balance(&bob), 300);
    assert_eq!(r.campaign.get_pledge(&bob), 0);
    refund_env.as_contract(&r.campaign.address, || {
        assert!(!refund_env
            .storage()
            .persistent()
            .has(&DataKey::Pledge(bob.clone())));
    });
}

#[test]
fn pledge_writes_and_hot_reads_renew_existing_entries() {
    let env = Env::default();
    let s = setup(&env);
    let alice = Address::generate(&env);
    let bob = Address::generate(&env);
    s.token_admin.mint(&alice, &300);
    s.token_admin.mint(&bob, &200);
    s.campaign.pledge(&alice, &100);
    s.campaign.pledge(&bob, &200);
    assert_eq!(pledge_ttl(&env, &s.campaign.address, &alice), TTL_EXTEND_TO);
    advance_ledgers(&env, 2 * DAY_IN_LEDGERS);
    s.campaign.pledge(&alice, &200);
    assert_eq!(pledge_ttl(&env, &s.campaign.address, &alice), TTL_EXTEND_TO);
    // Alice's write must not reset another backer's amount or lifetime.
    assert_eq!(
        pledge_ttl(&env, &s.campaign.address, &bob),
        TTL_EXTEND_TO - 2 * DAY_IN_LEDGERS
    );
    assert_eq!(s.campaign.get_pledge(&bob), 200);
    assert_eq!(pledge_ttl(&env, &s.campaign.address, &bob), TTL_EXTEND_TO);
    advance_ledgers(&env, 2 * DAY_IN_LEDGERS);
    assert_eq!(s.campaign.get_pledge(&alice), 300);
    assert_eq!(pledge_ttl(&env, &s.campaign.address, &alice), TTL_EXTEND_TO);
    assert_eq!(s.campaign.get_total_pledged(), 500);
}

#[test]
fn missing_pledge_read_does_not_create_or_extend_an_entry() {
    let env = Env::default();
    let s = setup(&env);
    let stranger = Address::generate(&env);
    env.mock_auths(&[]);
    assert_eq!(s.campaign.get_pledge(&stranger), 0);
    env.as_contract(&s.campaign.address, || {
        assert!(!env.storage().persistent().has(&DataKey::Pledge(stranger)));
    });
}

#[test]
fn pledge_read_renews_at_threshold_but_not_above_it() {
    let env = Env::default();
    let s = setup(&env);
    let alice = Address::generate(&env);
    s.token_admin.mint(&alice, &100);
    s.campaign.pledge(&alice, &100);
    advance_ledgers(&env, DAY_IN_LEDGERS - 1);
    assert_eq!(s.campaign.get_pledge(&alice), 100);
    assert_eq!(
        pledge_ttl(&env, &s.campaign.address, &alice),
        TTL_THRESHOLD + 1
    );
    advance_ledgers(&env, 1);
    assert_eq!(s.campaign.get_pledge(&alice), 100);
    assert_eq!(pledge_ttl(&env, &s.campaign.address, &alice), TTL_EXTEND_TO);
}

#[test]
fn refund_works_after_default_ttl_without_backer_authorization() {
    let env = Env::default();
    let s = setup(&env);
    let alice = Address::generate(&env);
    let bob = Address::generate(&env);
    s.token_admin.mint(&alice, &300);
    s.token_admin.mint(&bob, &200);
    s.campaign.pledge(&alice, &300);
    s.campaign.pledge(&bob, &200);
    let advance = env.ledger().get().min_persistent_entry_ttl + 1;
    assert!(pledge_ttl(&env, &s.campaign.address, &alice) > advance);
    assert!(pledge_ttl(&env, &s.campaign.address, &bob) > advance);
    advance_ledgers(&env, advance);
    advance_past_deadline(&env);
    env.mock_auths(&[]);
    s.campaign.refund(&alice);
    s.campaign.refund(&bob);
    assert_eq!(s.token.balance(&alice), 300);
    assert_eq!(s.token.balance(&bob), 200);
    assert_eq!(s.campaign.get_total_pledged(), 0);
    assert!(s.campaign.try_refund(&alice).is_err());
    assert_eq!(s.token.balance(&alice), 300);
}
