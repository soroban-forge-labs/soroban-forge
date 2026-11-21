#![no_std]

//! DAO-style governance: members open proposals and vote on them; a proposal
//! executes once its voting period ends with quorum and a majority in favour.
//!
//! **Voting power.** Every registered member has exactly one vote per
//! proposal. Callers cannot choose their weight: `cast_vote` takes no weight
//! argument and always counts `VOTE_WEIGHT` (1), so a single voter can never
//! outvote everyone else, and negative or inflated votes are impossible by
//! construction.
//!
//! Members are registered by the admin. Eligibility is snapshotted per
//! proposal: only members registered at or before the ledger a proposal was
//! created in may vote on it, so adding members mid-vote cannot swing an open
//! proposal. Removing a member revokes their ability to vote from then on.
//!
//! To weight votes by token holdings instead, replace `voting_power` with a
//! balance read through `token::Client` — and snapshot balances when the
//! proposal is created, or the same tokens can be moved between accounts and
//! voted with repeatedly.

use soroban_sdk::{contracterror, contract, contractimpl, contracttype, Address, Env, Symbol, Val, Vec, panic_with_error};

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum Error {
    AlreadyInitialized = 1,
    QuorumMustBePositive = 2,
    VotingPeriodMustBePositive = 3,
    AlreadyAMember = 4,
    NotAMember = 5,
    OnlyMembersCanCreateProposals = 6,
    AlreadyVotedOnThisProposal = 7,
    VotingPeriodHasEnded = 8,
    NotEligibleToVoteOnThisProposal = 9,
    VotingPeriodHasNotEndedYet = 10,
    ProposalAlreadyExecuted = 11,
    QuorumNotReached = 12,
    ProposalDidNotPass = 13,
}


/// Votes counted for each eligible voter.
pub const VOTE_WEIGHT: i128 = 1;

pub const DAY_IN_LEDGERS: u32 = 17_280;
pub const PERSISTENT_TTL: u32 = 30 * DAY_IN_LEDGERS;
pub const PERSISTENT_TTL_THRESHOLD: u32 = PERSISTENT_TTL - DAY_IN_LEDGERS;

/// A governance proposal.
#[contracttype]
#[derive(Clone)]
pub struct Proposal {
    pub id: u64,
    pub proposer: Address,
    /// Contract call dispatched when the proposal passes.
    pub target: Address,
    pub function_name: Symbol,
    pub args: Vec<Val>,
    /// Ledger sequence the proposal was created in; only members registered
    /// at or before it may vote (membership snapshot).
    pub created_ledger: u32,
    /// Absolute ledger sequence at which voting closes.
    pub voting_deadline: u32,
    /// Number of members who voted in favour (one vote each).
    pub votes_for: i128,
    /// Number of members who voted against.
    pub votes_against: i128,
    /// Whether the proposal has been executed.
    pub executed: bool,
}

#[contracttype]
pub enum DataKey {
    Proposal(u64),
    /// Set of (proposal_id, voter) pairs that have already voted.
    Voted(u64, Address),
    NextId,
    /// Minimum quorum (total votes required to execute).
    Quorum,
    /// Duration in ledgers for the voting period.
    VotingPeriod,
    Admin,
    /// Ledger sequence at which an address became a voting member.
    MemberSince(Address),
    /// Number of registered members.
    MemberCount,
}

#[contract]
pub struct GovernanceContract;

fn admin(env: &Env) -> Address {
    env.storage()
        .instance()
        .get(&DataKey::Admin)
        .expect("not initialized")
}

/// Voting power of `voter` on `proposal`: `VOTE_WEIGHT` for a member who was
/// registered when the proposal was created, otherwise none. Never derived
/// from anything the voter passes in.
fn voting_power(env: &Env, voter: &Address, proposal: &Proposal) -> i128 {
    match env
        .storage()
        .persistent()
        .get::<_, u32>(&DataKey::MemberSince(voter.clone()))
    {
        Some(since) if since <= proposal.created_ledger => VOTE_WEIGHT,
        _ => 0,
    }
}

