use super::*;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env};

fn setup(env: &Env) -> (AllowlistTokenClient<'_>, Address, Address, Address) {
    env.mock_all_auths();
    let admin = Address::generate(env);
    let id = env.register(AllowlistToken, (&admin,));
    let client = AllowlistTokenClient::new(env, &id);
    let alice = Address::generate(env);
    let bob = Address::generate(env);
    (client, admin, alice, bob)
}

#[test]
fn allowed_transfer_succeeds() {
    let env = Env::default();
    let (client, admin, alice, bob) = setup(&env);

    client.add_to_allowlist(&admin, &alice);
    client.add_to_allowlist(&admin, &bob);
    client.mint(&admin, &alice, &1000);

    client.transfer(&alice, &bob, &300);

    assert_eq!(client.balance(&alice), 700);
    assert_eq!(client.balance(&bob), 300);
}

#[test]
#[should_panic]
fn blocked_sender_transfer_fails() {
    let env = Env::default();
    let (client, admin, alice, bob) = setup(&env);

    // Alice is NOT on the allowlist
    client.add_to_allowlist(&admin, &bob);
    client.mint(&admin, &bob, &1000);

    // Bob tries to send to Alice who is allowed, but let's test sender not allowed
    // Actually: alice is not allowed, so she can't send
    // We need alice to have tokens first - mint requires allowlist
    // So: add alice, mint, remove alice, then try transfer
    client.add_to_allowlist(&admin, &alice);
    client.mint(&admin, &alice, &500);
    client.remove_from_allowlist(&admin, &alice);

    // Alice is no longer allowed — transfer should panic
    client.transfer(&alice, &bob, &100);
}

#[test]
#[should_panic]
fn blocked_recipient_transfer_fails() {
    let env = Env::default();
    let (client, admin, alice, bob) = setup(&env);

    client.add_to_allowlist(&admin, &alice);
    client.mint(&admin, &alice, &1000);
    // Bob is NOT on the allowlist

    client.transfer(&alice, &bob, &100);
}

#[test]
fn admin_manages_allowlist() {
    let env = Env::default();
    let (client, admin, alice, _) = setup(&env);

    assert!(!client.is_allowed(&alice));
    client.add_to_allowlist(&admin, &alice);
    assert!(client.is_allowed(&alice));
    client.remove_from_allowlist(&admin, &alice);
    assert!(!client.is_allowed(&alice));
}

#[test]
#[should_panic]
fn non_admin_cannot_add_to_allowlist() {
    let env = Env::default();
    let (client, _, alice, bob) = setup(&env);

    // Alice (not admin) tries to add bob
    client.add_to_allowlist(&alice, &bob);
}

#[test]
fn mint_and_total_supply() {
    let env = Env::default();
    let (client, admin, alice, _) = setup(&env);

    client.add_to_allowlist(&admin, &alice);
    client.mint(&admin, &alice, &500);
    assert_eq!(client.balance(&alice), 500);
    assert_eq!(client.total_supply(), 500);

    client.mint(&admin, &alice, &300);
    assert_eq!(client.balance(&alice), 800);
    assert_eq!(client.total_supply(), 800);
}

#[test]
#[should_panic]
fn mint_to_non_allowed_fails() {
    let env = Env::default();
    let (client, admin, alice, _) = setup(&env);
    // alice is not on allowlist
    client.mint(&admin, &alice, &100);
}
