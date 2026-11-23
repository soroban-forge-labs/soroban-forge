extern crate std;

use super::*;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{
    contract, contractimpl, token::StellarAssetClient, token::TokenClient, Address, Env, String,
};

/// Mock NFT contract implementing the `nft` template interface.
#[contract]
pub struct MockNftContract;

#[contractimpl]
impl MockNftContract {
    pub fn __constructor(env: Env, admin: Address, name: String, symbol: String) {
        env.storage().instance().set(&1u32, &admin);
        env.storage().instance().set(&2u32, &name);
        env.storage().instance().set(&3u32, &symbol);
    }

    pub fn owner_of(env: Env, token_id: u32) -> Address {
        env.storage().persistent().get(&token_id).unwrap()
    }

    pub fn balance_of(env: Env, owner: Address) -> u32 {
        env.storage().persistent().get(&owner).unwrap_or(0)
    }

    pub fn mint(env: Env, to: Address, token_id: u32, _uri: String) {
        env.storage().persistent().set(&token_id, &to);
        let b = env.storage().persistent().get::<_, u32>(&to).unwrap_or(0);
        env.storage().persistent().set(&to, &(b + 1));
    }

    pub fn transfer(env: Env, from: Address, to: Address, token_id: u32) {
        from.require_auth();
        let owner: Address = env.storage().persistent().get(&token_id).unwrap();
        if owner != from {
            panic!("not authorized");
        }
        env.storage().persistent().set(&token_id, &to);
        let from_b = env.storage().persistent().get::<_, u32>(&from).unwrap_or(0);
        if from_b > 0 {
            env.storage().persistent().set(&from, &(from_b - 1));
        }
        let to_b = env.storage().persistent().get::<_, u32>(&to).unwrap_or(0);
        env.storage().persistent().set(&to, &(to_b + 1));
    }
}

struct PaymentTok<'a> {
    address: Address,
    client: TokenClient<'a>,
    admin: StellarAssetClient<'a>,
}

fn make_payment_token(env: &Env) -> PaymentTok<'_> {
    let issuer = Address::generate(env);
    let sac = env.register_stellar_asset_contract_v2(issuer);
    let address = sac.address();
    PaymentTok {
        client: TokenClient::new(env, &address),
        admin: StellarAssetClient::new(env, &address),
        address,
    }
}

struct TestContext<'a> {
    marketplace_id: Address,
    marketplace: NftMarketplaceContractClient<'a>,
    nft_id: Address,
    nft: MockNftContractClient<'a>,
    payment: PaymentTok<'a>,
    admin: Address,
    treasury: Address,
    seller: Address,
    buyer: Address,
}

fn setup(env: &Env, fee_bps: u32) -> TestContext<'_> {
    env.mock_all_auths();

    let admin = Address::generate(env);
    let treasury = Address::generate(env);
    let seller = Address::generate(env);
    let buyer = Address::generate(env);

    let marketplace_id = env.register(
        NftMarketplaceContract,
        (admin.clone(), treasury.clone(), fee_bps),
    );
    let marketplace = NftMarketplaceContractClient::new(env, &marketplace_id);

    let nft_id = env.register(
        MockNftContract,
        (
            admin.clone(),
            String::from_str(env, "Forge NFT"),
            String::from_str(env, "FNFT"),
        ),
    );
    let nft = MockNftContractClient::new(env, &nft_id);

    let payment = make_payment_token(env);

    TestContext {
        marketplace_id,
        marketplace,
        nft_id,
        nft,
        payment,
        admin,
        treasury,
        seller,
        buyer,
    }
}

#[test]
fn test_initialization() {
    let env = Env::default();
    let ctx = setup(&env, 250);

    assert_eq!(ctx.marketplace.admin(), ctx.admin);
    assert_eq!(ctx.marketplace.treasury(), ctx.treasury);
    assert_eq!(ctx.marketplace.fee_bps(), 250);
}

