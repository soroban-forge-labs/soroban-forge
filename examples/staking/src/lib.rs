#![no_std]

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, panic_with_error, symbol_short, Address,
    Env,
};

const SCALE: i128 = 1_000_000_000_000_000_000;

pub const DAY_IN_LEDGERS: u32 = 17_280;
pub const INSTANCE_TTL: u32 = 30 * DAY_IN_LEDGERS;
pub const INSTANCE_TTL_THRESHOLD: u32 = INSTANCE_TTL - DAY_IN_LEDGERS;
pub const PERSISTENT_TTL: u32 = 30 * DAY_IN_LEDGERS;
pub const PERSISTENT_TTL_THRESHOLD: u32 = PERSISTENT_TTL - DAY_IN_LEDGERS;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum Error {
    InsufficientStake = 1,
    NotAdmin = 2,
    NoRewards = 3,
    InvalidAmount = 4,
    NoStakers = 5,
    /// An intermediate multiplication or division would overflow i128.
    /// This can happen when `amount * SCALE` exceeds i128::MAX, which
    /// is possible for 18-decimal tokens once `amount` exceeds roughly
    /// 1.7 × 10²⁰ base units (~170 tokens at 18 decimals).
    Overflow = 6,
}

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    Admin,
    StakeToken,
    RewardToken,
    AccRewardPerShare,
    TotalStaked,
    Staked(Address),
    RewardDebt(Address),
}

fn admin(env: &Env) -> Address {
    env.storage().instance().get(&DataKey::Admin).unwrap()
}

fn stake_token(env: &Env) -> Address {
    env.storage().instance().get(&DataKey::StakeToken).unwrap()
}

fn reward_token(env: &Env) -> Address {
    env.storage().instance().get(&DataKey::RewardToken).unwrap()
}

fn acc_per_share(env: &Env) -> i128 {
    env.storage()
        .instance()
        .get(&DataKey::AccRewardPerShare)
        .unwrap_or(0)
}

fn set_acc_per_share(env: &Env, val: i128) {
    env.storage()
        .instance()
        .set(&DataKey::AccRewardPerShare, &val);
}

fn total_staked(env: &Env) -> i128 {
    env.storage()
        .instance()
        .get(&DataKey::TotalStaked)
        .unwrap_or(0)
}

fn set_total_staked(env: &Env, val: i128) {
    env.storage().instance().set(&DataKey::TotalStaked, &val);
}

fn read_staked(env: &Env, user: &Address) -> i128 {
    let key = DataKey::Staked(user.clone());
    let val = env
        .storage()
        .persistent()
        .get(&key)
        .unwrap_or(0);
    if val > 0 {
        env.storage()
            .persistent()
            .extend_ttl(&key, PERSISTENT_TTL_THRESHOLD, PERSISTENT_TTL);
    }
    val
}

fn write_staked(env: &Env, user: &Address, val: i128) {
    let key = DataKey::Staked(user.clone());
    env.storage()
        .persistent()
        .set(&key, &val);
    env.storage()
        .persistent()
        .extend_ttl(&key, PERSISTENT_TTL_THRESHOLD, PERSISTENT_TTL);
}

fn reward_debt(env: &Env, user: &Address) -> i128 {
    let key = DataKey::RewardDebt(user.clone());
    let val = env
        .storage()
        .persistent()
        .get(&key)
        .unwrap_or(0);
    if val > 0 {
        env.storage()
            .persistent()
            .extend_ttl(&key, PERSISTENT_TTL_THRESHOLD, PERSISTENT_TTL);
    }
    val
}

fn set_reward_debt(env: &Env, user: &Address, val: i128) {
    let key = DataKey::RewardDebt(user.clone());
    env.storage()
        .persistent()
        .set(&key, &val);
    env.storage()
        .persistent()
        .extend_ttl(&key, PERSISTENT_TTL_THRESHOLD, PERSISTENT_TTL);
}

fn pending(env: &Env, user: &Address) -> i128 {
    let s = read_staked(env, user);
    if s == 0 {
        return 0;
    }
    let acc = acc_per_share(env);
    let debt = reward_debt(env, user);
    // s * acc can overflow when both values are large; fail loudly rather than wrap.
    let raw = s
        .checked_mul(acc)
        .unwrap_or_else(|| panic_with_error!(env, Error::Overflow))
        / SCALE;
    if raw > debt {
        raw - debt
    } else {
        0
    }
}

