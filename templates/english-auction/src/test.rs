extern crate std;

use super::*;
use soroban_sdk::testutils::storage::{Instance as _, Persistent as _};
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

#[allow(dead_code)]
struct TestContext<'a> {
    contract: EnglishAuctionContractClient<'a>,
    contract_address: Address,
    asset_token: Tok<'a>,
    payment_token: Tok<'a>,
    seller: Address,
    bidder1: Address,
    bidder2: Address,
    start_ts: u64,
    duration: u64,
    reserve_price: i128,
    asset_amount: i128,
}

fn setup(env: &Env) -> TestContext<'_> {
    env.mock_all_auths();

    let start_ts = 1_000_000_u64;
    env.ledger().with_mut(|li| li.timestamp = start_ts);

    let asset_token = make_token(env);
    let payment_token = make_token(env);
    let seller = Address::generate(env);
    let bidder1 = Address::generate(env);
    let bidder2 = Address::generate(env);

    let asset_amount = 100_i128;
    let reserve_price = 500_i128;
    let duration = 1_000_u64;

    let contract_id = env.register(EnglishAuctionContract, ());
    let contract_address = contract_id.clone();
    let contract = EnglishAuctionContractClient::new(env, &contract_id);

    contract.initialize(
        &seller,
        &asset_token.address,
        &payment_token.address,
        &asset_amount,
        &reserve_price,
        &duration,
        &0,
        &0,
    );

    // Mint tokens
    asset_token.admin.mint(&seller, &asset_amount);
    payment_token.admin.mint(&bidder1, &10_000);
    payment_token.admin.mint(&bidder2, &10_000);

    // Fund the auction
    contract.fund();

    TestContext {
        contract,
        contract_address,
        asset_token,
        payment_token,
        seller,
        bidder1,
        bidder2,
        start_ts,
        duration,
        reserve_price,
        asset_amount,
    }
}

#[test]
fn test_initialize() {
    let env = Env::default();
    let ctx = setup(&env);
    assert_eq!(ctx.contract.get_state(), AuctionState::Open);
}

#[test]
fn test_place_bid_at_reserve_price() {
    let env = Env::default();
    let ctx = setup(&env);

    ctx.contract.place_bid(&ctx.bidder1, &ctx.reserve_price);
    let (bidder, bid) = ctx.contract.get_highest_bid();
    assert_eq!(bidder, Some(ctx.bidder1.clone()));
    assert_eq!(bid, ctx.reserve_price);
}

#[test]
fn test_place_multiple_bids() {
    let env = Env::default();
    let ctx = setup(&env);

    ctx.contract.place_bid(&ctx.bidder1, &ctx.reserve_price);
    let (bidder, bid) = ctx.contract.get_highest_bid();
    assert_eq!(bidder, Some(ctx.bidder1.clone()));
    assert_eq!(bid, ctx.reserve_price);

    ctx.contract
        .place_bid(&ctx.bidder2, &(ctx.reserve_price + 100));
    let (bidder, bid) = ctx.contract.get_highest_bid();
    assert_eq!(bidder, Some(ctx.bidder2.clone()));
    assert_eq!(bid, ctx.reserve_price + 100);
}

#[test]
#[should_panic(expected = "bid is too low")]
fn test_bid_below_reserve() {
    let env = Env::default();
    let ctx = setup(&env);
    ctx.contract
        .place_bid(&ctx.bidder1, &(ctx.reserve_price - 1));
}

#[test]
fn test_settle_after_auction_ends() {
    let env = Env::default();
    let ctx = setup(&env);

    ctx.contract.place_bid(&ctx.bidder1, &ctx.reserve_price);
    let final_bid = ctx.reserve_price + 200;
    ctx.contract.place_bid(&ctx.bidder2, &final_bid);

    // Fast-forward past auction end
    env.ledger()
        .with_mut(|li| li.timestamp = ctx.start_ts + ctx.duration + 1);

    ctx.contract.settle();

    assert_eq!(ctx.contract.get_state(), AuctionState::Settled);
}

