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

#[allow(dead_code)]
struct TestContext<'a> {
    contract: DutchAuctionContractClient<'a>,
    asset_token: Tok<'a>,
    payment_token: Tok<'a>,
    seller: Address,
    buyer: Address,
    start_ts: u64,
    duration: u64,
    start_price: i128,
    floor_price: i128,
    asset_amount: i128,
}

fn setup(env: &Env) -> TestContext<'_> {
    env.mock_all_auths();

    let start_ts = 1_000_000_u64;
    env.ledger().with_mut(|li| li.timestamp = start_ts);

    let asset_token = make_token(env);
    let payment_token = make_token(env);
    let seller = Address::generate(env);
    let buyer = Address::generate(env);

    let asset_amount = 100_i128;
    let start_price = 1_000_i128;
    let floor_price = 200_i128;
    let duration = 1_000_u64;

    let contract_id = env.register(DutchAuctionContract, ());
    let contract = DutchAuctionContractClient::new(env, &contract_id);

    contract.initialize(
        &seller,
        &asset_token.address,
        &payment_token.address,
        &asset_amount,
        &start_price,
        &floor_price,
        &duration,
    );

    // Mint tokens to seller and buyer
    asset_token.admin.mint(&seller, &asset_amount);
    payment_token.admin.mint(&buyer, &10_000);

    // Fund the auction
    contract.fund();

    TestContext {
        contract,
        asset_token,
        payment_token,
        seller,
        buyer,
        start_ts,
        duration,
        start_price,
        floor_price,
        asset_amount,
    }
}

#[test]
fn test_price_curve_at_several_timestamps() {
    let env = Env::default();
    let ctx = setup(&env);

    // At auction start (t = 0s elapsed): price == start_price (1000)
    env.ledger().with_mut(|li| li.timestamp = ctx.start_ts);
    assert_eq!(ctx.contract.get_current_price(), 1000);

    // Quarter-way through (t = 250s elapsed): 1000 - (800 * 250 / 1000) = 800
    env.ledger()
        .with_mut(|li| li.timestamp = ctx.start_ts + 250);
    assert_eq!(ctx.contract.get_current_price(), 800);

    // Halfway through (t = 500s elapsed): 1000 - (800 * 500 / 1000) = 600
    env.ledger()
        .with_mut(|li| li.timestamp = ctx.start_ts + 500);
    assert_eq!(ctx.contract.get_current_price(), 600);

    // Three-quarters through (t = 750s elapsed): 1000 - (800 * 750 / 1000) = 400
    env.ledger()
        .with_mut(|li| li.timestamp = ctx.start_ts + 750);
    assert_eq!(ctx.contract.get_current_price(), 400);

    // At the end of the duration window (t = 1000s elapsed): price == floor_price (200)
    env.ledger()
        .with_mut(|li| li.timestamp = ctx.start_ts + ctx.duration);
    assert_eq!(ctx.contract.get_current_price(), 200);

    // Past the duration window (t = 2000s elapsed): price stays at floor_price (200)
    env.ledger()
        .with_mut(|li| li.timestamp = ctx.start_ts + 2000);
    assert_eq!(ctx.contract.get_current_price(), 200);
}

#[test]
fn test_purchase_at_floor_price() {
    let env = Env::default();
    let ctx = setup(&env);

    // Advance timestamp past the auction duration to reach the floor price
    env.ledger()
        .with_mut(|li| li.timestamp = ctx.start_ts + ctx.duration + 500);
    assert_eq!(ctx.contract.get_current_price(), ctx.floor_price);

    let price_paid = ctx.contract.buy(&ctx.buyer);
    assert_eq!(price_paid, ctx.floor_price);

    // Seller received floor price in payment tokens
    assert_eq!(
        ctx.payment_token.client.balance(&ctx.seller),
        ctx.floor_price
    );
    // Buyer received the auctioned asset tokens
    assert_eq!(
        ctx.asset_token.client.balance(&ctx.buyer),
        ctx.asset_amount
    );
    // Contract has no remaining asset tokens
    assert_eq!(
        ctx.asset_token.client.balance(&ctx.contract.address),
        0
    );

    // State is Settled
    assert_eq!(ctx.contract.get_state(), AuctionState::Settled);
}

#[test]
fn test_purchase_mid_auction() {
    let env = Env::default();
    let ctx = setup(&env);

    // Halfway through: price is 600
    env.ledger()
        .with_mut(|li| li.timestamp = ctx.start_ts + 500);
    assert_eq!(ctx.contract.get_current_price(), 600);

    let price_paid = ctx.contract.buy(&ctx.buyer);
    assert_eq!(price_paid, 600);

    assert_eq!(ctx.payment_token.client.balance(&ctx.seller), 600);
    assert_eq!(
        ctx.asset_token.client.balance(&ctx.buyer),
        ctx.asset_amount
    );
    assert_eq!(ctx.contract.get_state(), AuctionState::Settled);
}

#[test]
#[should_panic(expected = "auction is not open for purchase")]
fn test_cannot_buy_twice() {
    let env = Env::default();
    let ctx = setup(&env);

    ctx.contract.buy(&ctx.buyer);
    // Second purchase attempt should panic
    ctx.contract.buy(&ctx.buyer);
}

#[test]
fn test_cancel_auction() {
    let env = Env::default();
    let ctx = setup(&env);

    assert_eq!(ctx.contract.get_state(), AuctionState::Open);
    ctx.contract.cancel();
    assert_eq!(ctx.contract.get_state(), AuctionState::Canceled);

    // Asset tokens refunded to seller
    assert_eq!(
        ctx.asset_token.client.balance(&ctx.seller),
        ctx.asset_amount
    );
}

#[test]
#[should_panic(expected = "can only cancel an open auction")]
fn test_cannot_cancel_after_settled() {
    let env = Env::default();
    let ctx = setup(&env);

    ctx.contract.buy(&ctx.buyer);
    ctx.contract.cancel();
}
