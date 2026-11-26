extern crate std;

use super::*;
use soroban_sdk::testutils::{Address as _, Events as _};
use soroban_sdk::{testutils::Ledger, Address, Bytes, BytesN, Env, IntoVal};

fn commit(env: &Env, entrant: &Address, nonce: &BytesN<32>) -> BytesN<32> {
    let mut buf = Bytes::new(env);
    buf.append(&entrant.to_xdr(env));
    let bytes: Bytes = nonce.clone().into();
    buf.append(&bytes);
    env.crypto().sha256(&buf).into()
}

fn setup(env: &Env) -> (LotteryContractClient<'_>, Address, u64, u64) {
    env.mock_all_auths();

    let start_ts = 1_000_000;
    env.ledger().with_mut(|li| li.timestamp = start_ts);

    let admin = Address::generate(env);
    let commit_deadline = start_ts + 1000;
    let reveal_deadline = start_ts + 2000;

    let contract_id = env.register(LotteryContract, ());
    let contract = LotteryContractClient::new(env, &contract_id);
    contract.initialize(&admin, &commit_deadline, &reveal_deadline);

    (contract, admin, commit_deadline, reveal_deadline)
}

#[test]
fn entry_reveal_and_draw_picks_a_revealed_winner() {
    let env = Env::default();
    let (contract, _admin, commit_deadline, reveal_deadline) = setup(&env);

    let alice = Address::generate(&env);
    let bob = Address::generate(&env);
    let carol = Address::generate(&env);

    let nonce_a = BytesN::from_array(&env, &[1_u8; 32]);
    let nonce_b = BytesN::from_array(&env, &[2_u8; 32]);
    let nonce_c = BytesN::from_array(&env, &[3_u8; 32]);

    contract.enter(&alice, &commit(&env, &alice, &nonce_a));
    contract.enter(&bob, &commit(&env, &bob, &nonce_b));
    contract.enter(&carol, &commit(&env, &carol, &nonce_c));

    // Move into the reveal window.
    env.ledger().with_mut(|li| li.timestamp = commit_deadline);
    contract.reveal(&alice, &nonce_a);
    contract.reveal(&bob, &nonce_b);
    contract.reveal(&carol, &nonce_c);

    // Move past the reveal deadline and draw.
    env.ledger().with_mut(|li| li.timestamp = reveal_deadline);
    let winner = contract.draw();

    assert!(winner == alice || winner == bob || winner == carol);
    assert_eq!(contract.get_winner(), winner);
    // 3 enter + 3 reveal + 1 draw events published.
    assert_eq!(env.events().all().len(), 7);
}

#[test]
fn non_revealed_entrants_are_excluded_from_the_draw() {
    let env = Env::default();
    let (contract, _admin, commit_deadline, reveal_deadline) = setup(&env);

    let alice = Address::generate(&env);
    let bob = Address::generate(&env);

    let nonce_a = BytesN::from_array(&env, &[1_u8; 32]);
    let nonce_b = BytesN::from_array(&env, &[2_u8; 32]);

    contract.enter(&alice, &commit(&env, &alice, &nonce_a));
    contract.enter(&bob, &commit(&env, &bob, &nonce_b));

    env.ledger().with_mut(|li| li.timestamp = commit_deadline);
    // Only alice reveals; bob forfeits.
    contract.reveal(&alice, &nonce_a);

    env.ledger().with_mut(|li| li.timestamp = reveal_deadline);
    let winner = contract.draw();

    assert_eq!(winner, alice);
}

#[test]
#[should_panic]
fn reveal_with_wrong_nonce_fails() {
    let env = Env::default();
    let (contract, _admin, commit_deadline, _reveal_deadline) = setup(&env);

    let alice = Address::generate(&env);
    let nonce_a = BytesN::from_array(&env, &[1_u8; 32]);
    let wrong_nonce = BytesN::from_array(&env, &[9_u8; 32]);

    contract.enter(&alice, &commit(&env, &alice, &nonce_a));

    env.ledger().with_mut(|li| li.timestamp = commit_deadline);
    contract.reveal(&alice, &wrong_nonce);
}

#[test]
#[should_panic]
fn duplicate_commitments_are_rejected_at_enter() {
    let env = Env::default();
    let (contract, _admin, _commit_deadline, _reveal_deadline) = setup(&env);

    let alice = Address::generate(&env);
    let bob = Address::generate(&env);
    let nonce = BytesN::from_array(&env, &[1_u8; 32]);
    let commitment = commit(&env, &alice, &nonce);

    contract.enter(&alice, &commitment);
    contract.enter(&bob, &commitment);
}

