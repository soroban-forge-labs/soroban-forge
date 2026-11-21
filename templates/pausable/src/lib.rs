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

#[contract]
pub struct PausableContract;

#[contractimpl]
impl PausableContract {
    /// Deploy-time setup: `admin` is the only address that may flip the
    /// breaker, and the contract starts unpaused.
    pub fn __constructor(env: Env, admin: Address) {
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::Paused, &false);
    }

    /// Return the admin address.
    pub fn admin(env: Env) -> Address {
        read_admin(&env)
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
        Ok(next)
    }

    /// Guarded admin entrypoint: put the counter back to zero. Being
    /// admin-only does not exempt it from the breaker — the pause check comes
    /// first, so a paused contract stays frozen even for the admin.
    pub fn reset(env: Env) -> Result<(), Error> {
        require_not_paused(&env)?;
        read_admin(&env).require_auth();
        env.storage().instance().set(&DataKey::Count, &0u32);
        Ok(())
    }

    /// Read the counter. Unguarded: reads keep working while paused.
    pub fn count(env: Env) -> u32 {
        env.storage().instance().get(&DataKey::Count).unwrap_or(0)
    }
}

#[cfg(test)]
mod test;
