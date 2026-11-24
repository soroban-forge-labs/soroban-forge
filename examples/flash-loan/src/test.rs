use super::*;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{token::StellarAssetClient, token::TokenClient, Address, Env, InvokeError};

const FEE_BPS: u32 = 30; // 0.30%
/// A high fee so accrual survives the integer rounding in share accounting.
const HIGH_FEE_BPS: u32 = 1_000; // 10%
const POOL_LIQUIDITY: i128 = 1_000_000;
const BORROWER_RESERVES: i128 = 10_000;
const LOAN: i128 = 100_000;
const LOAN_FEE: i128 = 300; // 30 bps of 100_000

/// The pool reports failures by panicking with a [`Error`] variant, so a
/// `try_` call surfaces it as a raw host error rather than the enum. This
/// turns a variant into the value to compare against.
fn contract_err(error: Error) -> Result<soroban_sdk::Error, InvokeError> {
    Ok(soroban_sdk::Error::from_contract_error(error as u32))
}

/// Storage keys for the example borrowers below.
#[contracttype]
pub enum BorrowerKey {
    Pool,
    SeenBalance,
}

/// A well-behaved borrower: repays principal + fee out of its own reserves.
///
/// This is the shape a real borrower takes — the arbitrage or liquidation that
/// makes the loan worth taking would go where the comment is.
#[contract]
pub struct GoodBorrower;

#[contractimpl]
impl GoodBorrower {
    pub fn __constructor(env: Env, pool: Address) {
        env.storage().instance().set(&BorrowerKey::Pool, &pool);
    }

    pub fn exec(env: Env, pool: Address, token: Address, amount: i128, fee: i128) {
        // `exec` is public: anyone can call it with arguments they chose. Two
        // checks make that harmless. First, the pool must be the one we were
        // deployed against — a caller cannot point us at their own "pool".
        let expected: Address = env.storage().instance().get(&BorrowerKey::Pool).unwrap();
        assert_eq!(pool, expected, "callback from an unexpected pool");
        // Second, the pool must actually be the caller. A contract address's
        // authorization is satisfied by it being the direct caller, so this
        // holds during a real loan and fails for a direct call by anyone else.
        pool.require_auth();

        let client = token::Client::new(&env, &token);
        let me = env.current_contract_address();

        // Record what we hold mid-loan so a test can prove the funds really
        // arrived before the pool's repayment check ran.
        env.storage()
            .instance()
            .set(&BorrowerKey::SeenBalance, &client.balance(&me));

        // ... put the strategy here: arbitrage, liquidation, collateral swap.

        client.transfer(&me, &pool, &(amount + fee));
    }

    pub fn seen_balance(env: Env) -> i128 {
        env.storage()
            .instance()
            .get(&BorrowerKey::SeenBalance)
            .unwrap_or(0)
    }
}

/// A borrower that keeps the money. Every loan to it must revert.
#[contract]
pub struct BadBorrower;

#[contractimpl]
impl BadBorrower {
    pub fn exec(_env: Env, _pool: Address, _token: Address, _amount: i128, _fee: i128) {
        // Deliberately does nothing: the borrowed funds stay right here.
    }
}

/// Returns the principal but pockets the fee — repayment must be exact.
#[contract]
pub struct StingyBorrower;

#[contractimpl]
impl StingyBorrower {
    pub fn exec(env: Env, pool: Address, token: Address, amount: i128, _fee: i128) {
        let client = token::Client::new(&env, &token);
        client.transfer(&env.current_contract_address(), &pool, &amount);
    }
}

/// Tries to take a second loan while still inside the first one's callback.
#[contract]
pub struct ReentrantBorrower;

#[contractimpl]
impl ReentrantBorrower {
    pub fn exec(env: Env, pool: Address, token: Address, amount: i128, fee: i128) {
        let me = env.current_contract_address();
        FlashLoanPoolClient::new(&env, &pool).flash_loan(&me, &amount);
        let client = token::Client::new(&env, &token);
        client.transfer(&me, &pool, &(amount + fee));
    }
}

struct Fixture<'a> {
    pool: FlashLoanPoolClient<'a>,
    pool_id: Address,
    token: TokenClient<'a>,
    token_id: Address,
    admin: Address,
}