#[test]
fn test_list_and_get_listing() {
    let env = Env::default();
    let ctx = setup(&env, 250);

    // Mint NFT to seller
    ctx.nft.mint(
        &ctx.seller,
        &1,
        &String::from_str(&env, "https://example.com/1"),
    );
    assert_eq!(ctx.nft.owner_of(&1), ctx.seller);

    // Seller lists NFT
    ctx.marketplace
        .list(&ctx.seller, &ctx.nft_id, &1, &ctx.payment.address, &1_000);

    // NFT is now held in escrow by marketplace contract
    assert_eq!(ctx.nft.owner_of(&1), ctx.marketplace_id);

    // Query listing
    let listing = ctx.marketplace.get_listing(&ctx.nft_id, &1).unwrap();
    assert_eq!(listing.seller, ctx.seller);
    assert_eq!(listing.nft_contract, ctx.nft_id);
    assert_eq!(listing.token_id, 1);
    assert_eq!(listing.payment_token, ctx.payment.address);
    assert_eq!(listing.price, 1_000);
}

#[test]
fn test_buy_and_fee_accounting() {
    let env = Env::default();
    // 500 bps = 5% fee
    let ctx = setup(&env, 500);

    ctx.nft.mint(
        &ctx.seller,
        &1,
        &String::from_str(&env, "https://example.com/1"),
    );
    ctx.marketplace
        .list(&ctx.seller, &ctx.nft_id, &1, &ctx.payment.address, &1_000);

    // Mint payment tokens to buyer
    ctx.payment.admin.mint(&ctx.buyer, &2_000);

    // Buyer purchases NFT
    ctx.marketplace.buy(&ctx.buyer, &ctx.nft_id, &1);

    // Verify NFT ownership transferred to buyer
    assert_eq!(ctx.nft.owner_of(&1), ctx.buyer);
    assert_eq!(ctx.nft.balance_of(&ctx.buyer), 1);
    assert_eq!(ctx.nft.balance_of(&ctx.seller), 0);

    // Verify listing is removed
    assert_eq!(ctx.marketplace.get_listing(&ctx.nft_id, &1), None);

    // Fee accounting:
    // Total price: 1000
    // Fee: 1000 * 500 / 10000 = 50 (to treasury)
    // Seller: 1000 - 50 = 950 (to seller)
    // Buyer remaining: 2000 - 1000 = 1000
    assert_eq!(ctx.payment.client.balance(&ctx.treasury), 50);
    assert_eq!(ctx.payment.client.balance(&ctx.seller), 950);
    assert_eq!(ctx.payment.client.balance(&ctx.buyer), 1_000);
}

#[test]
fn test_buy_with_zero_fee() {
    let env = Env::default();
    let ctx = setup(&env, 0);

    ctx.nft.mint(
        &ctx.seller,
        &42,
        &String::from_str(&env, "https://example.com/42"),
    );
    ctx.marketplace
        .list(&ctx.seller, &ctx.nft_id, &42, &ctx.payment.address, &500);

    ctx.payment.admin.mint(&ctx.buyer, &500);
    ctx.marketplace.buy(&ctx.buyer, &ctx.nft_id, &42);

    assert_eq!(ctx.nft.owner_of(&42), ctx.buyer);
    assert_eq!(ctx.payment.client.balance(&ctx.treasury), 0);
    assert_eq!(ctx.payment.client.balance(&ctx.seller), 500);
    assert_eq!(ctx.payment.client.balance(&ctx.buyer), 0);
}

#[test]
fn test_buy_with_max_fee() {
    let env = Env::default();
    // MAX_FEE_BPS = 1000 bps = 10% fee
    let ctx = setup(&env, MAX_FEE_BPS);

    ctx.nft.mint(
        &ctx.seller,
        &7,
        &String::from_str(&env, "https://example.com/7"),
    );
    ctx.marketplace
        .list(&ctx.seller, &ctx.nft_id, &7, &ctx.payment.address, &1_000);

    ctx.payment.admin.mint(&ctx.buyer, &1_000);
    ctx.marketplace.buy(&ctx.buyer, &ctx.nft_id, &7);

    assert_eq!(ctx.nft.owner_of(&7), ctx.buyer);
    assert_eq!(ctx.payment.client.balance(&ctx.treasury), 100);
    assert_eq!(ctx.payment.client.balance(&ctx.seller), 900);
    assert_eq!(ctx.payment.client.balance(&ctx.buyer), 0);
}
#[test]
fn test_cancel() {
    let env = Env::default();
    let ctx = setup(&env, 250);

    ctx.nft.mint(
        &ctx.seller,
        &10,
        &String::from_str(&env, "https://example.com/10"),
    );
    ctx.marketplace
        .list(&ctx.seller, &ctx.nft_id, &10, &ctx.payment.address, &1_000);

    assert_eq!(ctx.nft.owner_of(&10), ctx.marketplace_id);

    // Seller cancels listing
    ctx.marketplace.cancel(&ctx.seller, &ctx.nft_id, &10);

    // NFT returned to seller
    assert_eq!(ctx.nft.owner_of(&10), ctx.seller);
    assert_eq!(ctx.marketplace.get_listing(&ctx.nft_id, &10), None);
}

