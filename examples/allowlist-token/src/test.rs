use super::*;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::token::{Client as TokenClient, StellarAssetClient};
use soroban_sdk::{Address, Env, String};

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

    let token = TokenClient::new(&env, &client.address);
    token.transfer(&alice, &bob, &300);

    assert_eq!(token.balance(&alice), 700);
    assert_eq!(token.balance(&bob), 300);
}

#[test]
#[should_panic]
fn blocked_sender_transfer_fails() {
    let env = Env::default();
    let (client, admin, alice, bob) = setup(&env);

    client.add_to_allowlist(&admin, &bob);
    client.mint(&admin, &bob, &1000);

    client.add_to_allowlist(&admin, &alice);
    client.mint(&admin, &alice, &500);
    client.remove_from_allowlist(&admin, &alice);

    // Alice is no longer allowed — transfer should panic
    let token = TokenClient::new(&env, &client.address);
    token.transfer(&alice, &bob, &100);
}

#[test]
#[should_panic]
fn blocked_recipient_transfer_fails() {
    let env = Env::default();
    let (client, admin, alice, bob) = setup(&env);

    client.add_to_allowlist(&admin, &alice);
    client.mint(&admin, &alice, &1000);
    // Bob is NOT on the allowlist

    let token = TokenClient::new(&env, &client.address);
    token.transfer(&alice, &bob, &100);
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
    let token = TokenClient::new(&env, &client.address);
    assert_eq!(token.balance(&alice), 500);
    assert_eq!(token.total_supply(), 500);

    client.mint(&admin, &alice, &300);
    assert_eq!(token.balance(&alice), 800);
    assert_eq!(token.total_supply(), 800);
}

#[test]
#[should_panic]
fn mint_to_non_allowed_fails() {
    let env = Env::default();
    let (client, admin, alice, _) = setup(&env);
    // alice is not on allowlist
    client.mint(&admin, &alice, &100);
}

#[test]
fn sep41_metadata() {
    let env = Env::default();
    let (client, _, _, _) = setup(&env);
    let token = TokenClient::new(&env, &client.address);

    assert_eq!(token.decimals(), 7);
    assert_eq!(token.name(), String::from_str(&env, "Allowlist Token"));
    assert_eq!(token.symbol(), String::from_str(&env, "ALT"));
}

#[test]
fn approve_and_transfer_from_respects_allowlist() {
    let env = Env::default();
    let (client, admin, alice, bob) = setup(&env);
    let carol = Address::generate(&env);

    client.add_to_allowlist(&admin, &alice);
    client.add_to_allowlist(&admin, &bob);
    client.mint(&admin, &alice, &1000);

    let token = TokenClient::new(&env, &client.address);
    token.approve(&alice, &bob, &400, &1000);
    assert_eq!(token.allowance(&alice, &bob), 400);

    token.transfer_from(&bob, &alice, &carol, &100);
    assert_eq!(token.balance(&carol), 100);
    assert_eq!(token.allowance(&alice, &bob), 300);
}

#[test]
#[should_panic]
fn transfer_from_blocked_recipient_fails() {
    let env = Env::default();
    let (client, admin, alice, bob) = setup(&env);
    let carol = Address::generate(&env);

    client.add_to_allowlist(&admin, &alice);
    client.add_to_allowlist(&admin, &bob);
    client.mint(&admin, &alice, &1000);

    let token = TokenClient::new(&env, &client.address);
    token.approve(&alice, &bob, &400, &1000);

    // Carol is not on the allowlist
    token.transfer_from(&bob, &alice, &carol, &100);
}

#[test]
#[should_panic]
fn transfer_from_blocked_sender_fails() {
    let env = Env::default();
    let (client, admin, alice, bob) = setup(&env);

    client.add_to_allowlist(&admin, &alice);
    client.add_to_allowlist(&admin, &bob);
    client.mint(&admin, &alice, &1000);

    let token = TokenClient::new(&env, &client.address);
    token.approve(&alice, &bob, &400, &1000);

    client.remove_from_allowlist(&admin, &alice);
    token.transfer_from(&bob, &alice, &bob, &100);
}