#[contractimpl]
impl GovernanceContract {
    /// Initialize the contract.
    ///
    /// * `admin`         — address allowed to manage members
    /// * `quorum`        — minimum total votes (for + against) required to execute;
    ///   with one vote per member this is a number of members
    /// * `voting_period` — voting window in ledgers (~5 s per ledger on mainnet)
    pub fn __constructor(env: Env, admin: Address, quorum: i128, voting_period: u32) {
        if quorum <= 0 {
            panic_with_error!(&env, Error::QuorumMustBePositive);
        }
        if voting_period == 0 {
            panic_with_error!(&env, Error::VotingPeriodMustBePositive);
        }
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::Quorum, &quorum);
        env.storage()
            .instance()
            .set(&DataKey::VotingPeriod, &voting_period);
        env.storage().instance().set(&DataKey::NextId, &0u64);
        env.storage().instance().set(&DataKey::MemberCount, &0u32);
    }

    /// Register `member` as a voter (admin only). Members may vote on
    /// proposals created from this ledger onwards.
    pub fn add_member(env: Env, member: Address) {
        admin(&env).require_auth();
        let key = DataKey::MemberSince(member);
        if env.storage().persistent().has(&key) {
            panic_with_error!(&env, Error::AlreadyAMember);
        }
        env.storage()
            .persistent()
            .set(&key, &env.ledger().sequence());
        env.storage()
            .persistent()
            .extend_ttl(&key, PERSISTENT_TTL_THRESHOLD, PERSISTENT_TTL);
        let count: u32 = Self::member_count(env.clone());
        env.storage()
            .instance()
            .set(&DataKey::MemberCount, &(count + 1));
    }

    /// Remove `member` (admin only). They can no longer vote; votes they
    /// already cast stay counted.
    pub fn remove_member(env: Env, member: Address) {
        admin(&env).require_auth();
        let key = DataKey::MemberSince(member);
        if !env.storage().persistent().has(&key) {
            panic_with_error!(&env, Error::NotAMember);
        }
        env.storage().persistent().remove(&key);
        let count: u32 = Self::member_count(env.clone());
        env.storage()
            .instance()
            .set(&DataKey::MemberCount, &(count - 1));
    }

    /// Whether `address` is currently a registered member.
    pub fn is_member(env: Env, address: Address) -> bool {
        env.storage()
            .persistent()
            .has(&DataKey::MemberSince(address))
    }

    /// Number of registered members.
    pub fn member_count(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&DataKey::MemberCount)
            .unwrap_or(0)
    }

    /// Create a new governance proposal (members only).
    ///
    /// Returns the proposal id.
    pub fn create_proposal(
        env: Env,
        proposer: Address,
        target: Address,
        function_name: Symbol,
        args: Vec<Val>,
    ) -> u64 {
        proposer.require_auth();
        if !Self::is_member(env.clone(), proposer.clone()) {
            panic_with_error!(&env, Error::OnlyMembersCanCreateProposals);
        }

        let id: u64 = env.storage().instance().get(&DataKey::NextId).unwrap_or(0);
        let voting_period: u32 = env
            .storage()
            .instance()
            .get(&DataKey::VotingPeriod)
            .unwrap_or(1000);
        let created_ledger = env.ledger().sequence();
        let deadline = created_ledger + voting_period;

        let proposal = Proposal {
            id,
            proposer,
            target,
            function_name,
            args,
            created_ledger,
            voting_deadline: deadline,
            votes_for: 0,
            votes_against: 0,
            executed: false,
        };
        let key = DataKey::Proposal(id);
        env.storage().persistent().set(&key, &proposal);
        env.storage()
            .persistent()
            .extend_ttl(&key, PERSISTENT_TTL_THRESHOLD, PERSISTENT_TTL);
        env.storage().instance().set(&DataKey::NextId, &(id + 1));
        id
    }

    /// Cast a vote on a proposal.
    ///
    /// * `in_favour` — `true` to vote for, `false` to vote against
    ///
    /// The vote counts exactly `VOTE_WEIGHT` (1). Voting power comes from
    /// membership at the time the proposal was created, never from the
    /// caller, so it cannot be inflated or negative.
    pub fn cast_vote(env: Env, voter: Address, proposal_id: u64, in_favour: bool) {
        voter.require_auth();

        // Prevent double-voting.
        let voted_key = DataKey::Voted(proposal_id, voter.clone());
        if env.storage().persistent().has(&voted_key) {
            panic_with_error!(&env, Error::AlreadyVotedOnThisProposal);
        }

        let prop_key = DataKey::Proposal(proposal_id);
        let mut proposal: Proposal = env
            .storage()
            .persistent()
            .get(&prop_key)
            .expect("proposal not found");

        if env.ledger().sequence() > proposal.voting_deadline {
            panic_with_error!(&env, Error::VotingPeriodHasEnded);
        }

        let power = voting_power(&env, &voter, &proposal);
        if power <= 0 {
            panic_with_error!(&env, Error::NotEligibleToVoteOnThisProposal);
        }

        if in_favour {
            proposal.votes_for += power;
        } else {
            proposal.votes_against += power;
        }

        env.storage().persistent().set(&prop_key, &proposal);
        env.storage()
            .persistent()
            .extend_ttl(&prop_key, PERSISTENT_TTL_THRESHOLD, PERSISTENT_TTL);
        env.storage().persistent().set(&voted_key, &true);
        env.storage()
            .persistent()
            .extend_ttl(&voted_key, PERSISTENT_TTL_THRESHOLD, PERSISTENT_TTL);
    }

    /// Execute a proposal after its voting period has ended.
    ///
    /// Succeeds only when:
    /// 1. The voting deadline has passed.
    /// 2. Total votes meet or exceed the configured quorum.
    /// 3. Votes in favour exceed votes against.
    pub fn execute_proposal(env: Env, proposal_id: u64) {
        let prop_key = DataKey::Proposal(proposal_id);
        let mut proposal: Proposal = env
            .storage()
            .persistent()
            .get(&prop_key)
            .expect("proposal not found");

        if env.ledger().sequence() <= proposal.voting_deadline {
            panic_with_error!(&env, Error::VotingPeriodHasNotEndedYet);
        }
        if proposal.executed {
            panic_with_error!(&env, Error::ProposalAlreadyExecuted);
        }

        let quorum: i128 = env.storage().instance().get(&DataKey::Quorum).unwrap_or(1);
        let total = proposal.votes_for + proposal.votes_against;
        if total < quorum {
            panic_with_error!(&env, Error::QuorumNotReached);
        }
        if proposal.votes_for <= proposal.votes_against {
            panic_with_error!(&env, Error::ProposalDidNotPass);
        }

        proposal.executed = true;
        env.storage().persistent().set(&prop_key, &proposal);
        env.storage()
            .persistent()
            .extend_ttl(&prop_key, PERSISTENT_TTL_THRESHOLD, PERSISTENT_TTL);

        env.invoke_contract::<Val>(&proposal.target, &proposal.function_name, proposal.args);
    }

    /// Return the current state of a proposal.
    pub fn get_proposal(env: Env, proposal_id: u64) -> Proposal {
        env.storage()
            .persistent()
            .get(&DataKey::Proposal(proposal_id))
            .expect("proposal not found")
    }
}

#[cfg(test)]
mod test;