#[test]
#[should_panic(expected = "Error(Contract, #1)")]
fn test_unauthorized_cancel_by_other_user() {
    let env = Env::default();
    let ctx = setup(&env, 250);

    ctx.nft.mint(
        &ctx.seller,
        &10,
        &String::from_str(&env, "https://example.com/10"),
    );
    ctx.marketplace
        .list(&ctx.seller, &ctx.nft_id, &10, &ctx.payment.address, &1_000);

    let mallory = Address::generate(&env);
    // Mallory attempts to cancel Alice's listing
    ctx.marketplace.cancel(&mallory, &ctx.nft_id, &10);
}

#[test]
fn test_admin_updates() {
    let env = Env::default();
    let ctx = setup(&env, 250);

    let new_treasury = Address::generate(&env);
    let new_admin = Address::generate(&env);

    ctx.marketplace.set_fee_bps(&300);
    assert_eq!(ctx.marketplace.fee_bps(), 300);

    ctx.marketplace.set_treasury(&new_treasury);
    assert_eq!(ctx.marketplace.treasury(), new_treasury);

    ctx.marketplace.set_admin(&new_admin);
    assert_eq!(ctx.marketplace.admin(), new_admin);
}

#[test]
#[should_panic(expected = "Error(Contract, #4)")]
fn test_list_invalid_price() {
    let env = Env::default();
    let ctx = setup(&env, 250);

    ctx.nft.mint(
        &ctx.seller,
        &1,
        &String::from_str(&env, "https://example.com/1"),
    );
    ctx.marketplace
        .list(&ctx.seller, &ctx.nft_id, &1, &ctx.payment.address, &0);
}

#[test]
#[should_panic(expected = "Error(Contract, #2)")]
fn test_list_already_listed() {
    let env = Env::default();
    let ctx = setup(&env, 250);

    ctx.nft.mint(
        &ctx.seller,
        &1,
        &String::from_str(&env, "https://example.com/1"),
    );
    ctx.marketplace
        .list(&ctx.seller, &ctx.nft_id, &1, &ctx.payment.address, &1_000);

    // Attempting to list same token again
    ctx.marketplace
        .list(&ctx.seller, &ctx.nft_id, &1, &ctx.payment.address, &2_000);
}

#[test]
#[should_panic(expected = "Error(Contract, #3)")]
fn test_buy_nonexistent_listing() {
    let env = Env::default();
    let ctx = setup(&env, 250);

    ctx.marketplace.buy(&ctx.buyer, &ctx.nft_id, &999);
}

#[test]
#[should_panic(expected = "Error(Contract, #3)")]
fn test_cancel_nonexistent_listing() {
    let env = Env::default();
    let ctx = setup(&env, 250);

    ctx.marketplace.cancel(&ctx.seller, &ctx.nft_id, &999);
}

#[test]
#[should_panic(expected = "Error(Contract, #5)")]
fn test_set_fee_bps_too_high() {
    let env = Env::default();
    let ctx = setup(&env, 250);

    ctx.marketplace.set_fee_bps(&10_001);
}

