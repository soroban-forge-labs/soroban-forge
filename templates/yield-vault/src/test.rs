#![cfg(test)]

use super::*;
use soroban_sdk::{
    testutils::Address as _,
    token::{Client as TokenClient, StellarAssetClient},
    Address, Env,
};

struct Tok<'a> {
    client: TokenClient<'a>,
    admin: StellarAssetClient<'a>,
    address: Address,
}

fn make_token<'a>(env: &Env) -> Tok<'a> {
    let admin = Address::generate(env);
    let token_contract = env.register_stellar_asset_contract_v2(admin.clone());
    let client = TokenClient::new(env, &token_contract.address());
    let admin_client = StellarAssetClient::new(env, &token_contract.address());
    Tok {
        client,
        admin: admin_client,
        address: token_contract.address(),
    }
}

struct TestContext<'a> {
    contract_id: Address,
    vault: YieldVaultContractClient<'a>,
    asset: Tok<'a>,
    alice: Address,
    bob: Address,
    yield_provider: Address,
}

fn setup(env: &Env) -> TestContext<'_> {
    env.mock_all_auths();

    let asset = make_token(env);
    let contract_id = env.register(YieldVaultContract, (asset.address.clone(),));
    let vault = YieldVaultContractClient::new(env, &contract_id);

    let alice = Address::generate(env);
    let bob = Address::generate(env);
    let yield_provider = Address::generate(env);

    asset.admin.mint(&alice, &10_000);
    asset.admin.mint(&bob, &10_000);
    asset.admin.mint(&yield_provider, &10_000);

    TestContext {
        contract_id,
        vault,
        asset,
        alice,
        bob,
        yield_provider,
    }
}

#[test]
fn test_first_deposit() {
    let env = Env::default();
    let ctx = setup(&env);

    assert_eq!(ctx.vault.total_assets(), 0);
    assert_eq!(ctx.vault.total_shares(), 0);

    // First deposit: 1:1 ratio
    let shares = ctx.vault.deposit(&ctx.alice, &1_000);
    assert_eq!(shares, 1_000);

    assert_eq!(ctx.vault.total_assets(), 1_000);
    assert_eq!(ctx.vault.total_shares(), 1_000);
    assert_eq!(ctx.vault.balance_of(&ctx.alice), 1_000);
    assert_eq!(ctx.asset.client.balance(&ctx.contract_id), 1_000);
    assert_eq!(ctx.asset.client.balance(&ctx.alice), 9_000);
}

#[test]
fn test_proportional_shares_after_yield_increase() {
    let env = Env::default();
    let ctx = setup(&env);

    // 1. Alice deposits 1,000 assets -> gets 1,000 shares
    ctx.vault.deposit(&ctx.alice, &1_000);

    // 2. Yield provider deposits 500 assets of yield into the vault
    ctx.vault.add_yield(&ctx.yield_provider, &500);

    // Total assets = 1,500, Total shares = 1,000
    // Each share is now worth 1.5 assets
    assert_eq!(ctx.vault.total_assets(), 1_500);
    assert_eq!(ctx.vault.total_shares(), 1_000);

    // 3. Bob deposits 1,500 assets
    // Expected shares: (1500 assets * 1000 total_shares) / 1500 total_assets = 1000 shares
    let bob_shares = ctx.vault.deposit(&ctx.bob, &1_500);
    assert_eq!(bob_shares, 1_000);

    // Both Alice and Bob own 1,000 shares out of 2,000 total shares (50% each)
    assert_eq!(ctx.vault.total_shares(), 2_000);
    assert_eq!(ctx.vault.total_assets(), 3_000);
    assert_eq!(ctx.vault.balance_of(&ctx.alice), 1_000);
    assert_eq!(ctx.vault.balance_of(&ctx.bob), 1_000);
}

#[test]
fn test_full_withdrawal() {
    let env = Env::default();
    let ctx = setup(&env);

    // Alice deposits 1,000 assets -> 1,000 shares
    ctx.vault.deposit(&ctx.alice, &1_000);

    // 500 assets of yield added
    ctx.vault.add_yield(&ctx.yield_provider, &500);

    // Bob deposits 1,500 assets -> 1,000 shares
    ctx.vault.deposit(&ctx.bob, &1_500);

    // Alice withdraws all 1,000 shares
    // Entitled assets: (1000 shares * 3000 total_assets) / 2000 total_shares = 1500 assets
    let alice_assets = ctx.vault.withdraw(&ctx.alice, &1_000);
    assert_eq!(alice_assets, 1_500);
    assert_eq!(ctx.vault.balance_of(&ctx.alice), 0);
    // Alice started with 10_000, deposited 1_000, withdrew 1_500 -> 10_500
    assert_eq!(ctx.asset.client.balance(&ctx.alice), 10_500);

    // Remaining vault state: 1,500 assets, 1,000 shares
    assert_eq!(ctx.vault.total_assets(), 1_500);
    assert_eq!(ctx.vault.total_shares(), 1_000);

    // Bob withdraws all 1,000 shares
    let bob_assets = ctx.vault.withdraw(&ctx.bob, &1_000);
    assert_eq!(bob_assets, 1_500);
    assert_eq!(ctx.vault.balance_of(&ctx.bob), 0);
    assert_eq!(ctx.asset.client.balance(&ctx.bob), 10_000);

    // Vault is fully empty
    assert_eq!(ctx.vault.total_assets(), 0);
    assert_eq!(ctx.vault.total_shares(), 0);
    assert_eq!(ctx.asset.client.balance(&ctx.contract_id), 0);
}