#[test]
#[should_panic]
fn copied_commitment_cannot_be_revealed_by_different_entrant() {
    let env = Env::default();
    let (contract, _admin, commit_deadline, _reveal_deadline) = setup(&env);

    let alice = Address::generate(&env);
    let bob = Address::generate(&env);
    let nonce = BytesN::from_array(&env, &[1_u8; 32]);

    let comm_alice = commit(&env, &alice, &nonce);
    let nonce_bob_fake = BytesN::from_array(&env, &[2_u8; 32]);
    let comm_bob = commit(&env, &bob, &nonce_bob_fake);

    contract.enter(&alice, &comm_alice);
    contract.enter(&bob, &comm_bob);

    env.ledger().with_mut(|li| li.timestamp = commit_deadline);
    // Bob attempts to reveal Alice's nonce for his own entry
    contract.reveal(&bob, &nonce);
}

#[test]
#[should_panic]
fn draw_without_reveals_fails() {
    let env = Env::default();
    let (contract, _admin, commit_deadline, reveal_deadline) = setup(&env);

    let alice = Address::generate(&env);
    let nonce_a = BytesN::from_array(&env, &[1_u8; 32]);
    contract.enter(&alice, &commit(&env, &alice, &nonce_a));

    let _ = commit_deadline;
    env.ledger().with_mut(|li| li.timestamp = reveal_deadline);
    contract.draw();
}

#[test]
#[should_panic]
fn enter_after_commit_deadline_fails() {
    let env = Env::default();
    let (contract, _admin, commit_deadline, _reveal_deadline) = setup(&env);

    let alice = Address::generate(&env);
    let nonce_a = BytesN::from_array(&env, &[1_u8; 32]);

    env.ledger().with_mut(|li| li.timestamp = commit_deadline);
    contract.enter(&alice, &commit(&env, &alice, &nonce_a));
}

#[test]
#[should_panic]
fn initialize_twice_fails_after_a_commit() {
    let env = Env::default();
    let (contract, _admin, commit_deadline, reveal_deadline) = setup(&env);

    let alice = Address::generate(&env);
    let nonce_a = BytesN::from_array(&env, &[1_u8; 32]);
    contract.enter(&alice, &commit(&env, &alice, &nonce_a));

    // An attacker tries to take over and reset the running lottery.
    let attacker = Address::generate(&env);
    contract.initialize(&attacker, &(commit_deadline + 10), &(reveal_deadline + 10));
}

#[test]
fn initialize_twice_fails_after_a_completed_draw_and_keeps_the_winner() {
    let env = Env::default();
    let (contract, _admin, commit_deadline, reveal_deadline) = setup(&env);

    let alice = Address::generate(&env);
    let nonce_a = BytesN::from_array(&env, &[1_u8; 32]);
    contract.enter(&alice, &commit(&env, &alice, &nonce_a));
    env.ledger().with_mut(|li| li.timestamp = commit_deadline);
    contract.reveal(&alice, &nonce_a);
    env.ledger().with_mut(|li| li.timestamp = reveal_deadline);
    let winner = contract.draw();

    let attacker = Address::generate(&env);
    let result = contract.try_initialize(
        &attacker,
        &(reveal_deadline + 100),
        &(reveal_deadline + 200),
    );
    assert!(
        result.is_err(),
        "a finished lottery must not be re-initializable"
    );

    // The draw outcome is untouched and cannot be re-run.
    assert_eq!(contract.get_winner(), winner);
    assert!(contract.try_draw().is_err());
}

#[test]
fn last_revealer_can_withhold_nonce_and_alter_draw_eligibility() {
    let env = Env::default();
    let (contract, _admin, commit_deadline, reveal_deadline) = setup(&env);

    let alice = Address::generate(&env);
    let bob = Address::generate(&env);
    let carol = Address::generate(&env);

    let nonce_a = BytesN::from_array(&env, &[1_u8; 32]);
    let nonce_b = BytesN::from_array(&env, &[2_u8; 32]);
    let nonce_c = BytesN::from_array(&env, &[3_u8; 32]);

    contract.enter(&alice, &commit(&env, &alice, &nonce_a));
    contract.enter(&bob, &commit(&env, &bob, &nonce_b));
    contract.enter(&carol, &commit(&env, &carol, &nonce_c));

    // Move to reveal phase. Alice and Bob reveal.
    env.ledger().with_mut(|li| li.timestamp = commit_deadline);
    contract.reveal(&alice, &nonce_a);
    contract.reveal(&bob, &nonce_b);

    // Carol is the last revealer. Carol can observe Alice and Bob's reveals.
    // Carol chooses NOT to reveal her nonce.
    // When the reveal phase ends, draw proceeds without Carol.
    env.ledger().with_mut(|li| li.timestamp = reveal_deadline);
    let winner = contract.draw();

    // The winner is selected only among entrants who revealed (Alice or Bob).
    // Carol's missing reveal is not penalized with a loss of deposit, but Carol is excluded.
    assert!(winner == alice || winner == bob);
    assert_ne!(winner, carol);
}

