use super::*;
use soroban_sdk::testutils::storage::{Instance as _, Persistent as _};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::testutils::{Events as _, MockAuth, MockAuthInvoke};
use soroban_sdk::testutils::Ledger as _;
use soroban_sdk::token::{StellarAssetClient, TokenClient};
use soroban_sdk::{Address, Env, IntoVal, Symbol, Val, Vec};

fn setup(env: &Env) -> (AllowlistTokenClient<'_>, Address, Address, Address) {
    env.mock_all_auths();
    let admin = Address::generate(env);
    let id = env.register(AllowlistToken, (&admin,));
    let client = AllowlistTokenClient::new(env, &id);
    let alice = Address::generate(env);
    let bob = Address::generate(env);
    (client, admin, alice, bob)
}

/// Returns a SEP-41 `token::Client` bound to the same contract address.
fn token<'a>(env: &'a Env, client: &AllowlistTokenClient<'_>) -> TokenClient<'a> {
    TokenClient::new(env, &client.address)
}

/// Collects the topics of every event emitted so far.
fn event_topics(env: &Env) -> Vec<Vec<Val>> {
    let mut topics = Vec::new(env);
    for (_, t, _) in env.events().all().iter() {
        topics.push_back(t);
    }
    topics
}

fn has_event(env: &Env, name: &str) -> bool {
    let expected: Val = Symbol::new(env, name).into_val(env);
    for t in event_topics(env).iter() {
        if t.len() >= 2 && t.get(1).unwrap() == expected {
            return true;
        }
    }
    false
}

#[test]
fn allowed_transfer_succeeds() {
    let env = Env::default();
    let (client, admin, alice, bob) = setup(&env);
    let token = token(&env, &client);

    client.add_to_allowlist(&admin, &alice);
    client.add_to_allowlist(&admin, &bob);
    client.mint(&admin, &alice, &1000);

    token.transfer(&alice, &bob, &300);

    assert_eq!(token.balance(&alice), 700);
    assert_eq!(token.balance(&bob), 300);
}

#[test]
fn sep41_metadata() {
    let env = Env::default();
    let (client, _, _, _) = setup(&env);
    let token = token(&env, &client);

    assert_eq!(token.decimals(), 7);
    assert_eq!(token.name(), String::from_str(&env, "Allowlist Token"));
    assert_eq!(token.symbol(), String::from_str(&env, "ALT"));
}

#[test]
fn approve_and_transfer_from_succeeds() {
    let env = Env::default();
    let (client, admin, alice, bob) = setup(&env);
    let token = token(&env, &client);
    let spender = Address::generate(&env);

    client.add_to_allowlist(&admin, &alice);
    client.add_to_allowlist(&admin, &bob);
    client.add_to_allowlist(&admin, &spender);
    client.mint(&admin, &alice, &1000);

    token.approve(&alice, &spender, &400, &200);
    assert_eq!(token.allowance(&alice, &spender), 400);

    token.transfer_from(&spender, &alice, &bob, &250);

    assert_eq!(token.balance(&alice), 750);
    assert_eq!(token.balance(&bob), 250);
    assert_eq!(token.allowance(&alice, &spender), 150);
}

#[test]
#[should_panic]
fn transfer_from_blocked_sender_fails() {
    let env = Env::default();
    let (client, admin, alice, bob) = setup(&env);
    let token = token(&env, &client);
    let spender = Address::generate(&env);

    client.add_to_allowlist(&admin, &alice);
    client.add_to_allowlist(&admin, &bob);
    client.add_to_allowlist(&admin, &spender);
    client.mint(&admin, &alice, &1000);
    token.approve(&alice, &spender, &400, &200);

    // Alice (the `from` account) is removed from the allowlist.
    client.remove_from_allowlist(&admin, &alice);

    token.transfer_from(&spender, &alice, &bob, &100);
}

#[test]
#[should_panic]
fn transfer_from_blocked_recipient_fails() {
    let env = Env::default();
    let (client, admin, alice, bob) = setup(&env);
    let token = token(&env, &client);
    let spender = Address::generate(&env);

    client.add_to_allowlist(&admin, &alice);
    client.add_to_allowlist(&admin, &spender);
    client.mint(&admin, &alice, &1000);
    token.approve(&alice, &spender, &400, &200);

    // Bob (the `to` account) is not on the allowlist.
    token.transfer_from(&spender, &alice, &bob, &100);
}

#[test]
fn burn_reduces_balance_and_supply() {
    let env = Env::default();
    let (client, admin, alice, _) = setup(&env);
    let token = token(&env, &client);

    client.add_to_allowlist(&admin, &alice);
    client.mint(&admin, &alice, &1000);

    token.burn(&alice, &400);

    assert_eq!(token.balance(&alice), 600);
    assert_eq!(token.total_supply(), 600);
}

#[test]
fn emits_standard_events() {
    let env = Env::default();
    let (client, admin, alice, bob) = setup(&env);
    let token = token(&env, &client);

    client.add_to_allowlist(&admin, &alice);
    client.add_to_allowlist(&admin, &bob);

    client.mint(&admin, &alice, &1000);
    assert!(has_event(&env, "mint"));

    token.transfer(&alice, &bob, &100);
    assert!(has_event(&env, "transfer"));

    token.approve(&alice, &bob, &50, &200);
    assert!(has_event(&env, "approve"));

    token.burn(&alice, &10);
    assert!(has_event(&env, "burn"));
}