#[test]
fn test_cancel_auction_after_bid_fails() {
    let env = Env::default();
    let ctx = setup(&env);

    ctx.contract.place_bid(&ctx.bidder1, &ctx.reserve_price);
    let result = ctx.contract.try_cancel();

    assert!(result.is_err());
    assert_eq!(ctx.contract.get_state(), AuctionState::Open);
    assert_eq!(
        ctx.asset_token.client.balance(&ctx.contract_address),
        ctx.asset_amount
    );
    assert_eq!(ctx.asset_token.client.balance(&ctx.seller), 0);
    assert_eq!(
        ctx.payment_token.client.balance(&ctx.contract_address),
        ctx.reserve_price
    );
    assert_eq!(
        ctx.payment_token.client.balance(&ctx.bidder1),
        10_000 - ctx.reserve_price
    );
}

#[test]
fn test_cancel_ended_auction_with_bid_fails_before_settlement() {
    let env = Env::default();
    let ctx = setup(&env);

    ctx.contract.place_bid(&ctx.bidder1, &ctx.reserve_price);
    env.ledger()
        .with_mut(|li| li.timestamp = ctx.start_ts + ctx.duration + 1);

    let result = ctx.contract.try_cancel();

    assert!(result.is_err());
    assert_eq!(ctx.contract.get_state(), AuctionState::Open);
    assert_eq!(
        ctx.asset_token.client.balance(&ctx.contract_address),
        ctx.asset_amount
    );
    assert_eq!(ctx.asset_token.client.balance(&ctx.seller), 0);
    assert_eq!(
        ctx.payment_token.client.balance(&ctx.contract_address),
        ctx.reserve_price
    );
    assert_eq!(
        ctx.payment_token.client.balance(&ctx.bidder1),
        10_000 - ctx.reserve_price
    );
}

#[test]
fn test_cancel_ended_auction_without_bids() {
    let env = Env::default();
    let ctx = setup(&env);

    env.ledger()
        .with_mut(|li| li.timestamp = ctx.start_ts + ctx.duration + 1);
    ctx.contract.cancel();

    assert_eq!(ctx.contract.get_state(), AuctionState::Canceled);
    assert_eq!(
        ctx.asset_token.client.balance(&ctx.seller),
        ctx.asset_amount
    );
    assert_eq!(ctx.asset_token.client.balance(&ctx.contract_address), 0);
}

#[test]
fn test_refund_fails_does_not_block_new_bid() {
    let env = Env::default();
    env.mock_all_auths();

    let start_ts = 1_000_000_u64;
    env.ledger().with_mut(|li| li.timestamp = start_ts);

    let asset_token = make_token(&env);
    let payment_token = make_token(&env);
    let seller = Address::generate(&env);
    let bidder1 = Address::generate(&env);
    let bidder2 = Address::generate(&env);
    let bidder3 = Address::generate(&env);

    let asset_amount = 100_i128;
    let reserve_price = 500_i128;
    let duration = 1_000_u64;

    let contract_id = env.register(EnglishAuctionContract, ());
    let contract = EnglishAuctionContractClient::new(&env, &contract_id);

    contract.initialize(
        &seller,
        &asset_token.address,
        &payment_token.address,
        &asset_amount,
        &reserve_price,
        &duration,
        &0,
        &0,
    );

    // Mint tokens
    asset_token.admin.mint(&seller, &asset_amount);
    payment_token.admin.mint(&bidder1, &10_000);
    payment_token.admin.mint(&bidder2, &10_000);
    payment_token.admin.mint(&bidder3, &10_000);

    // Fund the auction
    contract.fund();

    // First bid
    contract.place_bid(&bidder1, &reserve_price);

    // Make bidder1 have no trustline or be unable to receive refund by
    // creating a scenario where the refund transfer would fail - but we
    // can't easily make transfer fail in mock_all_auths; instead, we
    // test that the pull refund pattern is implemented correctly
    // by verifying bidder2 can bid after bidder1
    contract.place_bid(&bidder2, &(reserve_price + 100));

    let (bidder, bid) = contract.get_highest_bid();
    assert_eq!(bidder, Some(bidder2.clone()));
    assert_eq!(bid, reserve_price + 100);

    // Bidder1 should have a refund available
    contract.withdraw_refund(&bidder1);
}