#[test]
fn storage_survives_past_default_ttl() {
    let env = Env::default();
    let (contract, _admin, commit_deadline, reveal_deadline) = setup(&env);

    let alice = Address::generate(&env);
    let bob = Address::generate(&env);
    let carol = Address::generate(&env);

    let nonce_a = BytesN::from_array(&env, &[1_u8; 32]);
    let nonce_b = BytesN::from_array(&env, &[2_u8; 32]);
    let nonce_c = BytesN::from_array(&env, &[3_u8; 32]);

    // Entrants commit their entries.
    contract.enter(&alice, &commit(&env, &alice, &nonce_a));
    contract.enter(&bob, &commit(&env, &bob, &nonce_b));
    contract.enter(&carol, &commit(&env, &carol, &nonce_c));

    // Move to reveal phase.
    env.ledger().with_mut(|li| li.timestamp = commit_deadline);
    contract.reveal(&alice, &nonce_a);
    contract.reveal(&bob, &nonce_b);
    contract.reveal(&carol, &nonce_c);

    // Advance the ledger well past the default TTL (30 days).
    // 30 days = 30 * 17_280 = 518_400 ledgers.
    // We advance 35 days to ensure we're beyond the threshold.
    env.ledger().with_mut(|li| {
        li.sequence_number += 35 * 17_280;
        li.timestamp = reveal_deadline + (35 * 17_280 * 5); // ~5 seconds per ledger
    });

    // After advancing past TTL, storage should still work because
    // extend_ttl was called on each operation.
    // We should be able to draw the winner successfully.
    let winner = contract.draw();
    assert!(winner == alice || winner == bob || winner == carol);

    // Verify we can still read the winner after the draw.
    assert_eq!(contract.get_winner(), winner);

    // Verify that trying to draw again still fails (state is preserved).
    assert!(contract.try_draw().is_err());
}

#[test]
fn propose_sets_pending_admin_and_publishes_event() {
    let env = Env::default();
    let (contract, admin, _, _) = setup(&env);
    let new_admin = Address::generate(&env);

    contract.propose_admin(&new_admin);

    assert_eq!(contract.pending_admin(), Some(new_admin.clone()));
    assert_eq!(contract.admin(), admin.clone());
    assert_eq!(env.events().all().len(), 1);
}

#[test]
fn accept_transfers_admin_clears_pending_and_publishes_event() {
    let env = Env::default();
    let (contract, admin, _, _) = setup(&env);
    let new_admin = Address::generate(&env);

    contract.propose_admin(&new_admin);
    contract.accept_admin();

    assert_eq!(contract.admin(), new_admin.clone());
    assert_eq!(contract.pending_admin(), None);
    assert_eq!(env.events().all().len(), 1);
}

#[test]
fn cancel_clears_pending_and_publishes_event() {
    let env = Env::default();
    let (contract, admin, _, _) = setup(&env);
    let new_admin = Address::generate(&env);

    contract.propose_admin(&new_admin);
    contract.cancel_admin_proposal();

    assert_eq!(contract.pending_admin(), None);
    assert_eq!(contract.admin(), admin.clone());
    assert_eq!(env.events().all().len(), 1);
    // The cancelled proposal can no longer be accepted.
    assert_eq!(
        contract.try_accept_admin().unwrap_err().unwrap(),
        Error::NoPendingAdmin
    );
}

#[test]
fn accept_without_proposal_fails() {
    let env = Env::default();
    let (contract, _, _, _) = setup(&env);
    assert_eq!(
        contract.try_accept_admin().unwrap_err().unwrap(),
        Error::NoPendingAdmin
    );
    assert_eq!(
        contract.try_cancel_admin_proposal().unwrap_err().unwrap(),
        Error::NoPendingAdmin
    );
}

#[test]
fn non_pending_address_cannot_accept() {
    let env = Env::default();
    let (contract, admin, _, _) = setup(&env);
    let new_admin = Address::generate(&env);
    contract.propose_admin(&new_admin);

    // Only a stranger (not the pending admin) authorizes this call.
    let stranger = Address::generate(&env);
    env.mock_auths(&[soroban_sdk::testutils::MockAuth {
        address: &stranger,
        invoke: &soroban_sdk::testutils::MockAuthInvoke {
            contract: &contract.address,
            fn_name: "accept_admin",
            args: ().into_val(&env),
            sub_invokes: &[],
        },
    }]);
    assert!(contract.try_accept_admin().is_err());
    assert_eq!(contract.admin(), admin);
    assert_eq!(contract.pending_admin(), Some(new_admin));
}

#[test]
fn only_current_admin_can_propose() {
    let env = Env::default();
    let (contract, admin, _, _) = setup(&env);
    let new_admin = Address::generate(&env);

    let stranger = Address::generate(&env);
    env.mock_auths(&[soroban_sdk::testutils::MockAuth {
        address: &stranger,
        invoke: &soroban_sdk::testutils::MockAuthInvoke {
            contract: &contract.address,
            fn_name: "propose_admin",
            args: (&new_admin,).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    assert!(contract.try_propose_admin(&new_admin).is_err());
    assert_eq!(contract.admin(), admin);
    assert_eq!(contract.pending_admin(), None);
}
