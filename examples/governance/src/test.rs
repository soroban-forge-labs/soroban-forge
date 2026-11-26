#![cfg(test)]

extern crate std;

use super::*;
use soroban_sdk::{
    contract, contractimpl, symbol_short, vec,
    testutils::{storage::Instance as _, Address as _, Events, Ledger},
    Address, Env, IntoVal, Symbol,
};

#[contract]
pub struct MockTargetContract;

#[contractimpl]
impl MockTargetContract {
    pub fn record(env: Env, value: u32) {
        let calls: u32 = env
            .storage()
            .instance()
            .get(&symbol_short!("calls"))
            .unwrap_or(0);
        env.storage()
            .instance()
            .set(&symbol_short!("calls"), &(calls + 1));
        env.storage().instance().set(&symbol_short!("value"), &value);
    }

    pub fn get_calls(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&symbol_short!("calls"))
            .unwrap_or(0)
    }

    pub fn get_value(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&symbol_short!("value"))
            .unwrap_or(0)
    }
}

struct Dao<'a> {
    env: Env,
    client: GovernanceContractClient<'a>,
    target_id: Address,
    target: MockTargetContractClient<'a>,
}

/// A DAO with quorum 3, a 100-ledger voting period and `members` registered.
fn setup<'a>(members: usize) -> (Dao<'a>, std::vec::Vec<Address>) {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(GovernanceContract, ());
    let client = GovernanceContractClient::new(&env, &contract_id);
    let target_id = env.register(MockTargetContract, ());
    let target = MockTargetContractClient::new(&env, &target_id);
    let admin = Address::generate(&env);
    client.initialize(&admin, &3, &100);

    let mut addrs = std::vec::Vec::new();
    for _ in 0..members {
        let member = Address::generate(&env);
        client.add_member(&member);
        addrs.push(member);
    }
    (Dao {
        env,
        client,
        target_id,
        target,
    }, addrs)
}

fn create_proposal(dao: &Dao, proposer: &Address) -> u64 {
    dao.client.create_proposal(
        proposer,
        &dao.target_id,
        &Symbol::new(&dao.env, "record"),
        &vec![&dao.env, 42_u32.into_val(&dao.env)],
    )
}

fn end_voting(env: &Env) {
    env.ledger().with_mut(|li| li.sequence_number += 101);
}

#[test]
fn create_and_vote_and_execute() {
    let (dao, m) = setup(4);
    let pid = create_proposal(&dao, &m[0]);
    dao.client.cast_vote(&m[0], &pid, &true);
    dao.client.cast_vote(&m[1], &pid, &true);
    dao.client.cast_vote(&m[2], &pid, &false);

    end_voting(&dao.env);
    dao.client.execute_proposal(&pid);

    let proposal = dao.client.get_proposal(&pid);
    assert!(proposal.executed);
    assert_eq!(proposal.votes_for, 2);
    assert_eq!(proposal.votes_against, 1);
    assert_eq!(dao.target.get_calls(), 1);
    assert_eq!(dao.target.get_value(), 42);
}

#[test]
fn passed_proposal_invokes_target_once() {
    let (dao, members) = setup(3);
    let proposal_id = create_proposal(&dao, &members[0]);
    for member in &members {
        dao.client.cast_vote(member, &proposal_id, &true);
    }

    end_voting(&dao.env);
    dao.client.execute_proposal(&proposal_id);

    assert_eq!(dao.target.get_calls(), 1);
    assert_eq!(dao.target.get_value(), 42);
    assert!(dao.client.try_execute_proposal(&proposal_id).is_err());
    assert_eq!(dao.target.get_calls(), 1);
}

#[test]
fn every_vote_counts_exactly_one() {
    let (dao, m) = setup(3);
    let pid = create_proposal(&dao, &m[0]);
    for member in &m {
        dao.client.cast_vote(member, &pid, &true);
    }
    assert_eq!(dao.client.get_proposal(&pid).votes_for, 3 * VOTE_WEIGHT);
}

