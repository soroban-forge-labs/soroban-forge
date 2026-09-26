//! A crowdfunding escrow contract with a funding target and deadline.
//!
//! Backers `pledge` a token until the deadline. Afterwards, either the owner
//! `claim`s the funds (target reached) or every backer gets a `refund`
//! (target missed). Funds are held by the contract itself in escrow.

#![no_std]

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, panic_with_error, symbol_short, token,
    Address, Env,
};

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum Error {
    InvalidAmount = 1,
    DeadlineInPast = 2,
    DeadlinePassed = 3,
    DeadlineNotReached = 4,
    TargetNotReached = 5,
    TargetReached = 6,
    AlreadyClaimed = 7,
    NothingPledged = 8,
}

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    Owner,
    Token,
    Target,
    Deadline,
    TotalPledged,
    Claimed,
    Pledge(Address),
}

fn owner(env: &Env) -> Address {
    env.storage().instance().get(&DataKey::Owner).unwrap()
}

fn token_address(env: &Env) -> Address {
    env.storage().instance().get(&DataKey::Token).unwrap()
}

fn target(env: &Env) -> i128 {
    env.storage().instance().get(&DataKey::Target).unwrap()
}

fn deadline(env: &Env) -> u64 {
    env.storage().instance().get(&DataKey::Deadline).unwrap()
}

fn total_pledged(env: &Env) -> i128 {
    env.storage()
        .instance()
        .get(&DataKey::TotalPledged)
        .unwrap_or(0)
}

fn pledge_of(env: &Env, backer: &Address) -> i128 {
    env.storage()
        .persistent()
        .get(&DataKey::Pledge(backer.clone()))
        .unwrap_or(0)
}

#[contract]
pub struct CrowdfundContract;

#[contractimpl]
impl CrowdfundContract {
    /// Deploy-time setup.
    ///
    /// * `owner` — receives the funds if the campaign succeeds
    /// * `token` — address of the token pledges are made in
    /// * `target` — funding goal (in the token's smallest unit)
    /// * `deadline` — unix timestamp after which pledging closes
    pub fn __constructor(env: Env, owner: Address, token: Address, target: i128, deadline: u64) {
        if target <= 0 {
            panic_with_error!(&env, Error::InvalidAmount);
        }
        if deadline <= env.ledger().timestamp() {
            panic_with_error!(&env, Error::DeadlineInPast);
        }
        env.storage().instance().set(&DataKey::Owner, &owner);
        env.storage().instance().set(&DataKey::Token, &token);
        env.storage().instance().set(&DataKey::Target, &target);
        env.storage().instance().set(&DataKey::Deadline, &deadline);
        env.storage().instance().set(&DataKey::TotalPledged, &0i128);
        env.storage().instance().set(&DataKey::Claimed, &false);
    }

    /// Pledge `amount` of the campaign token. Only possible before the
    /// deadline. The tokens are transferred into the contract's escrow.
    pub fn pledge(env: Env, from: Address, amount: i128) {
        from.require_auth();
        if amount <= 0 {
            panic_with_error!(&env, Error::InvalidAmount);
        }
        if env.ledger().timestamp() >= deadline(&env) {
            panic_with_error!(&env, Error::DeadlinePassed);
        }

        token::Client::new(&env, &token_address(&env)).transfer(
            &from,
            &env.current_contract_address(),
            &amount,
        );

        let new_pledge = pledge_of(&env, &from) + amount;
        env.storage()
            .persistent()
            .set(&DataKey::Pledge(from.clone()), &new_pledge);
        env.storage()
            .instance()
            .set(&DataKey::TotalPledged, &(total_pledged(&env) + amount));

        env.events()
            .publish((symbol_short!("pledge"), from), amount);
    }

    /// Owner claims the escrowed funds after a successful campaign.
    pub fn claim(env: Env) {
        let owner = owner(&env);
        owner.require_auth();
        if env.ledger().timestamp() < deadline(&env) {
            panic_with_error!(&env, Error::DeadlineNotReached);
        }
        let total = total_pledged(&env);
        if total < target(&env) {
            panic_with_error!(&env, Error::TargetNotReached);
        }
        let claimed: bool = env.storage().instance().get(&DataKey::Claimed).unwrap();
        if claimed {
            panic_with_error!(&env, Error::AlreadyClaimed);
        }
        env.storage().instance().set(&DataKey::Claimed, &true);

        token::Client::new(&env, &token_address(&env)).transfer(
            &env.current_contract_address(),
            &owner,
            &total,
        );
        env.events()
            .publish((symbol_short!("claim"), owner), total);
    }

    /// Refund `to`'s pledge after a failed campaign. Anyone may trigger a
    /// refund on a backer's behalf; funds always go back to the backer.
    pub fn refund(env: Env, to: Address) {
        if env.ledger().timestamp() < deadline(&env) {
            panic_with_error!(&env, Error::DeadlineNotReached);
        }
        if total_pledged(&env) >= target(&env) {
            panic_with_error!(&env, Error::TargetReached);
        }
        let amount = pledge_of(&env, &to);
        if amount == 0 {
            panic_with_error!(&env, Error::NothingPledged);
        }

        env.storage()
            .persistent()
            .remove(&DataKey::Pledge(to.clone()));
        env.storage()
            .instance()
            .set(&DataKey::TotalPledged, &(total_pledged(&env) - amount));

        token::Client::new(&env, &token_address(&env)).transfer(
            &env.current_contract_address(),
            &to,
            &amount,
        );
        env.events().publish((symbol_short!("refund"), to), amount);
    }

    // --- read-only getters ---

    pub fn get_pledge(env: Env, backer: Address) -> i128 {
        pledge_of(&env, &backer)
    }

    pub fn get_total_pledged(env: Env) -> i128 {
        total_pledged(&env)
    }

    pub fn get_target(env: Env) -> i128 {
        target(&env)
    }

    pub fn get_deadline(env: Env) -> u64 {
        deadline(&env)
    }

    pub fn get_token(env: Env) -> Address {
        token_address(&env)
    }

    pub fn get_owner(env: Env) -> Address {
        owner(&env)
    }
}

#[cfg(test)]
mod test;
