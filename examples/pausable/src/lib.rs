//! A minimal circuit breaker: the admin fixed at deploy time can `pause` and
//! `unpause` the contract, and every guarded entrypoint refuses to run while
//! it is paused.
//!
//! `increment` and `reset` stand in for whatever your contract actually does —
//! they call [`require_not_paused`] first, which is the whole pattern. Read-only
//! entrypoints (`count`, `is_paused`, `admin`) are deliberately left unguarded
//! so state stays inspectable during an incident.

#![no_std]

use soroban_sdk::{
    contract, contracterror, contractevent, contractimpl, contracttype, Address, Env,
};

/// 1 day in ledgers, assuming 5 seconds per ledger.
pub const DAY_IN_LEDGERS: u32 = 17_280;
/// Instance storage TTL is extended to 30 days.
pub const INSTANCE_TTL: u32 = 30 * DAY_IN_LEDGERS;
/// Threshold at which instance storage TTL is extended (29 days).
pub const INSTANCE_TTL_THRESHOLD: u32 = INSTANCE_TTL - DAY_IN_LEDGERS;

fn extend_instance_ttl(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(INSTANCE_TTL_THRESHOLD, INSTANCE_TTL);
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum Error {
    /// A guarded entrypoint was called while the contract is paused.
    Paused = 1,
    /// `pause` was called on an already-paused contract.
    AlreadyPaused = 2,
    /// `unpause` was called on a contract that is not paused.
    NotPaused = 3,
    /// `increment` would push the counter past `u32::MAX`.
    Overflow = 4,
    /// `accept_admin` or `cancel_admin_proposal` called with no proposal pending.
    NoPendingAdmin = 5,
}

/// Emitted when the breaker is opened.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Paused {
    #[topic]
    pub admin: Address,
}

/// Emitted when the breaker is closed again.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Unpaused {
    #[topic]
    pub admin: Address,
}

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    Admin,
    PendingAdmin,
    Paused,
    Count,
}

fn read_admin(env: &Env) -> Address {
    env.storage().instance().get(&DataKey::Admin).unwrap()
}

fn read_paused(env: &Env) -> bool {
    env.storage()
        .instance()
        .get(&DataKey::Paused)
        .unwrap_or(false)
}

/// Reject the call when the breaker is open.
///
/// Call this with `?` at the top of every entrypoint that must stop during an
/// incident.
fn require_not_paused(env: &Env) -> Result<(), Error> {
    if read_paused(env) {
        Err(Error::Paused)
    } else {
        Ok(())
    }
}

/// Emitted when the admin proposes a successor.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdminProposed {
    #[topic]
    pub admin: Address,
    pub new_admin: Address,
}

/// Emitted when the pending admin accepts and becomes admin.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdminAccepted {
    #[topic]
    pub old_admin: Address,
    pub new_admin: Address,
}

/// Emitted when the admin cancels a pending proposal.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdminProposalCancelled {
    #[topic]
    pub admin: Address,
    pub pending: Address,
}

#[contract]
pub struct PausableContract;

#[contractimpl]
impl PausableContract {
    /// Deploy-time setup: `admin` is the only address that may flip the
    /// breaker, and the contract starts unpaused.
    pub fn __constructor(env: Env, admin: Address) {
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::Paused, &false);
        extend_instance_ttl(&env);
    }

    /// Return the admin address.
    pub fn admin(env: Env) -> Address {
        read_admin(&env)
    }

    /// Return the pending admin, if a handover has been proposed.
    pub fn pending_admin(env: Env) -> Option<Address> {
        env.storage().instance().get(&DataKey::PendingAdmin)
    }

    /// Step one of a handover: the current admin nominates `new_admin`.
    pub fn propose_admin(env: Env, new_admin: Address) {
        let admin: Address = env.storage().instance().get(&DataKey::Admin).unwrap();
        admin.require_auth();
        env.storage()
            .instance()
            .set(&DataKey::PendingAdmin, &new_admin);
        AdminProposed { admin, new_admin }.publish(&env);
    }

    /// Step two: the pending admin accepts and becomes the admin.
    pub fn accept_admin(env: Env) -> Result<(), Error> {
        let pending: Address = env
            .storage()
            .instance()
            .get(&DataKey::PendingAdmin)
            .ok_or(Error::NoPendingAdmin)?;
        pending.require_auth();
        let old_admin: Address = env.storage().instance().get(&DataKey::Admin).unwrap();
        env.storage().instance().set(&DataKey::Admin, &pending);
        env.storage().instance().remove(&DataKey::PendingAdmin);
        AdminAccepted {
            old_admin,
            new_admin: pending,
        }
        .publish(&env);
        Ok(())
    }

    /// The current admin withdraws a pending proposal.
    pub fn cancel_admin_proposal(env: Env) -> Result<(), Error> {
        let admin: Address = env.storage().instance().get(&DataKey::Admin).unwrap();
        admin.require_auth();
        let pending: Address = env
            .storage()
            .instance()
            .get(&DataKey::PendingAdmin)
            .ok_or(Error::NoPendingAdmin)?;
        env.storage().instance().remove(&DataKey::PendingAdmin);
        AdminProposalCancelled { admin, pending }.publish(&env);
        Ok(())
    }

    /// Whether guarded entrypoints are currently rejecting calls.
    pub fn is_paused(env: Env) -> bool {
        read_paused(&env)
    }

    /// Open the breaker. Requires admin authorization.
    pub fn pause(env: Env) -> Result<(), Error> {
        let admin = read_admin(&env);
        admin.require_auth();
        if read_paused(&env) {
            return Err(Error::AlreadyPaused);
        }
        env.storage().instance().set(&DataKey::Paused, &true);
        extend_instance_ttl(&env);
        Paused { admin }.publish(&env);
        Ok(())
    }

    /// Close the breaker again. Requires admin authorization.
    pub fn unpause(env: Env) -> Result<(), Error> {
        let admin = read_admin(&env);
        admin.require_auth();
        if !read_paused(&env) {
            return Err(Error::NotPaused);
        }
        env.storage().instance().set(&DataKey::Paused, &false);
        extend_instance_ttl(&env);
        Unpaused { admin }.publish(&env);
        Ok(())
    }

    /// Guarded example entrypoint: add `by` to the counter and return the new
    /// value. Rejected with [`Error::Paused`] while paused.
    pub fn increment(env: Env, by: u32) -> Result<u32, Error> {
        require_not_paused(&env)?;
        let next = Self::count(env.clone())
            .checked_add(by)
            .ok_or(Error::Overflow)?;
        env.storage().instance().set(&DataKey::Count, &next);
        extend_instance_ttl(&env);
        Ok(next)
    }

    /// Guarded admin entrypoint: put the counter back to zero. Being
    /// admin-only does not exempt it from the breaker — the pause check comes
    /// first, so a paused contract stays frozen even for the admin.
    pub fn reset(env: Env) -> Result<(), Error> {
        require_not_paused(&env)?;
        read_admin(&env).require_auth();
        env.storage().instance().set(&DataKey::Count, &0u32);
        extend_instance_ttl(&env);
        Ok(())
    }

    /// Read the counter. Unguarded: reads keep working while paused.
    pub fn count(env: Env) -> u32 {
        env.storage().instance().get(&DataKey::Count).unwrap_or(0)
    }
}

#[cfg(test)]
mod test;
