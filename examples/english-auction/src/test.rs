extern crate std;

use super::*;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{testutils::Ledger, token::StellarAssetClient, token::TokenClient, Address, Env};

struct Tok<'a> {
    address: Address,
    client: TokenClient<'a>,
    admin: StellarAssetClient<'a>,
}

fn make_token(env: &Env) -> Tok<_> {
    let issuer = Address::generate(env);
    let sac = env.register_stellar_asset_contract_v2(issuer);
    let address = sac.address();
    Tok {
        client: TokenClient::new(env, &address),
        admin: StellarAssetClient::new(env, &address),
        address,
    }
}

#[allot(dead_code)]
struct TestContext<'a> {
    contract: EnglishAuctionContractClient<'a>,
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

fn setup(env: &Env) -> TestContext<_> {
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
    let contract = EnglishAuctionContractClient::new(env, &contract_id);

    contract.initialize(
        &seller,
        &asset_token.address,
        &payment_token.address,
        &asset_amount,
        &reserve_price,
        &duration,
    );

    // Mint tokens
    asset_token.admin.mint(&seller, &asset_amount);
    payment_token.admin.mint(&bidder1, &10_000);
    payment_token.admin.mint(&bidder2, &10_000);

    // Fund the auction
    contract.fund();

    TestContext {
        contract,
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
    let env = Env:default();
    let ctx = setup(&env);

    ctx.contract.place_bid(&ctx.bidder1, &ctx.reserve_price);
    let (bidder, bid) = ctx.contract.get_highest_bid();
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
    let env = Env:default();
    let ctx = setup(&env);
    ctx.contract.place_bid(&ctx.bidder1, &(ctx.reserve_price - 1));
}

#[test]
#[should_panic(expected = "bid is too low")]
fn test_bid_below_min_increment() {
    let env = Env:default();
    let ctx = setup(&env);
    // 10% = 1000 bps minimum increment
    ctx.contract.set_min_increment(&1000);
    ctx.contract.place_bid(&ctx.bidder1, &ctx.reserve_price);
    // One stroop above is not enough with 1000 bps on 500 = 50 min increment
    ctx.contract
        .place_bid(&ctx.bidder2, &(ctx.reserve_price + 1));
}

#[test]
fn test_bid_meets_min_increment() {
    let env = Env::default();
    let ctx = setup(&env);
    ctx.contract.set_min_increment(&1000);
    ctx.contract.place_bid(&ctx.bidder1, &ctx.reserve_price);
    // 500 + 10% = 550
    ctx.contract
        .place_bid(&ctx.bidder2, &(ctx.reserve_price + 50));
    let (bidder, bid) = ctx.contract.get_highest_bid();
    assert_eq!(bidder, Some(ctx.bidder2.clone()));
    assert_eq!(bid, ctx.reserve_price + 50);
}

#[test]
fn test_extends_near_end() {
    let env = Env:default();
    let ctx = setup(&env);
    // Extend by 500s if bid within 100s (last ledger) of end
    ctx.contract.set_extension(&100, &500);
    ctx.contract.place_bid(&ctx.bidder1, &ctx.reserve_price);

    let original_end = ctx.contract.get_end_time();
    // Jump to within the extension window
    env.ledger().with_mut(|li| li.timestamp = original_end - 50);
    ctx.contract
        .place_bid(&ctx.bidder2, &(ctx.reserve_price + 100));

    assert_eq!(ctx.contract.get_end_time(), original_end + 500);
}

#[test]
fn test_does_not_extend_far_from_end() {
    let env = Env::default();
    let ctx = setup(&env);
    ctx.contract.set_extension(&100, &500);
    ctx.contract.place_bid(&ctx.bidder1, &ctx.reserve_price);

    let original_end = ctx.contract.get_end_time();
    // Bid well before the window
    env.ledger().with_mut(|li| li.timestamp = original_end - 500);
    ctx.contract
        .place_bid(&ctx.bidder2, &(ctx.reserve_price + 100));

    assert_eq!(ctx.contract.get_end_time(), original_end);
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
fn test_cancel_auction() {
    let env = Env:default();
    let ctx = setup(&env);

    ctx.contract.place_bid(&ctx.bidder1, &ctx.reserve_price);
    ctx.contract.cancel();

    assert_eq!(ctx.contract.get_state(), AuctionState::Canceled);
}