#[test]
fn test_late_funding_keeps_the_full_bidding_window() {
    // #577: the bidding clock starts in `fund`, so a seller who funds long
    // after `initialize` still gets the whole `duration_seconds`.
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = 1_000);

    let asset_token = make_token(&env);
    let payment_token = make_token(&env);
    let seller = Address::generate(&env);
    let bidder = Address::generate(&env);
    let contract_id = env.register(EnglishAuctionContract, ());
    let contract = EnglishAuctionContractClient::new(&env, &contract_id);

    contract.initialize(
        &seller,
        &asset_token.address,
        &payment_token.address,
        &100,
        &500,
        &1_000,
        &0,
        &0,
    );
    asset_token.admin.mint(&seller, &100);
    payment_token.admin.mint(&bidder, &10_000);

    // Fund well after what would have been the end under the old behaviour.
    env.ledger().with_mut(|li| li.timestamp = 5_000);
    contract.fund();

    // Near the end of the window that started at funding: still open.
    env.ledger().with_mut(|li| li.timestamp = 5_900);
    contract.place_bid(&bidder, &500);
    assert_eq!(contract.get_highest_bid(), (Some(bidder), 500));
}

#[test]
fn test_contract_survives_past_default_ttl() {
    let env = Env::default();
    env.mock_all_auths();

    let start_ts = 1_000_000_u64;
    env.ledger().with_mut(|li| li.timestamp = start_ts);

    let asset_token = make_token(&env);
    let payment_token = make_token(&env);
    let seller = Address::generate(&env);
    let bidder1 = Address::generate(&env);
    let bidder2 = Address::generate(&env);
    let bidder3 = Address::generate(&env);

    let asset_amount = 100_i128;
    let reserve_price = 500_i128;
    // Long enough for the auction to stay open past the default ledger TTL.
    let duration = 7 * 86_400_u64;

    let contract_id = env.register(EnglishAuctionContract, ());
    let contract = EnglishAuctionContractClient::new(&env, &contract_id);

    contract.initialize(
        &seller,
        &asset_token.address,
        &payment_token.address,
        &asset_amount,
        &reserve_price,
        &duration,
        &500,
        &0,
    );

    asset_token.admin.mint(&seller, &asset_amount);
    payment_token.admin.mint(&bidder1, &10_000);
    payment_token.admin.mint(&bidder2, &10_000);
    payment_token.admin.mint(&bidder3, &10_000);

    contract.fund();
    contract.place_bid(&bidder1, &500);
    contract.place_bid(&bidder2, &600);

    // Advance well past the default persistent/instance TTL.
    let default_ttl = env.ledger().get().min_persistent_entry_ttl;
    let advance = default_ttl + 100;
    env.ledger().with_mut(|li| {
        li.sequence_number += advance;
        li.timestamp += u64::from(advance) * 5;
    });

    // Verify neither instance storage nor bidder1's persistent refund entry expired
    // (get_ttl panics if an entry has expired, preventing auto-restore from masking bugs).
    env.as_contract(&contract_id, || {
        let instance_ttl = env.storage().instance().get_ttl();
        assert!(
            instance_ttl >= INSTANCE_TTL_EXTEND_TO - advance,
            "instance storage TTL should remain extended past default TTL, got {instance_ttl}"
        );
        let refund_ttl = env
            .storage()
            .persistent()
            .get_ttl(&DataKey::Refund(bidder1.clone()));
        assert!(
            refund_ttl >= PERSISTENT_TTL_EXTEND_TO - advance,
            "persistent refund TTL should remain extended past default TTL, got {refund_ttl}"
        );
    });

    // Contract operations continue to work seamlessly past the default TTL.
    assert_eq!(contract.get_state(), AuctionState::Open);
    assert_eq!(contract.get_highest_bid(), (Some(bidder2.clone()), 600));
    assert_eq!(contract.get_refund(&bidder1), 500);

    contract.place_bid(&bidder3, &700);
    assert_eq!(contract.get_highest_bid(), (Some(bidder3.clone()), 700));
    assert_eq!(contract.get_refund(&bidder2), 600);

    contract.withdraw_refund(&bidder1);
    assert_eq!(payment_token.client.balance(&bidder1), 10_000);
    assert_eq!(contract.get_refund(&bidder1), 0);

    // Advance past auction end and past default TTL again, then settle and withdraw remaining refund.
    env.ledger().with_mut(|li| {
        li.sequence_number += advance;
        li.timestamp = start_ts + duration + 1;
    });

    contract.settle();
    assert_eq!(contract.get_state(), AuctionState::Settled);
    assert_eq!(asset_token.client.balance(&bidder3), asset_amount);
    assert_eq!(payment_token.client.balance(&seller), 700);

    contract.withdraw_refund(&bidder2);
    assert_eq!(payment_token.client.balance(&bidder2), 10_000);
}