fn setup(env: &Env) -> Fixture<'_> {
    env.mock_all_auths();

    let issuer = Address::generate(env);
    let token_id = env.register_stellar_asset_contract_v2(issuer).address();
    let admin = Address::generate(env);
    let pool_id = env.register(FlashLoanPool, (admin.clone(), token_id.clone(), FEE_BPS));

    // The admin seeds the pool through `deposit`, which mints LP shares to the
    // admin. Minting straight to the contract would leave the pool funded with
    // no one able to withdraw — every token must be backed by a share.
    StellarAssetClient::new(env, &token_id).mint(&admin, &POOL_LIQUIDITY);
    FlashLoanPoolClient::new(env, &pool_id).deposit(&admin, &POOL_LIQUIDITY);

    Fixture {
        pool: FlashLoanPoolClient::new(env, &pool_id),
        pool_id,
        token: TokenClient::new(env, &token_id),
        token_id,
        admin,
    }
}

/// Register a borrower contract and give it reserves to pay fees from.
fn register_borrower(env: &Env, fx: &Fixture, funded: bool) -> Address {
    let id = env.register(GoodBorrower, (fx.pool_id.clone(),));
    if funded {
        StellarAssetClient::new(env, &fx.token_id).mint(&id, &BORROWER_RESERVES);
    }
    id
}

/// A pool with no seed liquidity: the first caller to deposit owns 100% of
/// it, which makes fee accrual to a single LP unambiguous.
struct EmptyPool<'a> {
    pool: FlashLoanPoolClient<'a>,
    pool_id: Address,
    token: TokenClient<'a>,
    token_id: Address,
}

fn empty_pool(env: &Env, fee_bps: u32) -> EmptyPool<'_> {
    env.mock_all_auths();

    let issuer = Address::generate(env);
    let token_id = env.register_stellar_asset_contract_v2(issuer).address();
    let admin = Address::generate(env);
    let pool_id = env.register(FlashLoanPool, (admin, token_id, fee_bps));

    EmptyPool {
        pool: FlashLoanPoolClient::new(env, &pool_id),
        pool_id,
        token: TokenClient::new(env, &token_id),
        token_id,
    }
}

#[test]
fn repaying_borrower_completes_the_loan() {
    let env = Env::default();
    let fx = setup(&env);
    let borrower = register_borrower(&env, &fx, true);

    assert_eq!(fx.pool.fee_for(&LOAN), LOAN_FEE);
    fx.pool.flash_loan(&borrower, &LOAN);

    // The pool is up exactly one fee; the borrower paid it out of reserves.
    assert_eq!(fx.token.balance(&fx.pool_id), POOL_LIQUIDITY + LOAN_FEE);
    assert_eq!(fx.token.balance(&borrower), BORROWER_RESERVES - LOAN_FEE);
}

#[test]
fn borrower_holds_the_funds_during_the_callback() {
    let env = Env::default();
    let fx = setup(&env);
    let borrower = register_borrower(&env, &fx, true);

    fx.pool.flash_loan(&borrower, &LOAN);

    // The principal really was in the borrower's hands mid-call — the loan is
    // not an accounting trick.
    let seen = GoodBorrowerClient::new(&env, &borrower).seen_balance();
    assert_eq!(seen, BORROWER_RESERVES + LOAN);
}

#[test]
#[should_panic]
fn non_repaying_borrower_reverts() {
    let env = Env::default();
    let fx = setup(&env);
    let borrower = env.register(BadBorrower, ());

    fx.pool.flash_loan(&borrower, &LOAN);
}

#[test]
fn non_repaying_borrower_leaves_no_trace() {
    let env = Env::default();
    let fx = setup(&env);
    let borrower = env.register(BadBorrower, ());

    // Same failure as above, caught so we can inspect the aftermath.
    assert_eq!(
        fx.pool.try_flash_loan(&borrower, &LOAN),
        Err(contract_err(Error::NotRepaid))
    );

    // The transfer that funded the loan was rolled back with the panic: the
    // borrower never got to keep a stroop. This is the whole security model.
    assert_eq!(fx.token.balance(&fx.pool_id), POOL_LIQUIDITY);
    assert_eq!(fx.token.balance(&borrower), 0);
}

#[test]
fn partial_repayment_reverts() {
    let env = Env::default();
    let fx = setup(&env);
    let borrower = env.register(StingyBorrower, ());
    StellarAssetClient::new(&env, &fx.token_id).mint(&borrower, &BORROWER_RESERVES);

    // Returning the principal is not enough — the fee is part of repayment.
    assert_eq!(
        fx.pool.try_flash_loan(&borrower, &LOAN),
        Err(contract_err(Error::NotRepaid))
    );
    assert_eq!(fx.token.balance(&fx.pool_id), POOL_LIQUIDITY);
    assert_eq!(fx.token.balance(&borrower), BORROWER_RESERVES);
}