#[test]
fn a_single_voter_cannot_outvote_everyone_else() {
    // The attacker is a member and votes first; four others vote against.
    let (dao, m) = setup(5);
    let attacker = &m[0];
    let pid = create_proposal(&dao, attacker);

    dao.client.cast_vote(attacker, &pid, &true);
    for member in &m[1..] {
        dao.client.cast_vote(member, &pid, &false);
    }

    // A second vote (the only other lever) is refused...
    assert!(dao.client.try_cast_vote(attacker, &pid, &true).is_err());

    let proposal = dao.client.get_proposal(&pid);
    assert_eq!(proposal.votes_for, 1);
    assert_eq!(proposal.votes_against, 4);

    // ...so the proposal fails, as the majority decided.
    end_voting(&dao.env);
    assert!(dao.client.try_execute_proposal(&pid).is_err());
    assert!(!dao.client.get_proposal(&pid).executed);
}

#[test]
fn a_lone_voter_cannot_pass_a_proposal_below_quorum() {
    // Previously one caller could pass `weight = i128::MAX`; now a single
    // vote is always 1, which can never satisfy a quorum of 3.
    let (dao, m) = setup(5);
    let pid = create_proposal(&dao, &m[0]);
    dao.client.cast_vote(&m[0], &pid, &true);

    end_voting(&dao.env);
    let result = dao.client.try_execute_proposal(&pid);
    assert!(result.is_err(), "quorum of 3 cannot be met by one vote");
}

#[test]
#[should_panic(expected = "not eligible to vote")]
fn non_members_cannot_vote() {
    let (dao, m) = setup(2);
    let pid = create_proposal(&dao, &m[0]);
    let outsider = Address::generate(&dao.env);
    dao.client.cast_vote(&outsider, &pid, &true);
}

#[test]
fn many_fresh_addresses_cannot_flood_a_vote() {
    // Sybil attempt: generating new addresses gives them no voting power.
    let (dao, m) = setup(3);
    let pid = create_proposal(&dao, &m[0]);
    for _ in 0..10 {
        let sybil = Address::generate(&dao.env);
        assert!(dao.client.try_cast_vote(&sybil, &pid, &true).is_err());
    }
    assert_eq!(dao.client.get_proposal(&pid).votes_for, 0);
}

#[test]
fn members_added_after_a_proposal_cannot_vote_on_it() {
    // Membership is snapshotted at proposal creation, so the admin cannot
    // add friendly voters to swing an open proposal.
    let (dao, m) = setup(3);
    let pid = create_proposal(&dao, &m[0]);

    dao.env.ledger().with_mut(|li| li.sequence_number += 1);
    let late = Address::generate(&dao.env);
    dao.client.add_member(&late);

    assert!(dao.client.try_cast_vote(&late, &pid, &true).is_err());

    // They can vote on proposals created after they joined.
    let next = create_proposal(&dao, &m[0]);
    dao.client.cast_vote(&late, &next, &true);
    assert_eq!(dao.client.get_proposal(&next).votes_for, 1);
}

#[test]
fn removed_members_cannot_vote() {
    let (dao, m) = setup(3);
    let pid = create_proposal(&dao, &m[0]);
    dao.client.remove_member(&m[2]);

    assert!(!dao.client.is_member(&m[2]));
    assert_eq!(dao.client.member_count(), 2);
    assert!(dao.client.try_cast_vote(&m[2], &pid, &false).is_err());
}

#[test]
#[should_panic(expected = "only members can create proposals")]
fn non_members_cannot_create_proposals() {
    let (dao, _m) = setup(1);
    let outsider = Address::generate(&dao.env);
    create_proposal(&dao, &outsider);
}

#[test]
#[should_panic(expected = "already voted")]
fn double_vote_is_rejected() {
    let (dao, m) = setup(3);
    let pid = create_proposal(&dao, &m[0]);
    dao.client.cast_vote(&m[1], &pid, &true);
    dao.client.cast_vote(&m[1], &pid, &true); // should panic
}

#[test]
#[should_panic(expected = "quorum not reached")]
fn execute_fails_below_quorum() {
    let (dao, m) = setup(3);
    let pid = create_proposal(&dao, &m[0]);
    dao.client.cast_vote(&m[0], &pid, &true);
    dao.client.cast_vote(&m[1], &pid, &true); // quorum is 3

    end_voting(&dao.env);
    dao.client.execute_proposal(&pid);
}

#[test]
#[should_panic(expected = "quorum must be positive")]
fn zero_quorum_is_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    let client = GovernanceContractClient::new(&env, &env.register(GovernanceContract, ()));
    client.initialize(&Address::generate(&env), &0, &100);
}