#[test]
fn test_rounding_favours_vault() {
    let env = Env::default();
    let ctx = setup(&env);

    // Initial state: Alice deposits 1,000 -> 1,000 shares
    ctx.vault.deposit(&ctx.alice, &1_000);

    // Add 500 yield -> 1,500 assets, 1,000 shares (1.5 assets per share)
    ctx.vault.add_yield(&ctx.yield_provider, &500);

    // 1. Assert deposit rounding direction:
    // Bob deposits 10 assets.
    // Exact mathematical shares: 10 * 1000 / 1500 = 6.6666...
    // Vault-favoured policy (floor): 6 shares.
    let preview_shares = ctx.vault.convert_to_shares(&10);
    assert_eq!(preview_shares, 6);

    let minted_shares = ctx.vault.deposit(&ctx.bob, &10);
    assert_eq!(minted_shares, 6);

    // Vault now has:
    // total_assets = 1510
    // total_shares = 1006
    assert_eq!(ctx.vault.total_assets(), 1_510);
    assert_eq!(ctx.vault.total_shares(), 1_006);

    // 2. Assert withdrawal rounding direction:
    // Bob immediately redeems his 6 shares.
    // Exact mathematical assets: 6 * 1510 / 1006 = 9.00596...
    // Vault-favoured policy (floor): 9 assets.
    let preview_assets = ctx.vault.convert_to_assets(&6);
    assert_eq!(preview_assets, 9);

    let redeemed_assets = ctx.vault.withdraw(&ctx.bob, &6);
    assert_eq!(redeemed_assets, 9);

    // Bob deposited 10 assets and received 9 back. The 1 asset difference remains
    // in the vault as surplus for remaining shareholders.
    assert_eq!(ctx.vault.total_assets(), 1_501);
    assert_eq!(ctx.vault.total_shares(), 1_000);
}

#[test]
#[should_panic(expected = "Error(Contract, #3)")]
fn test_cannot_deposit_zero() {
    let env = Env::default();
    let ctx = setup(&env);
    ctx.vault.deposit(&ctx.alice, &0);
}

#[test]
#[should_panic(expected = "Error(Contract, #4)")]
fn test_cannot_withdraw_more_than_balance() {
    let env = Env::default();
    let ctx = setup(&env);
    ctx.vault.deposit(&ctx.alice, &500);
    ctx.vault.withdraw(&ctx.alice, &501);
}

#[test]
#[should_panic(expected = "Error(Contract, #7)")]
fn test_cannot_add_yield_to_empty_vault() {
    let env = Env::default();
    env.mock_all_auths();
    let asset = make_token(&env);
    let contract_id = env.register(YieldVaultContract, (asset.address.clone(),));
    let vault = YieldVaultContractClient::new(&env, &contract_id);

    let provider = Address::generate(&env);
    asset.admin.mint(&provider, &1_000);
    vault.add_yield(&provider, &100);
}

#[test]
#[should_panic(expected = "Error(Contract, #5)")]
fn test_cannot_seed_empty_vault_with_dust_deposit() {
    let env = Env::default();
    let ctx = setup(&env);
    ctx.vault.deposit(&ctx.alice, &1);
}

#[test]
fn test_transfer_shares_and_withdraw_by_receiver() {
    let env = Env::default();
    let ctx = setup(&env);

    ctx.vault.deposit(&ctx.alice, &500);
    assert_eq!(ctx.vault.balance_of(&ctx.alice), 500);

    ctx.vault.transfer(&ctx.alice, &ctx.bob, &200);
    assert_eq!(ctx.vault.balance_of(&ctx.alice), 300);
    assert_eq!(ctx.vault.balance_of(&ctx.bob), 200);

    // Bob, who received shares via transfer, can redeem them himself.
    let assets = ctx.vault.withdraw(&ctx.bob, &200);
    assert_eq!(assets, 200);
    assert_eq!(ctx.vault.balance_of(&ctx.bob), 0);
}

#[test]
fn test_approve_and_transfer_from() {
    let env = Env::default();
    let ctx = setup(&env);

    ctx.vault.deposit(&ctx.alice, &500);
    ctx.vault.approve(&ctx.alice, &ctx.bob, &150);
    assert_eq!(ctx.vault.allowance(&ctx.alice, &ctx.bob), 150);

    ctx.vault.transfer_from(&ctx.bob, &ctx.alice, &ctx.bob, &100);
    assert_eq!(ctx.vault.balance_of(&ctx.alice), 400);
    assert_eq!(ctx.vault.balance_of(&ctx.bob), 100);
    assert_eq!(ctx.vault.allowance(&ctx.alice, &ctx.bob), 50);
}

#[test]
#[should_panic(expected = "Error(Contract, #8)")]
fn test_transfer_from_without_allowance_fails() {
    let env = Env::default();
    let ctx = setup(&env);

    ctx.vault.deposit(&ctx.alice, &500);
    ctx.vault.transfer_from(&ctx.bob, &ctx.alice, &ctx.bob, &100);
}