#[test]
fn back_to_back_loans_accumulate_fees() {
    let env = Env::default();
    let fx = setup(&env);
    let borrower = register_borrower(&env, &fx, true);

    fx.pool.flash_loan(&borrower, &LOAN);
    fx.pool.flash_loan(&borrower, &LOAN);

    assert_eq!(fx.token.balance(&fx.pool_id), POOL_LIQUIDITY + 2 * LOAN_FEE);
    assert_eq!(
        fx.token.balance(&borrower),
        BORROWER_RESERVES - 2 * LOAN_FEE
    );
}

#[test]
fn zero_amount_is_rejected() {
    let env = Env::default();
    let fx = setup(&env);
    let borrower = register_borrower(&env, &fx, true);

    assert_eq!(
        fx.pool.try_flash_loan(&borrower, &0),
        Err(contract_err(Error::InvalidAmount))
    );
}

#[test]
fn loan_larger_than_liquidity_is_rejected() {
    let env = Env::default();
    let fx = setup(&env);
    let borrower = register_borrower(&env, &fx, true);

    assert_eq!(
        fx.pool.try_flash_loan(&borrower, &(POOL_LIQUIDITY + 1)),
        Err(contract_err(Error::InsufficientLiquidity))
    );
    assert_eq!(fx.token.balance(&fx.pool_id), POOL_LIQUIDITY);
}

#[test]
fn fee_is_rounded_up() {
    let env = Env::default();
    let fx = setup(&env);

    // 30 bps of 1 unit is 0.003 — rounded in the pool's favour, never to zero.
    assert_eq!(fx.pool.fee_for(&1), 1);
    assert_eq!(fx.pool.fee_for(&10_000), 30);
    assert_eq!(fx.pool.fee_for(&10_001), 31);
}

#[test]
fn deposit_and_withdraw_move_liquidity() {
    let env = Env::default();
    let fx = setup(&env);
    let lender = Address::generate(&env);
    StellarAssetClient::new(&env, &fx.token_id).mint(&lender, &50_000);

    fx.pool.deposit(&lender, &50_000);
    assert_eq!(fx.pool.balance(), POOL_LIQUIDITY + 50_000);

    // The admin redeems 20k of their 1M shares; the pool drops by that amount.
    fx.pool.withdraw(&fx.admin, &20_000);
    assert_eq!(fx.pool.balance(), POOL_LIQUIDITY + 50_000 - 20_000);
    assert_eq!(fx.token.balance(&fx.admin), 20_000);
}

#[test]
#[should_panic]
fn constructor_rejects_a_fee_above_the_cap() {
    let env = Env::default();
    env.mock_all_auths();
    let issuer = Address::generate(&env);
    let token_id = env.register_stellar_asset_contract_v2(issuer).address();
    let admin = Address::generate(&env);

    env.register(FlashLoanPool, (admin, token_id, MAX_FEE_BPS + 1));
}

#[test]
fn reentering_the_pool_during_the_callback_is_rejected() {
    use soroban_sdk::xdr::{ScErrorCode, ScErrorType};

    let env = Env::default();
    let fx = setup(&env);
    let borrower = env.register(ReentrantBorrower, ());
    StellarAssetClient::new(&env, &fx.token_id).mint(&borrower, &BORROWER_RESERVES);

    // Note the error: `Context, InvalidAction` is a *host* error, not one of
    // this contract's variants. Soroban refuses to call a contract already on
    // the call stack, so the pool cannot be re-entered mid-callback — no
    // reentrancy guard of our own is involved. Do not carry that assumption
    // to platforms that lack it.
    assert_eq!(
        fx.pool.try_flash_loan(&borrower, &LOAN),
        Err(Ok(soroban_sdk::Error::from_type_and_code(
            ScErrorType::Context,
            ScErrorCode::InvalidAction
        )))
    );
    assert_eq!(fx.token.balance(&fx.pool_id), POOL_LIQUIDITY);
}

#[test]
fn the_loan_path_needs_no_external_authorization() {
    let env = Env::default();
    let fx = setup(&env);
    let borrower = register_borrower(&env, &fx, true);

    // Drop the mocked authorizations `setup` installed: nothing in the loan
    // path needs a signature. The pool moves its own funds because it is the
    // caller of the token contract, and the `pool.require_auth()` inside the
    // borrower's callback is satisfied by the pool being the direct caller.
    // That is what makes the check in `GoodBorrower::exec` a real guard
    // rather than decoration.
    env.set_auths(&[]);
    fx.pool.flash_loan(&borrower, &LOAN);

    assert_eq!(fx.token.balance(&fx.pool_id), POOL_LIQUIDITY + LOAN_FEE);
}

