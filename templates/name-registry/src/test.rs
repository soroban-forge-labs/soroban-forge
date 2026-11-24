use super::*;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{symbol_short, Address, Env};

/// A role with no special meaning to the contract, used to show that role
/// names are just symbols.
const MANAGER_ROLE: Symbol = symbol_short!("manager");

struct Fixture<'a> {
    client: AccessControlContractClient<'a>,
    admin: Address,
    alice: Address,
    bob: Address,
}

fn setup(env: &Env) -> Fixture<'_> {
    env.mock_all_auths();
    let admin = Address::generate(env);
    let id = env.register(AccessControlContract, (&admin,));
    Fixture {
        client: AccessControlContractClient::new(env, &id),
        admin,
        alice: Address::generate(env),
        bob: Address::generate(env),
    }
}

// --- roles ------------------------------------------------------------------

#[test]
fn constructor_grants_admin_role() {
    let env = Env::default();
    let f = setup(&env);

    assert!(f.client.has_role(&ADMIN_ROLE, &f.admin));
    assert_eq!(f.client.role_member_count(&ADMIN_ROLE), 1);
}

#[test]
fn admin_grants_and_revokes_a_role() {
    let env = Env::default();
    let f = setup(&env);

    assert!(!f.client.has_role(&MINTER_ROLE, &f.alice));

    f.client.grant_role(&f.admin, &MINTER_ROLE, &f.alice);
    assert!(f.client.has_role(&MINTER_ROLE, &f.alice));
    assert_eq!(f.client.role_member_count(&MINTER_ROLE), 1);

    f.client.revoke_role(&f.admin, &MINTER_ROLE, &f.alice);
    assert!(!f.client.has_role(&MINTER_ROLE, &f.alice));
    assert_eq!(f.client.role_member_count(&MINTER_ROLE), 0);
}

#[test]
fn granting_twice_is_idempotent() {
    let env = Env::default();
    let f = setup(&env);

    f.client.grant_role(&f.admin, &MINTER_ROLE, &f.alice);
    f.client.grant_role(&f.admin, &MINTER_ROLE, &f.alice);

    assert!(f.client.has_role(&MINTER_ROLE, &f.alice));
    assert_eq!(f.client.role_member_count(&MINTER_ROLE), 1);
}

#[test]
fn revoking_a_role_not_held_is_a_noop() {
    let env = Env::default();
    let f = setup(&env);

    f.client.revoke_role(&f.admin, &MINTER_ROLE, &f.alice);

    assert!(!f.client.has_role(&MINTER_ROLE, &f.alice));
    assert_eq!(f.client.role_member_count(&MINTER_ROLE), 0);
}

#[test]
#[should_panic(expected = "Error(Contract, #1)")]
fn non_admin_cannot_grant_a_role() {
    let env = Env::default();
    let f = setup(&env);

    // Alice holds no role at all.
    f.client.grant_role(&f.alice, &MINTER_ROLE, &f.bob);
}

#[test]
#[should_panic(expected = "Error(Contract, #1)")]
fn non_admin_cannot_revoke_a_role() {
    let env = Env::default();
    let f = setup(&env);

    f.client.grant_role(&f.admin, &MINTER_ROLE, &f.bob);
    f.client.revoke_role(&f.alice, &MINTER_ROLE, &f.bob);
}

#[test]
fn renounce_role_removes_own_role() {
    let env = Env::default();
    let f = setup(&env);

    f.client.grant_role(&f.admin, &MINTER_ROLE, &f.alice);
    f.client.renounce_role(&f.alice, &MINTER_ROLE);

    assert!(!f.client.has_role(&MINTER_ROLE, &f.alice));
    assert_eq!(f.client.role_member_count(&MINTER_ROLE), 0);
}

// --- an admin role administering other roles --------------------------------

#[test]
fn role_admin_defaults_to_admin_role() {
    let env = Env::default();
    let f = setup(&env);

    assert_eq!(f.client.get_role_admin(&MINTER_ROLE), ADMIN_ROLE);
}

