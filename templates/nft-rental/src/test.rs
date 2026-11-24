use super::*;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env, String};

fn setup(env: &Env) -> (NftContractClient<'_>, Address) {
    env.mock_all_auths();
    let admin = Address::generate(env);
    let contract_id = env.register(
        NftContract,
        (
            admin.clone(),
            String::from_str(env, "Forge NFT"),
            String::from_str(env, "FNFT"),
        ),
    );
    (NftContractClient::new(env, &contract_id), admin)
}

#[test]
fn metadata() {
    let env = Env::default();
    let (nft, admin) = setup(&env);
    assert_eq!(nft.name(), String::from_str(&env, "Forge NFT"));
    assert_eq!(nft.symbol(), String::from_str(&env, "FNFT"));
    assert_eq!(nft.admin(), admin);
}

#[test]
fn mint_and_owner_of() {
    let env = Env::default();
    let (nft, _admin) = setup(&env);
    let alice = Address::generate(&env);
    let uri = String::from_str(&env, "https://example.com/nft/1");

    nft.mint(&alice, &101, &uri);

    assert_eq!(nft.owner_of(&101), alice);
    assert_eq!(nft.balance_of(&alice), 1);
    assert_eq!(nft.token_uri(&101), uri);
}

#[test]
fn transfer() {
    let env = Env::default();
    let (nft, _admin) = setup(&env);
    let alice = Address::generate(&env);
    let bob = Address::generate(&env);
    let uri = String::from_str(&env, "https://example.com/nft/1");

    nft.mint(&alice, &1, &uri);
    assert_eq!(nft.owner_of(&1), alice);
    assert_eq!(nft.balance_of(&alice), 1);
    assert_eq!(nft.balance_of(&bob), 0);

    nft.transfer(&alice, &bob, &1);

    assert_eq!(nft.owner_of(&1), bob);
    assert_eq!(nft.balance_of(&alice), 0);
    assert_eq!(nft.balance_of(&bob), 1);
}

#[test]
#[should_panic]
fn unauthorized_transfer() {
    let env = Env::default();
    let (nft, _admin) = setup(&env);
    let alice = Address::generate(&env);
    let bob = Address::generate(&env);
    let carol = Address::generate(&env);
    let uri = String::from_str(&env, "https://example.com/nft/1");

    nft.mint(&alice, &1, &uri);

    // Bob tries to transfer Alice's token to Carol
    nft.transfer(&bob, &carol, &1);
}

#[test]
fn burn() {
    let env = Env::default();
    let (nft, _admin) = setup(&env);
    let alice = Address::generate(&env);
    let uri = String::from_str(&env, "https://example.com/nft/1");

    nft.mint(&alice, &1, &uri);
    assert_eq!(nft.balance_of(&alice), 1);

    nft.burn(&alice, &1);
    assert_eq!(nft.balance_of(&alice), 0);
}

#[test]
#[should_panic]
fn query_burned_token_panics() {
    let env = Env::default();
    let (nft, _admin) = setup(&env);
    let alice = Address::generate(&env);
    let uri = String::from_str(&env, "https://example.com/nft/1");

    nft.mint(&alice, &1, &uri);
    nft.burn(&alice, &1);

    // Token should no longer exist
    let _owner = nft.owner_of(&1);
}

#[test]
#[should_panic]
fn mint_duplicate_token_panics() {
    let env = Env::default();
    let (nft, _admin) = setup(&env);
    let alice = Address::generate(&env);
    let uri = String::from_str(&env, "https://example.com/nft/1");

    nft.mint(&alice, &1, &uri);
    nft.mint(&alice, &1, &uri);
}

#[test]
fn approve_single_token() {
    let env = Env::default();
    let (nft, _admin) = setup(&env);
    let alice = Address::generate(&env);
    let bob = Address::generate(&env);
    let uri = String::from_str(&env, "https://example.com/nft/1");

    nft.mint(&alice, &1, &uri);
    nft.approve(&alice, &bob, &1);

    assert_eq!(nft.get_approved(&1), Some(bob.clone()));
}

#[test]
fn get_approved_returns_none_when_not_approved() {
    let env = Env::default();
    let (nft, _admin) = setup(&env);
    let alice = Address::generate(&env);
    let uri = String::from_str(&env, "https://example.com/nft/1");

    nft.mint(&alice, &1, &uri);

    assert_eq!(nft.get_approved(&1), None);
}

#[test]
#[should_panic]
fn approve_requires_owner_auth() {
    let env = Env::default();
    let (nft, _admin) = setup(&env);
    let alice = Address::generate(&env);
    let bob = Address::generate(&env);
    let carol = Address::generate(&env);
    let uri = String::from_str(&env, "https://example.com/nft/1");

    nft.mint(&alice, &1, &uri);

    // Bob (not owner) tries to approve Carol
    nft.approve(&bob, &carol, &1);
}