#[test]
fn a_non_admin_depositor_can_redeem_with_fees() {
    let env = Env::default();
    // A pool seeded only by the LP we are testing, so they own 100% of every
    // fee and rounding cannot hide the accrual.
    let ep = empty_pool(&env, HIGH_FEE_BPS);
    let lp = Address::generate(&env);
    StellarAssetClient::new(&env, &ep.token_id).mint(&lp, &100_000);
    ep.pool.deposit(&lp, &100_000);

    // A loan accrues a fee to the whole pool — and the LP owns the whole pool.
    let borrower = env.register(GoodBorrower, (ep.pool_id.clone(),));
    StellarAssetClient::new(&env, &ep.token_id).mint(&borrower, &BORROWER_RESERVES);
    let loan_fee = ep.pool.fee_for(&LOAN);
    ep.pool.flash_loan(&borrower, &LOAN);

    // The LP redeems their entire position. They get back their principal
    // plus the entire fee, since no one else shares the pool.
    let total = ep.pool.balance();
    ep.pool.withdraw(&lp, &total);
    assert_eq!(ep.token.balance(&lp), 100_000 + loan_fee);
    assert_eq!(ep.pool.shares(&lp), 0);
    assert_eq!(ep.pool.total_shares(), 0);
    assert_eq!(ep.pool.balance(), 0);
}

#[test]
fn depositor_withdraws_more_than_deposited_because_of_fees() {
    let env = Env::default();
    let ep = empty_pool(&env, HIGH_FEE_BPS);
    let lp = Address::generate(&env);
    StellarAssetClient::new(&env, &ep.token_id).mint(&lp, &100_000);
    ep.pool.deposit(&lp, &100_000);

    let borrower = env.register(GoodBorrower, (ep.pool_id.clone(),));
    StellarAssetClient::new(&env, &ep.token_id).mint(&borrower, &BORROWER_RESERVES);
    ep.pool.flash_loan(&borrower, &LOAN);

    // Full redemption returns strictly more than the deposit.
    ep.pool.withdraw(&lp, &100_000);
    assert!(ep.token.balance(&lp) > 100_000);
}

#[test]
#[should_panic]
fn a_non_shareholder_cannot_withdraw() {
    let env = Env::default();
    let fx = setup(&env);
    let stranger = Address::generate(&env);

    // No deposit, no shares — the only path into the pool is deposit.
    assert_eq!(fx.pool.shares(&stranger), 0);
    fx.pool.withdraw(&stranger, &1_000);
}

#[test]
fn a_shareholder_cannot_withdraw_more_than_their_share() {
    let env = Env::default();
    let fx = setup(&env);
    let lp = Address::generate(&env);
    StellarAssetClient::new(env, &fx.token_id).mint(&lp, &100_000);
    fx.pool.deposit(&lp, &100_000);

    // The LP owns 100k of 1.1M ≈ 9%. Asking for more than that fails.
    assert_eq!(
        fx.pool.try_withdraw(&lp, &1_000_000),
        Err(contract_err(Error::InsufficientLiquidity))
    );
}

#[test]
fn deposit_then_full_redeem_returns_principal() {
    let env = Env::default();
    let fx = setup(&env);
    let lp = Address::generate(&env);
    StellarAssetClient::new(env, &fx.token_id).mint(&lp, &50_000);
    fx.pool.deposit(&lp, &50_000);

    fx.pool.withdraw(&lp, &50_000);
    assert_eq!(fx.token.balance(&lp), 50_000);
    assert_eq!(fx.pool.shares(&lp), 0);
    assert_eq!(fx.pool.total_shares(), POOL_LIQUIDITY);
}

#[test]
fn second_depositor_gets_proportional_shares() {
    let env = Env::default();
    let fx = setup(&env);
    let lp = Address::generate(&env);
    StellarAssetClient::new(env, &fx.token_id).mint(&lp, &100_000);
    fx.pool.deposit(&lp, &100_000);

    // Pool holds 1.1M backed by 1M shares; the LP holds 100_000 / 1.1M ≈
    // 90_909 shares. Full redemption returns exactly their deposit (no fees
    // have accrued), proving the price mechanism is fair, not arbitrary.
    fx.pool.withdraw(&lp, &100_000);
    assert_eq!(fx.token.balance(&lp), 100_000);
}