#[test]
#[should_panic(expected = "quorum must be positive")]
fn negative_quorum_is_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    let client = GovernanceContractClient::new(&env, &env.register(GovernanceContract, ()));
    client.initialize(&Address::generate(&env), &-1, &100);
}

#[test]
#[should_panic(expected = "voting period must be positive")]
fn zero_voting_period_is_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    let client = GovernanceContractClient::new(&env, &env.register(GovernanceContract, ()));
    client.initialize(&Address::generate(&env), &3, &0);
}

#[test]
fn instance_storage_outlives_the_default_ttl() {
    let (dao, _) = setup(0);

    // Well past the default instance TTL, but within the extended one.
    dao.env.ledger().with_mut(|li| li.sequence_number += 100_000);

    let member = Address::generate(&dao.env);
    dao.client.add_member(&member);
    assert_eq!(dao.client.member_count(), 1);

    let ttl = dao
        .env
        .as_contract(&dao.client.address, || dao.env.storage().instance().get_ttl());
    assert!(ttl >= INSTANCE_TTL_THRESHOLD, "instance TTL was not extended: {}", ttl);
}

#[test]
fn admin_handover_propose_then_accept() {
    let (dao, _) = setup(0);
    let old = dao.client.get_admin();
    let new_admin = Address::generate(&dao.env);

    dao.client.propose_admin(&new_admin);
    assert_eq!(dao.env.events().all().len(), 1);
    assert_eq!(dao.client.get_pending_admin(), Some(new_admin.clone()));
    // Proposing alone does not transfer anything.
    assert_eq!(dao.client.get_admin(), old);

    dao.client.accept_admin();
    assert_eq!(dao.client.get_admin(), new_admin);
    assert_eq!(dao.client.get_pending_admin(), None);
    // Accepting published an event.
    assert_eq!(dao.env.events().all().len(), 1);
}

#[test]
fn admin_handover_cancel() {
    let (dao, _) = setup(0);
    let old = dao.client.get_admin();
    let new_admin = Address::generate(&dao.env);

    dao.client.propose_admin(&new_admin);
    dao.client.cancel_admin_proposal();
    assert_eq!(dao.client.get_pending_admin(), None);
    assert_eq!(dao.client.get_admin(), old);
    assert_eq!(
        dao.client.try_accept_admin().unwrap_err().unwrap(),
        Error::NoPendingAdmin
    );
}

#[test]
fn admin_handover_without_proposal_fails() {
    let (dao, _) = setup(0);
    assert_eq!(
        dao.client.try_accept_admin().unwrap_err().unwrap(),
        Error::NoPendingAdmin
    );
    assert_eq!(
        dao.client.try_cancel_admin_proposal().unwrap_err().unwrap(),
        Error::NoPendingAdmin
    );
}

#[test]
fn non_pending_address_cannot_accept_admin() {
    use soroban_sdk::testutils::{MockAuth, MockAuthInvoke};
    let (dao, _) = setup(0);
    let old = dao.client.get_admin();
    let new_admin = Address::generate(&dao.env);
    let attacker = Address::generate(&dao.env);
    dao.client.propose_admin(&new_admin);

    // Only `attacker` authorizes the call; the contract requires the
    // pending admin's auth, so it must be rejected.
    dao.env.mock_auths(&[MockAuth {
        address: &attacker,
        invoke: &MockAuthInvoke {
            contract: &dao.client.address,
            fn_name: "accept_admin",
            args: ().into_val(&dao.env),
            sub_invokes: &[],
        },
    }]);
    assert!(dao.client.try_accept_admin().is_err());
    assert_eq!(dao.client.get_admin(), old);
    assert_eq!(dao.client.get_pending_admin(), Some(new_admin));
}

#[test]
fn only_admin_can_propose_or_cancel() {
    use soroban_sdk::testutils::{MockAuth, MockAuthInvoke};
    let (dao, _) = setup(0);
    let outsider = Address::generate(&dao.env);
    let new_admin = Address::generate(&dao.env);
    dao.env.mock_auths(&[MockAuth {
        address: &outsider,
        invoke: &MockAuthInvoke {
            contract: &dao.client.address,
            fn_name: "propose_admin",
            args: (&new_admin,).into_val(&dao.env),
            sub_invokes: &[],
        },
    }]);
    assert!(dao.client.try_propose_admin(&new_admin).is_err());
    assert_eq!(dao.client.get_pending_admin(), None);
}