#[test]
fn delegated_role_admin_can_grant() {
    let env = Env::default();
    let f = setup(&env);

    // The admin hands MINTER_ROLE over to a new manager role, then makes Bob a
    // manager. Bob holds no admin role, but can now hand out MINTER_ROLE.
    f.client
        .set_role_admin(&f.admin, &MINTER_ROLE, &MANAGER_ROLE);
    f.client.grant_role(&f.admin, &MANAGER_ROLE, &f.bob);

    assert_eq!(f.client.get_role_admin(&MINTER_ROLE), MANAGER_ROLE);
    assert!(!f.client.has_role(&ADMIN_ROLE, &f.bob));

    f.client.grant_role(&f.bob, &MINTER_ROLE, &f.alice);
    assert!(f.client.has_role(&MINTER_ROLE, &f.alice));
}

#[test]
#[should_panic(expected = "Error(Contract, #1)")]
fn delegated_admin_cannot_grant_an_unrelated_role() {
    let env = Env::default();
    let f = setup(&env);

    f.client
        .set_role_admin(&f.admin, &MINTER_ROLE, &MANAGER_ROLE);
    f.client.grant_role(&f.admin, &MANAGER_ROLE, &f.bob);

    // Bob administers MINTER_ROLE only — BURNER_ROLE is still the admin's.
    f.client.grant_role(&f.bob, &BURNER_ROLE, &f.alice);
}

#[test]
#[should_panic(expected = "Error(Contract, #1)")]
fn non_admin_cannot_set_role_admin() {
    let env = Env::default();
    let f = setup(&env);

    f.client.grant_role(&f.admin, &MINTER_ROLE, &f.alice);
    f.client
        .set_role_admin(&f.alice, &BURNER_ROLE, &MANAGER_ROLE);
}

// --- last-admin guard -------------------------------------------------------

#[test]
#[should_panic(expected = "Error(Contract, #2)")]
fn cannot_revoke_the_last_admin() {
    let env = Env::default();
    let f = setup(&env);

    f.client.revoke_role(&f.admin, &ADMIN_ROLE, &f.admin);
}

#[test]
#[should_panic(expected = "Error(Contract, #2)")]
fn cannot_renounce_the_last_admin() {
    let env = Env::default();
    let f = setup(&env);

    f.client.renounce_role(&f.admin, &ADMIN_ROLE);
}

#[test]
fn a_second_admin_can_be_revoked() {
    let env = Env::default();
    let f = setup(&env);

    f.client.grant_role(&f.admin, &ADMIN_ROLE, &f.alice);
    assert_eq!(f.client.role_member_count(&ADMIN_ROLE), 2);

    f.client.revoke_role(&f.admin, &ADMIN_ROLE, &f.alice);
    assert!(!f.client.has_role(&ADMIN_ROLE, &f.alice));
    assert!(f.client.has_role(&ADMIN_ROLE, &f.admin));
    assert_eq!(f.client.role_member_count(&ADMIN_ROLE), 1);
}

// --- role-gated entrypoints -------------------------------------------------

#[test]
fn role_gated_mint_succeeds_for_a_minter() {
    let env = Env::default();
    let f = setup(&env);

    f.client.grant_role(&f.admin, &MINTER_ROLE, &f.alice);
    f.client.mint(&f.alice, &f.bob, &1000);

    assert_eq!(f.client.balance(&f.bob), 1000);
    assert_eq!(f.client.total_supply(), 1000);
}

#[test]
fn burn_only_debits_the_owner_balance() {
    let env = Env::default();
    let f = setup(&env);

    f.client.grant_role(&f.admin, &MINTER_ROLE, &f.alice);
    f.client.grant_role(&f.admin, &BURNER_ROLE, &f.bob);

    f.client.mint(&f.alice, &f.bob, &1000);
    f.client.burn(&f.bob, &400);

    assert_eq!(f.client.balance(&f.bob), 600);
    assert_eq!(f.client.total_supply(), 600);
}