#[test]
fn transfer_from_by_approved_operator() {
    let env = Env::default();
    let (nft, _admin) = setup(&env);
    let alice = Address::generate(&env);
    let bob = Address::generate(&env);
    let carol = Address::generate(&env);
    let uri = String::from_str(&env, "https://example.com/nft/1");

    nft.mint(&alice, &1, &uri);
    nft.approve(&alice, &bob, &1);

    // Bob (approved) transfers token from Alice to Carol
    nft.transfer_from(&alice, &carol, &1);

    assert_eq!(nft.owner_of(&1), carol);
    assert_eq!(nft.balance_of(&alice), 0);
    assert_eq!(nft.balance_of(&carol), 1);
}

#[test]
fn approval_cleared_after_transfer_from() {
    let env = Env::default();
    let (nft, _admin) = setup(&env);
    let alice = Address::generate(&env);
    let bob = Address::generate(&env);
    let carol = Address::generate(&env);
    let uri = String::from_str(&env, "https://example.com/nft/1");

    nft.mint(&alice, &1, &uri);
    nft.approve(&alice, &bob, &1);

    nft.transfer_from(&alice, &carol, &1);

    // Approval should be cleared after transfer
    assert_eq!(nft.get_approved(&1), None);
}

#[test]
#[should_panic]
fn transfer_from_by_non_approved_fails() {
    let env = Env::default();
    let (nft, _admin) = setup(&env);
    let alice = Address::generate(&env);
    let bob = Address::generate(&env);
    let carol = Address::generate(&env);
    let uri = String::from_str(&env, "https://example.com/nft/1");

    nft.mint(&alice, &1, &uri);

    // Bob (not approved) tries to transfer Alice's token
    nft.transfer_from(&alice, &carol, &1);
}

#[test]
fn set_approval_for_all() {
    let env = Env::default();
    let (nft, _admin) = setup(&env);
    let alice = Address::generate(&env);
    let bob = Address::generate(&env);

    nft.set_approval_for_all(&alice, &bob, &true);

    assert_eq!(nft.is_approved_for_all(&alice, &bob), true);
}

#[test]
fn is_approved_for_all_returns_false_when_not_approved() {
    let env = Env::default();
    let (nft, _admin) = setup(&env);
    let alice = Address::generate(&env);
    let bob = Address::generate(&env);

    assert_eq!(nft.is_approved_for_all(&alice, &bob), false);
}

#[test]
fn revoke_approval_for_all() {
    let env = Env::default();
    let (nft, _admin) = setup(&env);
    let alice = Address::generate(&env);
    let bob = Address::generate(&env);

    nft.set_approval_for_all(&alice, &bob, &true);
    assert_eq!(nft.is_approved_for_all(&alice, &bob), true);

    nft.set_approval_for_all(&alice, &bob, &false);
    assert_eq!(nft.is_approved_for_all(&alice, &bob), false);
}

#[test]
#[should_panic]
fn set_approval_for_all_requires_auth() {
    let env = Env::default();
    let (nft, _admin) = setup(&env);
    let alice = Address::generate(&env);
    let bob = Address::generate(&env);
    let carol = Address::generate(&env);

    env.mock_all_auths_allowing_non_root_auth();
    // Carol tries to approve Bob for all of Alice's tokens
    env.as_contract(&env.current_contract_address(), || {
        nft.set_approval_for_all(&bob, &true);
    });
}

#[test]
fn transfer_from_with_approval_for_all() {
    let env = Env::default();
    let (nft, _admin) = setup(&env);
    let alice = Address::generate(&env);
    let bob = Address::generate(&env);
    let carol = Address::generate(&env);
    let uri = String::from_str(&env, "https://example.com/nft/1");

    nft.mint(&alice, &1, &uri);
    nft.set_approval_for_all(&alice, &bob, &true);

    // Bob (approved for all) transfers token from Alice to Carol
    nft.transfer_from(&alice, &carol, &1);

    assert_eq!(nft.owner_of(&1), carol);
    assert_eq!(nft.balance_of(&alice), 0);
    assert_eq!(nft.balance_of(&carol), 1);
}

#[test]
fn owner_can_transfer_from_own_token() {
    let env = Env::default();
    let (nft, _admin) = setup(&env);
    let alice = Address::generate(&env);
    let bob = Address::generate(&env);
    let uri = String::from_str(&env, "https://example.com/nft/1");

    nft.mint(&alice, &1, &uri);

    // Alice transfers her own token using transfer_from
    nft.transfer_from(&alice, &bob, &1);

    assert_eq!(nft.owner_of(&1), bob);
    assert_eq!(nft.balance_of(&alice), 0);
    assert_eq!(nft.balance_of(&bob), 1);
}