#[test]
fn many_allowlisted_addresses_do_not_break_transfer() {
    let env = Env::default();
    let (client, admin, alice, bob) = setup(&env);
    let token = token(&env, &client);

    client.add_to_allowlist(&admin, &alice);
    client.add_to_allowlist(&admin, &bob);
    for _ in 0..60 {
        let who = Address::generate(&env);
        client.add_to_allowlist(&admin, &who);
    }
    client.mint(&admin, &alice, &1000);

    token.transfer(&alice, &bob, &300);
    assert_eq!(token.balance(&bob), 300);

    // Removing an entry deletes it: `is_allowed` is false and re-adding works.
    let victim = Address::generate(&env);
    client.add_to_allowlist(&admin, &victim);
    assert!(client.is_allowed(&victim));
    client.remove_from_allowlist(&admin, &victim);
    assert!(!client.is_allowed(&victim));
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
    token(&env, &client).transfer(&alice, &bob, &100);
}

#[test]
#[should_panic]
fn blocked_recipient_transfer_fails() {
    let env = Env::default();
    let (client, admin, alice, bob) = setup(&env);

    client.add_to_allowlist(&admin, &alice);
    client.mint(&admin, &alice, &1000);
    // Bob is NOT on the allowlist

    token(&env, &client).transfer(&alice, &bob, &100);
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

    let token = token(&env, &client);

    client.add_to_allowlist(&admin, &alice);
    client.mint(&admin, &alice, &500);
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
#[should_panic]
fn burn_blocked_account_fails() {
    let env = Env::default();
    let (client, admin, alice, _) = setup(&env);
    let token = token(&env, &client);

    client.add_to_allowlist(&admin, &alice);
    client.mint(&admin, &alice, &1000);
    client.remove_from_allowlist(&admin, &alice);

    token.burn(&alice, &100);
}

#[test]
fn storage_survives_default_ttl_expiry() {
    const DEFAULT_ENTRY_TTL: u32 = 4_096;

    let env = Env::default();
    let (client, admin, alice, bob) = setup(&env);
    client.add_to_allowlist(&admin, &alice);
    client.add_to_allowlist(&admin, &bob);
    client.mint(&admin, &alice, &1_000);

    let instance_ttl = env.as_contract(&client.address, || env.storage().instance().get_ttl());
    let balance_ttl = env.as_contract(&client.address, || {
        env.storage()
            .persistent()
            .get_ttl(&DataKey::Balance(alice.clone()))
    });
    assert!(instance_ttl > DEFAULT_ENTRY_TTL);
    assert!(balance_ttl > DEFAULT_ENTRY_TTL);

    env.ledger()
        .with_mut(|li| li.sequence_number += DEFAULT_ENTRY_TTL + 1);

    assert!(client.is_allowed(&alice));
    assert_eq!(client.balance(&alice), 1_000);
    assert_eq!(client.total_supply(), 1_000);

    client.transfer(&alice, &bob, &300);
    assert_eq!(client.balance(&alice), 700);
    assert_eq!(client.balance(&bob), 300);

    assert!(
        env.as_contract(&client.address, || env.storage().instance().get_ttl()) > DEFAULT_ENTRY_TTL
    );
    for addr in [&alice, &bob] {
        assert!(
            env.as_contract(&client.address, || {
                env.storage()
                    .persistent()
                    .get_ttl(&DataKey::Balance(addr.clone()))
            }) > DEFAULT_ENTRY_TTL
        );
    }
}

// --- two-step admin handover -------------------------------------------------

#[test]
fn propose_then_accept_hands_over_admin() {
    let env = Env::default();
    let (client, admin, alice, bob) = setup(&env);

    client.propose_admin(&alice);
    assert_eq!(client.pending_admin(), Some(alice.clone()));

    client.accept_admin();
    assert_eq!(client.pending_admin(), None);

    // Alice is the admin now and can mint; the old admin cannot.
    client.add_to_allowlist(&alice, &bob);
    client.mint(&alice, &bob, &10);
    assert_eq!(client.balance(&bob), 10);
    assert!(client.try_mint(&admin, &bob, &10).is_err());
}

#[test]
fn propose_and_accept_publish_events() {
    let env = Env::default();
    let (client, _admin, alice, _bob) = setup(&env);

    client.propose_admin(&alice);
    assert!(!env.events().all().events().is_empty());
    client.accept_admin();
    assert!(!env.events().all().events().is_empty());
}

#[test]
fn cancel_clears_pending_admin() {
    let env = Env::default();
    let (client, admin, alice, bob) = setup(&env);

    client.propose_admin(&alice);
    client.cancel_admin_proposal();
    assert_eq!(client.pending_admin(), None);

    // The original admin is untouched.
    client.add_to_allowlist(&admin, &bob);
    assert!(client.is_allowed(&bob));
}

#[test]
#[should_panic(expected = "Error(Contract, #7)")]
fn accept_after_cancel_fails() {
    let env = Env::default();
    let (client, _admin, alice, _bob) = setup(&env);

    client.propose_admin(&alice);
    client.cancel_admin_proposal();
    client.accept_admin();
}

#[test]
#[should_panic(expected = "Error(Auth")]
fn non_pending_address_cannot_accept() {
    let env = Env::default();
    let (client, _admin, alice, bob) = setup(&env);

    client.propose_admin(&alice);

    // Only bob authorizes; the contract requires alice's authorization.
    env.mock_auths(&[MockAuth {
        address: &bob,
        invoke: &MockAuthInvoke {
            contract: &client.address,
            fn_name: "accept_admin",
            args: ().into_val(&env),
            sub_invokes: &[],
        },
    }]);
    client.accept_admin();
}