#[test]
fn force_burn_allows_an_admin_burner_to_debit_any_balance() {
    let env = Env::default();
    let f = setup(&env);

    f.client.grant_role(&f.admin, &MINTER_ROLE, &f.alice);
    f.client.grant_role(&f.admin, &BURNER_ROLE, &f.bob);

    f.client.mint(&f.alice, &f.alice, &1000);
    f.client.mint(&f.alice, &f.bob, &1000);
    f.client.force_burn(&f.bob, &f.alice, &400);

    assert_eq!(f.client.balance(&f.alice), 600);
    assert_eq!(f.client.balance(&f.bob), 1000);
    assert_eq!(f.client.total_supply(), 1600);
}

#[test]
#[should_panic(expected = "Error(Contract, #1)")]
fn unauthorized_caller_cannot_mint() {
    let env = Env::default();
    let f = setup(&env);

    // Alice holds no role.
    f.client.mint(&f.alice, &f.bob, &1000);
}

#[test]
#[should_panic(expected = "Error(Contract, #1)")]
fn the_admin_alone_cannot_mint() {
    let env = Env::default();
    let f = setup(&env);

    // Holding ADMIN_ROLE means you can hand out MINTER_ROLE, not use it.
    f.client.mint(&f.admin, &f.bob, &1000);
}

#[test]
#[should_panic(expected = "Error(Contract, #1)")]
fn a_minter_cannot_force_burn() {
    let env = Env::default();
    let f = setup(&env);

    f.client.grant_role(&f.admin, &MINTER_ROLE, &f.alice);
    f.client.mint(&f.alice, &f.bob, &1000);
    f.client.force_burn(&f.alice, &f.bob, &100);
}

#[test]
#[should_panic(expected = "InvalidAction")]
fn burn_requires_the_owner_to_authorize() {
    let env = Env::default();
    env.mock_all_auths();
    let f = setup(&env);

    f.client.grant_role(&f.admin, &MINTER_ROLE, &f.alice);
    f.client.mint(&f.alice, &f.bob, &1000);

    env.set_auths(&[]);
    f.client.burn(&f.bob, &100);
}

#[test]
#[should_panic(expected = "Error(Contract, #1)")]
fn a_revoked_minter_can_no_longer_mint() {
    let env = Env::default();
    let f = setup(&env);

    f.client.grant_role(&f.admin, &MINTER_ROLE, &f.alice);
    f.client.mint(&f.alice, &f.bob, &1000);

    f.client.revoke_role(&f.admin, &MINTER_ROLE, &f.alice);
    f.client.mint(&f.alice, &f.bob, &1);
}

#[test]
#[should_panic(expected = "Error(Contract, #3)")]
fn mint_rejects_a_non_positive_amount() {
    let env = Env::default();
    let f = setup(&env);

    f.client.grant_role(&f.admin, &MINTER_ROLE, &f.alice);
    f.client.mint(&f.alice, &f.bob, &0);
}

#[test]
#[should_panic(expected = "Error(Contract, #4)")]
fn burn_rejects_more_than_the_balance() {
    let env = Env::default();
    let f = setup(&env);

    f.client.burn(&f.bob, &1);
}

// --- authorization ----------------------------------------------------------

#[test]
#[should_panic(expected = "InvalidAction")]
fn an_unauthorized_invocation_is_rejected() {
    // No `mock_all_auths` here: the minter holds the role but never signs, so
    // the `require_auth` inside `require_role` rejects the call.
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let id = env.register(AccessControlContract, (&admin,));
    let client = AccessControlContractClient::new(&env, &id);
    let alice = Address::generate(&env);
    let bob = Address::generate(&env);
    client.grant_role(&admin, &MINTER_ROLE, &alice);

    env.set_auths(&[]);
    client.mint(&alice, &bob, &1000);
}