#[test]
fn test_ttl_extended_on_state_changes_and_persistent_reads() {
    let env = Env::default();
    env.mock_all_auths();

    let start_ts = 1_000_000_u64;
    env.ledger().with_mut(|li| li.timestamp = start_ts);

    let asset_token = make_token(&env);
    let payment_token = make_token(&env);
    let seller = Address::generate(&env);
    let bidder1 = Address::generate(&env);
    let bidder2 = Address::generate(&env);

    let asset_amount = 100_i128;
    let reserve_price = 500_i128;
    let duration = 30 * 86_400_u64;

    let contract_id = env.register(EnglishAuctionContract, ());
    let contract = EnglishAuctionContractClient::new(&env, &contract_id);

    contract.initialize(
        &seller,
        &asset_token.address,
        &payment_token.address,
        &asset_amount,
        &reserve_price,
        &duration,
        &0,
        &0,
    );
    env.as_contract(&contract_id, || {
        assert_eq!(env.storage().instance().get_ttl(), INSTANCE_TTL_EXTEND_TO);
    });

    asset_token.admin.mint(&seller, &asset_amount);
    payment_token.admin.mint(&bidder1, &10_000);
    payment_token.admin.mint(&bidder2, &10_000);

    // Advance enough ledgers for remaining TTL to drop below threshold.
    let drop_below_threshold = DAY_IN_LEDGERS + 10;

    env.ledger()
        .with_mut(|li| li.sequence_number += drop_below_threshold);
    contract.fund();
    env.as_contract(&contract_id, || {
        assert_eq!(env.storage().instance().get_ttl(), INSTANCE_TTL_EXTEND_TO);
    });

    env.ledger()
        .with_mut(|li| li.sequence_number += drop_below_threshold);
    contract.place_bid(&bidder1, &500);
    env.as_contract(&contract_id, || {
        assert_eq!(env.storage().instance().get_ttl(), INSTANCE_TTL_EXTEND_TO);
    });

    env.ledger()
        .with_mut(|li| li.sequence_number += drop_below_threshold);
    contract.place_bid(&bidder2, &600);
    env.as_contract(&contract_id, || {
        assert_eq!(env.storage().instance().get_ttl(), INSTANCE_TTL_EXTEND_TO);
        assert_eq!(
            env.storage()
                .persistent()
                .get_ttl(&DataKey::Refund(bidder1.clone())),
            PERSISTENT_TTL_EXTEND_TO
        );
    });

    // Hot-path read of persistent refund entry bumps its TTL back to PERSISTENT_TTL_EXTEND_TO.
    env.ledger()
        .with_mut(|li| li.sequence_number += drop_below_threshold);
    assert_eq!(contract.get_refund(&bidder1), 500);
    env.as_contract(&contract_id, || {
        assert_eq!(
            env.storage()
                .persistent()
                .get_ttl(&DataKey::Refund(bidder1.clone())),
            PERSISTENT_TTL_EXTEND_TO
        );
    });

    // withdraw_refund bumps instance TTL.
    env.ledger()
        .with_mut(|li| li.sequence_number += drop_below_threshold);
    contract.withdraw_refund(&bidder1);
    env.as_contract(&contract_id, || {
        assert_eq!(env.storage().instance().get_ttl(), INSTANCE_TTL_EXTEND_TO);
    });

    // settle bumps instance TTL.
    env.ledger().with_mut(|li| {
        li.sequence_number += drop_below_threshold;
        li.timestamp = start_ts + duration + 1;
    });
    contract.settle();
    env.as_contract(&contract_id, || {
        assert_eq!(env.storage().instance().get_ttl(), INSTANCE_TTL_EXTEND_TO);
    });

    // cancel also bumps instance TTL on an auction with no bids.
    let cancel_contract_id = env.register(EnglishAuctionContract, ());
    let cancel_contract = EnglishAuctionContractClient::new(&env, &cancel_contract_id);
    asset_token.admin.mint(&seller, &asset_amount);
    cancel_contract.initialize(
        &seller,
        &asset_token.address,
        &payment_token.address,
        &asset_amount,
        &reserve_price,
        &duration,
        &0,
        &0,
    );
    cancel_contract.fund();
    env.ledger()
        .with_mut(|li| li.sequence_number += drop_below_threshold);
    cancel_contract.cancel();
    env.as_contract(&cancel_contract_id, || {
        assert_eq!(env.storage().instance().get_ttl(), INSTANCE_TTL_EXTEND_TO);
    });
}
