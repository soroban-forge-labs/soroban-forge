#![no_std]

use soroban_sdk::{contract, contractimpl, contracterror, contracttype, symbol_short, Address, Env, panic_with_error};

const SCALE: i128 = 1_000_000_000_000_000_000;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum Error {
    InsufficientStake = 1,
    NotAdmin = 2,
    NoRewards = 3,
}

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    Admin,
    Token,
    AccRewardPerShare,
    TotalStaked,
    Staked(Address),
    RewardDebt(Address),
}

fn admin(env: &Env) -> Address {
    env.storage().instance().get(&DataKey::Admin).unwrap()
}

fn token(env: &Env) -> Address {
    env.storage().instance().get(&DataKey::Token).unwrap()
}

fn acc_per_share(env: &Env) -> i128 {
    env.storage().instance().get(&DataKey::AccRewardPerShare).unwrap_or(0)
}

fn set_acc_per_share(env: &Env, val: i128) {
    env.storage().instance().set(&DataKey::AccRewardPerShare, &val);
}

fn total_staked(env: &Env) -> i128 {
    env.storage().instance().get(&DataKey::TotalStaked).unwrap_or(0)
}

fn set_total_staked(env: &Env, val: i128) {
    env.storage().instance().set(&DataKey::TotalStaked, &val);
}

fn read_staked(env: &Env, user: &Address) -> i128 {
    env.storage().persistent().get(&DataKey::Staked(user.clone())).unwrap_or(0)
}

fn write_staked(env: &Env, user: &Address, val: i128) {
    env.storage().persistent().set(&DataKey::Staked(user.clone()), &val);
}

fn reward_debt(env: &Env, user: &Address) -> i128 {
    env.storage().persistent().get(&DataKey::RewardDebt(user.clone())).unwrap_or(0)
}

fn set_reward_debt(env: &Env, user: &Address, val: i128) {
    env.storage().persistent().set(&DataKey::RewardDebt(user.clone()), &val);
}

fn pending(env: &Env, user: &Address) -> i128 {
    let s = read_staked(env, user);
    if s == 0 {
        return 0;
    }
    let acc = acc_per_share(env);
    let debt = reward_debt(env, user);
    let raw = s * acc / SCALE;
    if raw > debt { raw - debt } else { 0 }
}

fn _claim(env: &Env, user: &Address) -> i128 {
    let amt = pending(env, user);
    if amt > 0 {
        let s = read_staked(env, user);
        set_reward_debt(env, user, s * acc_per_share(env) / SCALE);
        let tok = token(env);
        soroban_sdk::token::Client::new(env, &tok).transfer(
            &env.current_contract_address(),
            user,
            &amt,
        );
    }
    amt
}

#[contract]
pub struct StakingContract;

#[contractimpl]
impl StakingContract {
    pub fn __constructor(env: Env, admin: Address, token: Address) {
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::Token, &token);
    }

    pub fn deposit(env: Env, user: Address, amount: i128) {
        user.require_auth();
        _claim(&env, &user);
        let new = read_staked(&env, &user) + amount;
        write_staked(&env, &user, new);
        set_reward_debt(&env, &user, new * acc_per_share(&env) / SCALE);
        set_total_staked(&env, total_staked(&env) + amount);
        env.events().publish((symbol_short!("deposit"), user), amount);
    }

    pub fn withdraw(env: Env, user: Address, amount: i128) {
        user.require_auth();
        let cur = read_staked(&env, &user);
        if cur < amount {
            panic_with_error!(&env, Error::InsufficientStake);
        }
        _claim(&env, &user);
        let new = cur - amount;
        write_staked(&env, &user, new);
        set_reward_debt(&env, &user, new * acc_per_share(&env) / SCALE);
        set_total_staked(&env, total_staked(&env) - amount);
        env.events().publish((symbol_short!("withdraw"), user), amount);
    }

    pub fn distribute(env: Env, amount: i128) {
        admin(&env).require_auth();
        let total = total_staked(&env);
        if total > 0 {
            let tok = token(&env);
            soroban_sdk::token::Client::new(&env, &tok).transfer(
                &admin(&env),
                &env.current_contract_address(),
                &amount,
            );
            set_acc_per_share(&env, acc_per_share(&env) + amount * SCALE / total);
        }
        env.events().publish((symbol_short!("distrib"),), amount);
    }

    pub fn claim(env: Env, user: Address) -> i128 {
        user.require_auth();
        let amt = _claim(&env, &user);
        if amt == 0 {
            panic_with_error!(&env, Error::NoRewards);
        }
        env.events().publish((symbol_short!("claim"), user), amt);
        amt
    }

    pub fn get_staked(env: Env, user: Address) -> i128 {
        read_staked(&env, &user)
    }

    pub fn get_total_staked(env: Env) -> i128 {
        total_staked(&env)
    }

    pub fn get_acc_reward_per_share(env: Env) -> i128 {
        acc_per_share(&env)
    }

    pub fn get_pending_reward(env: Env, user: Address) -> i128 {
        pending(&env, &user)
    }
}

#[cfg(test)]
mod test;