#[test]
fn burn_respects_allowlist() {
    let env = Env::default();
    let (client, admin, alice, _) = setup(&env);

    client.add_to_allowlist(&admin, &alice);
    client.mint(&admin, &alice, &1000);

    let token = TokenClient::new(&env, &client.address);
    token.burn(&alice, &250);
    assert_eq!(token.balance(&alice), 750);
    assert_eq!(token.total_supply(), 750);
}

#[test]
#[should_panic]
fn burn_from_non_allowed_fails() {
    let env = Env::default();
    let (client, admin, alice, _) = setup(&env);

    client.add_to_allowlist(&admin, &alice);
    client.mint(&admin, &alice, &1000);
    client.remove_from_allowlist(&admin, &alice);

    let token = TokenClient::new(&env, &client.address);
    token.burn(&alice, &250);
}

#[test]
fn emits_transfer_event() {
    let env = Env::default();
    let (client, admin, alice, bob) = setup(&env);

    client.add_to_allowlist(&admin, &alice);
    client.add_to_allowlist(&admin, &bob);
    client.mint(&admin, &alice, &1000);

    let token = TokenClient::new(&env, &client.address);
    token.transfer(&alice, &bob, &300);

    let events = env.events().all();
    let found = events.iter().any(|(_, topics, _)| {
        topics.len() == 3
            && topics.get(0) == Some(soroban_sdk::Symbol::new(&env, "transfer").into_val(&env))
    });
    assert!(found, "expected a transfer event");
}

#[test]
fn emits_mint_and_burn_events() {
    let env = Env::default();
    let (client, admin, alice, _) = setup(&env);

    client.add_to_allowlist(&admin, &alice);
    client.mint(&admin, &alice, &1000);

    let token = TokenClient::new(&env, &client.address);
    token.burn(&alice, &100);

    let events = env.events().all();
    let has_mint = events.iter().any(|(_, topics, _)| {
        topics.len() == 3
            && topics.get(0) == Some(soroban_sdk::Symbol::new(&env, "mint").into_val(&env))
    });
    let has_burn = events.iter().any(|(_, topics, _)| {
        topics.len() == 3
            && topics.get(0) == Some(soroban_sdk::Symbol::new(&env, "burn").into_val(&env))
    });
    assert!(has_mint, "expected a mint event");
    assert!(has_burn, "expected a burn event");
}

#[test]
fn transfer_cost_does_not_grow_with_allowlist_size() {
    let env = Env::default();
    let (client, admin, alice, bob) = setup(&env);

    client.add_to_allowlist(&admin, &alice);
    client.add_to_allowlist(&admin, &bob);
    client.mint(&admin, &alice, &1_000_000);

    let token = TokenClient::new(&env, &client.address);

    // Measure cost with a small allowlist.
    let before = env.cost_estimate().budget();
    token.transfer(&alice, &bob, &1);
    let small_cost = env.cost_estimate().budget().cpu_instruction_cost()
        - before.cpu_instruction_cost();

    // Grow the allowlist with many unrelated addresses.
    for _ in 0..200 {
        let addr = Address::generate(&env);
        client.add_to_allowlist(&admin, &addr);
    }

    let before = env.cost_estimate().budget();
    token.transfer(&alice, &bob, &1);
    let large_cost = env.cost_estimate().budget().cpu_instruction_cost()
        - before.cpu_instruction_cost();

    // Transfer cost must not scale with the number of allowlisted addresses.
    assert!(
        large_cost <= small_cost + small_cost / 4,
        "transfer cost grew with allowlist size: {} -> {}",
        small_cost,
        large_cost
    );
}

#[test]
fn remove_from_allowlist_deletes_entry() {
    let env = Env::default();
    let (client, admin, alice, _) = setup(&env);

    client.add_to_allowlist(&admin, &alice);
    assert!(client.is_allowed(&alice));
    client.remove_from_allowlist(&admin, &alice);
    assert!(!client.is_allowed(&alice));

    // Re-adding works and the entry is a fresh persistent key.
    client.add_to_allowlist(&admin, &alice);
    assert!(client.is_allowed(&alice));
}