#[test]
fn test_listing_records_fee_at_creation() {
    let env = Env::default();
    let ctx = setup(&env, 500);

    ctx.nft.mint(
        &ctx.seller,
        &1,
        &String::from_str(&env, "https://example.com/1"),
    );
    ctx.marketplace
        .list(&ctx.seller, &ctx.nft_id, &1, &ctx.payment.address, &1_000);

    // A later fee change must not touch the stored listing.
    ctx.marketplace.set_fee_bps(&MAX_FEE_BPS);

    let listing = ctx.marketplace.get_listing(&ctx.nft_id, &1).unwrap();
    assert_eq!(listing.fee_bps, 500);
    assert_eq!(ctx.marketplace.fee_bps(), MAX_FEE_BPS);
}

#[test]
fn test_fee_raised_after_listing_does_not_change_seller_payout() {
    let env = Env::default();
    // 500 bps = 5% when the seller lists.
    let ctx = setup(&env, 500);

    ctx.nft.mint(
        &ctx.seller,
        &1,
        &String::from_str(&env, "https://example.com/1"),
    );
    ctx.marketplace
        .list(&ctx.seller, &ctx.nft_id, &1, &ctx.payment.address, &1_000);

    // Admin raises the fee to the cap before the purchase lands.
    ctx.marketplace.set_fee_bps(&MAX_FEE_BPS);

    ctx.payment.admin.mint(&ctx.buyer, &1_000);
    ctx.marketplace.buy(&ctx.buyer, &ctx.nft_id, &1);

    // Seller still gets 1000 - 5% = 950; the raised fee is not applied.
    assert_eq!(ctx.payment.client.balance(&ctx.seller), 950);
    assert_eq!(ctx.payment.client.balance(&ctx.treasury), 50);
    assert_eq!(ctx.payment.client.balance(&ctx.buyer), 0);
    assert_eq!(ctx.nft.owner_of(&1), ctx.buyer);
}

#[test]
fn test_fee_lowered_after_listing_still_uses_listed_fee() {
    let env = Env::default();
    let ctx = setup(&env, 500);

    ctx.nft.mint(
        &ctx.seller,
        &1,
        &String::from_str(&env, "https://example.com/1"),
    );
    ctx.marketplace
        .list(&ctx.seller, &ctx.nft_id, &1, &ctx.payment.address, &1_000);

    ctx.marketplace.set_fee_bps(&0);

    ctx.payment.admin.mint(&ctx.buyer, &1_000);
    ctx.marketplace.buy(&ctx.buyer, &ctx.nft_id, &1);

    // The fee agreed at listing time applies in both directions.
    assert_eq!(ctx.payment.client.balance(&ctx.seller), 950);
    assert_eq!(ctx.payment.client.balance(&ctx.treasury), 50);
}

#[test]
fn test_new_listing_uses_the_new_fee() {
    let env = Env::default();
    let ctx = setup(&env, 500);

    ctx.marketplace.set_fee_bps(&200);

    ctx.nft.mint(
        &ctx.seller,
        &1,
        &String::from_str(&env, "https://example.com/1"),
    );
    ctx.marketplace
        .list(&ctx.seller, &ctx.nft_id, &1, &ctx.payment.address, &1_000);
    assert_eq!(
        ctx.marketplace
            .get_listing(&ctx.nft_id, &1)
            .unwrap()
            .fee_bps,
        200
    );

    ctx.payment.admin.mint(&ctx.buyer, &1_000);
    ctx.marketplace.buy(&ctx.buyer, &ctx.nft_id, &1);
    assert_eq!(ctx.payment.client.balance(&ctx.seller), 980);
    assert_eq!(ctx.payment.client.balance(&ctx.treasury), 20);
}

#[test]
fn test_set_fee_bps_accepts_cap_and_rejects_above_it() {
    let env = Env::default();
    let ctx = setup(&env, 250);

    ctx.marketplace.set_fee_bps(&MAX_FEE_BPS);
    assert_eq!(ctx.marketplace.fee_bps(), MAX_FEE_BPS);

    assert!(ctx.marketplace.try_set_fee_bps(&(MAX_FEE_BPS + 1)).is_err());
    // The rejected call changed nothing.
    assert_eq!(ctx.marketplace.fee_bps(), MAX_FEE_BPS);
}

#[test]
#[should_panic]
fn test_constructor_rejects_fee_above_cap() {
    let env = Env::default();
    setup(&env, MAX_FEE_BPS + 1);
}