fn _claim(env: &Env, user: &Address) -> i128 {
    let amt = pending(env, user);
    if amt > 0 {
        let s = read_staked(env, user);
        let new_debt = s
            .checked_mul(acc_per_share(env))
            .unwrap_or_else(|| panic_with_error!(env, Error::Overflow))
            / SCALE;
        set_reward_debt(env, user, new_debt);
        let tok = reward_token(env);
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
    /// `stake_token` is what users deposit; `reward_token` is what
    /// `distribute` pays out. Pass the same address twice for the classic
    /// single-token setup.
    pub fn __constructor(env: Env, admin: Address, stake_token: Address, reward_token: Address) {
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage()
            .instance()
            .set(&DataKey::StakeToken, &stake_token);
        env.storage()
            .instance()
            .set(&DataKey::RewardToken, &reward_token);
    }

    pub fn deposit(env: Env, user: Address, amount: i128) {
        user.require_auth();
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_TTL_THRESHOLD, INSTANCE_TTL);
        if amount <= 0 {
            panic_with_error!(&env, Error::InvalidAmount);
        }
        _claim(&env, &user);
        let tok = stake_token(&env);
        soroban_sdk::token::Client::new(&env, &tok).transfer(
            &user,
            &env.current_contract_address(),
            &amount,
        );
        let new = read_staked(&env, &user) + amount;
        write_staked(&env, &user, new);
        let new_debt = new
            .checked_mul(acc_per_share(&env))
            .unwrap_or_else(|| panic_with_error!(&env, Error::Overflow))
            / SCALE;
        set_reward_debt(&env, &user, new_debt);
        set_total_staked(&env, total_staked(&env) + amount);
        env.events()
            .publish((symbol_short!("deposit"), user), amount);
    }

    pub fn withdraw(env: Env, user: Address, amount: i128) {
        user.require_auth();
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_TTL_THRESHOLD, INSTANCE_TTL);
        if amount <= 0 {
            panic_with_error!(&env, Error::InvalidAmount);
        }
        let cur = read_staked(&env, &user);
        if cur < amount {
            panic_with_error!(&env, Error::InsufficientStake);
        }
        _claim(&env, &user);
        let new = cur - amount;
        write_staked(&env, &user, new);
        let new_debt = new
            .checked_mul(acc_per_share(&env))
            .unwrap_or_else(|| panic_with_error!(&env, Error::Overflow))
            / SCALE;
        set_reward_debt(&env, &user, new_debt);
        set_total_staked(&env, total_staked(&env) - amount);
        let tok = stake_token(&env);
        soroban_sdk::token::Client::new(&env, &tok).transfer(
            &env.current_contract_address(),
            &user,
            &amount,
        );
        env.events()
            .publish((symbol_short!("withdraw"), user), amount);
    }

    pub fn distribute(env: Env, amount: i128) {
        admin(&env).require_auth();
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_TTL_THRESHOLD, INSTANCE_TTL);
        let total = total_staked(&env);
        if total == 0 {
            panic_with_error!(&env, Error::NoStakers);
        }
        // Validate arithmetic before moving tokens: amount * SCALE overflows
        // i128 when amount > i128::MAX / SCALE (~1.7 × 10²⁰ base units, or
        // ~170 whole tokens at 18 decimals). Surface this as Error::Overflow
        // rather than a silent wrap-around trap.
        let delta = amount
            .checked_mul(SCALE)
            .unwrap_or_else(|| panic_with_error!(&env, Error::Overflow))
            / total;
        let tok = reward_token(&env);
        soroban_sdk::token::Client::new(&env, &tok).transfer(
            &admin(&env),
            &env.current_contract_address(),
            &amount,
        );
        set_acc_per_share(&env, acc_per_share(&env) + delta);
        env.events().publish((symbol_short!("distrib"),), amount);
    }

    pub fn claim(env: Env, user: Address) -> i128 {
        user.require_auth();
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_TTL_THRESHOLD, INSTANCE_TTL);
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

    pub fn get_stake_token(env: Env) -> Address {
        stake_token(&env)
    }

    pub fn get_reward_token(env: Env) -> Address {
        reward_token(&env)
    }
}

#[cfg(test)]
mod test;
