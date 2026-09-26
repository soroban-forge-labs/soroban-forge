use super::*;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{token::StellarAssetClient, token::TokenClient, Address, Env};

/// A registered Stellar Asset Contract plus a client to mint with.
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

/// Register a pool over two fresh tokens, returning the pool client, the two
/// tokens (ordered as passed to the constructor), and a funded provider.
fn setup(env: &Env) -> (LiquidityPoolClient<'_>, Tok<'_>, Tok<'_>, Address) {
    env.mock_all_auths();
    let token_a = make_token(env);
    let token_b = make_token(env);
    let pool_id = env.register(
        LiquidityPool,
        (token_a.address.clone(), token_b.address.clone()),
    );
    let pool = LiquidityPoolClient::new(env, &pool_id);
    let provider = Address::generate(env);
    (pool, token_a, token_b, provider)
}

#[test]
fn deposit_adds_liquidity_and_mints_shares() {
    let env = Env::default();
    let (pool, token_a, token_b, alice) = setup(&env);
    token_a.admin.mint(&alice, &1_000);
    token_b.admin.mint(&alice, &4_000);

    // First deposit sets the price: 1000 A + 4000 B.
    let minted = pool.deposit(&alice, &1_000, &1_000, &4_000, &4_000);
    assert!(minted > 0);
    assert_eq!(pool.get_reserves(), (1_000, 4_000));
    assert_eq!(pool.shares(&alice), minted);
    assert_eq!(pool.total_shares(), minted);
    // Tokens actually moved into the pool.
    assert_eq!(token_a.client.balance(&alice), 0);
    assert_eq!(token_b.client.balance(&alice), 0);

    // A second provider deposits at the same 1:4 ratio.
    let bob = Address::generate(&env);
    token_a.admin.mint(&bob, &500);
    token_b.admin.mint(&bob, &2_000);
    let bob_shares = pool.deposit(&bob, &500, &500, &2_000, &2_000);
    assert!(bob_shares > 0);
    assert_eq!(pool.get_reserves(), (1_500, 6_000));
}

#[test]
fn swap_preserves_constant_product_invariant() {
    let env = Env::default();
    let (pool, token_a, token_b, alice) = setup(&env);
    token_a.admin.mint(&alice, &1_000);
    token_b.admin.mint(&alice, &1_000);
    pool.deposit(&alice, &1_000, &1_000, &1_000, &1_000);

    let (ra0, rb0) = pool.get_reserves();
    let k_before = ra0 * rb0;

    // Trader buys 100 of token B with token A (buy_a = false).
    let trader = Address::generate(&env);
    token_a.admin.mint(&trader, &1_000);
    let spent = pool.swap(&trader, &false, &100, &1_000);

    let (ra1, rb1) = pool.get_reserves();
    let k_after = ra1 * rb1;

    // The core acceptance check: x*y=k holds and grows by the fee.
    assert!(k_after >= k_before, "invariant violated: {k_after} < {k_before}");
    // Trader received exactly the requested output and paid the input in.
    assert_eq!(token_b.client.balance(&trader), 100);
    assert_eq!(rb1, rb0 - 100);
    assert_eq!(ra1, ra0 + spent);
    assert_eq!(token_a.client.balance(&trader), 1_000 - spent);
}

#[test]
fn withdraw_removes_liquidity_proportionally() {
    let env = Env::default();
    let (pool, token_a, token_b, alice) = setup(&env);
    token_a.admin.mint(&alice, &1_000);
    token_b.admin.mint(&alice, &1_000);
    let minted = pool.deposit(&alice, &1_000, &1_000, &1_000, &1_000);

    let (out_a, out_b) = pool.withdraw(&alice, &minted, &1, &1);
    assert_eq!(out_a, 1_000);
    assert_eq!(out_b, 1_000);
    assert_eq!(pool.total_shares(), 0);
    assert_eq!(pool.shares(&alice), 0);
    assert_eq!(pool.get_reserves(), (0, 0));
    // Alice got her tokens back.
    assert_eq!(token_a.client.balance(&alice), 1_000);
    assert_eq!(token_b.client.balance(&alice), 1_000);
}

#[test]
fn swap_then_withdraw_returns_fees_to_provider() {
    let env = Env::default();
    let (pool, token_a, token_b, alice) = setup(&env);
    token_a.admin.mint(&alice, &1_000);
    token_b.admin.mint(&alice, &1_000);
    let minted = pool.deposit(&alice, &1_000, &1_000, &1_000, &1_000);

    let trader = Address::generate(&env);
    token_a.admin.mint(&trader, &1_000);
    pool.swap(&trader, &false, &100, &1_000);

    // As the sole provider, Alice withdraws everything; her token A balance
    // now exceeds the original 1000 because the swap fed fees into reserve A.
    let (out_a, out_b) = pool.withdraw(&alice, &minted, &1, &1);
    assert!(out_a > 1_000, "provider should reclaim more A than deposited");
    assert!(out_b < 1_000, "B reserve was drawn down by the swap");
}

#[test]
#[should_panic]
fn swap_exceeding_in_max_panics() {
    let env = Env::default();
    let (pool, token_a, token_b, alice) = setup(&env);
    token_a.admin.mint(&alice, &1_000);
    token_b.admin.mint(&alice, &1_000);
    pool.deposit(&alice, &1_000, &1_000, &1_000, &1_000);

    let trader = Address::generate(&env);
    token_a.admin.mint(&trader, &1_000);
    // Buying 100 B costs ~112 A; cap the input at 1 to force a slippage panic.
    pool.swap(&trader, &false, &100, &1);
}

#[test]
#[should_panic]
fn withdraw_below_min_panics() {
    let env = Env::default();
    let (pool, token_a, token_b, alice) = setup(&env);
    token_a.admin.mint(&alice, &1_000);
    token_b.admin.mint(&alice, &1_000);
    let minted = pool.deposit(&alice, &1_000, &1_000, &1_000, &1_000);

    // Demand far more token A out than the pool can return.
    pool.withdraw(&alice, &minted, &10_000, &1);
}
