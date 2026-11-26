#![no_std]

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, symbol_short, token, Address, Env,
};

/// Ledgers per day, assuming ~5s per ledger.
pub const DAY_IN_LEDGERS: u32 = 17_280;
/// Instance storage TTL (30 days): asset, total shares and total assets are
/// bumped on every state-changing call so the vault is not archived while in use.
pub const INSTANCE_TTL: u32 = 30 * DAY_IN_LEDGERS;
/// Threshold at which a state-changing call extends the instance TTL.
pub const INSTANCE_TTL_THRESHOLD: u32 = INSTANCE_TTL - DAY_IN_LEDGERS;
/// Share balance TTL (30 days), extended whenever a balance is written or read.
pub const SHARES_TTL: u32 = 30 * DAY_IN_LEDGERS;
/// Threshold at which a share balance read or write extends its TTL.
pub const SHARES_TTL_THRESHOLD: u32 = SHARES_TTL - DAY_IN_LEDGERS;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum VaultError {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    ZeroAmount = 3,
    InsufficientBalance = 4,
    ZeroShares = 5,
    ZeroAssets = 6,
    EmptyVaultYield = 7,
    InsufficientAllowance = 8,
}

#[contracttype]
#[derive(Clone)]
enum DataKey {
    Asset,
    TotalShares,
    TotalAssets,
    Shares(Address),
    Allowance(Address, Address),
}

fn extend_instance_ttl(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(INSTANCE_TTL_THRESHOLD, INSTANCE_TTL);
}

fn write_shares(env: &Env, account: &Address, shares: i128) {
    let key = DataKey::Shares(account.clone());
    env.storage().persistent().set(&key, &shares);
    env.storage()
        .persistent()
        .extend_ttl(&key, SHARES_TTL_THRESHOLD, SHARES_TTL);
}

#[contract]
pub struct YieldVaultContract;

#[contractimpl]
impl YieldVaultContract {
    /// Deploy-time setup with the underlying SEP-41 token. Runs atomically
    /// with deployment, so there is no window where an attacker could race
    /// a separate initialize call to pick the vault's asset.
    pub fn __constructor(env: Env, asset: Address) {
        env.storage().instance().set(&DataKey::Asset, &asset);
        env.storage().instance().set(&DataKey::TotalShares, &0_i128);
        env.storage().instance().set(&DataKey::TotalAssets, &0_i128);
        extend_instance_ttl(&env);
    }

    /// Return the underlying asset token address.
    pub fn asset(env: Env) -> Result<Address, VaultError> {
        env.storage()
            .instance()
            .get(&DataKey::Asset)
            .ok_or(VaultError::NotInitialized)
    }

    /// Return the total underlying assets held and managed by the vault.
    pub fn total_assets(env: Env) -> i128 {
        env.storage()
            .instance()
            .get(&DataKey::TotalAssets)
            .unwrap_or(0)
    }

    /// Return the total number of vault shares currently minted.
    pub fn total_shares(env: Env) -> i128 {
        env.storage()
            .instance()
            .get(&DataKey::TotalShares)
            .unwrap_or(0)
    }

    /// Return the share balance for a specific account.
    pub fn balance_of(env: Env, account: Address) -> i128 {
        let key = DataKey::Shares(account);
        if let Some(shares) = env.storage().persistent().get::<_, i128>(&key) {
            env.storage()
                .persistent()
                .extend_ttl(&key, SHARES_TTL_THRESHOLD, SHARES_TTL);
            shares
        } else {
            0
        }
    }

    /// Preview the amount of shares that would be minted for a deposit of `assets`.
    ///
    /// Rounding policy: always rounds DOWN (truncating integer division),
    /// which strictly favours the vault / existing shareholders.
    pub fn convert_to_shares(env: Env, assets: i128) -> i128 {
        if assets <= 0 {
            return 0;
        }

        let total_assets = Self::total_assets(env.clone());
        let total_shares = Self::total_shares(env);

        if total_shares == 0 || total_assets == 0 {
            // Initial 1:1 exchange rate
            assets
        } else {
            // Floor division: (assets * total_shares) / total_assets
            assets
                .checked_mul(total_shares)
                .expect("multiplication overflow")
                / total_assets
        }
    }

    /// Preview the amount of assets that would be redeemed for `shares`.
    ///
    /// Rounding policy: always rounds DOWN (truncating integer division),
    /// which strictly favours the vault / remaining shareholders.
    pub fn convert_to_assets(env: Env, shares: i128) -> i128 {
        if shares <= 0 {
            return 0;
        }

        let total_assets = Self::total_assets(env.clone());
        let total_shares = Self::total_shares(env);

        if total_shares == 0 {
            0
        } else {
            // Floor division: (shares * total_assets) / total_shares
            shares
                .checked_mul(total_assets)
                .expect("multiplication overflow")
                / total_shares
        }
    }

    /// Deposit `assets` of the underlying token into the vault in exchange for shares.
    ///
    /// Shares minted: `(assets * total_shares) / total_assets` (rounded down).
    /// The first deposit into an empty vault must be at least `100` units to
    /// prevent dust-share inflation attacks (#519).
    pub fn deposit(env: Env, from: Address, assets: i128) -> Result<i128, VaultError> {
        from.require_auth();
        extend_instance_ttl(&env);

        if assets <= 0 {
            return Err(VaultError::ZeroAmount);
        }

        if Self::total_shares(env.clone()) == 0 && assets < 100 {
            return Err(VaultError::ZeroShares);
        }

        let asset_address = Self::asset(env.clone())?;
        let shares = Self::convert_to_shares(env.clone(), assets);
        if shares <= 0 {
            return Err(VaultError::ZeroShares);
        }

        let token_client = token::Client::new(&env, &asset_address);
        token_client.transfer(&from, &env.current_contract_address(), &assets);

        let new_total_assets = Self::total_assets(env.clone())
            .checked_add(assets)
            .expect("overflow");
        let new_total_shares = Self::total_shares(env.clone())
            .checked_add(shares)
            .expect("overflow");
        let user_shares = Self::balance_of(env.clone(), from.clone())
            .checked_add(shares)
            .expect("overflow");

        env.storage()
            .instance()
            .set(&DataKey::TotalAssets, &new_total_assets);
        env.storage()
            .instance()
            .set(&DataKey::TotalShares, &new_total_shares);
        write_shares(&env, &from, user_shares);

        env.events()
            .publish((symbol_short!("deposit"), from), (assets, shares));

        Ok(shares)
    }

    /// Redeem `shares` for underlying assets.
    ///
    /// Assets returned: `(shares * total_assets) / total_shares` (rounded down).
    pub fn withdraw(env: Env, from: Address, shares: i128) -> Result<i128, VaultError> {
        from.require_auth();
        extend_instance_ttl(&env);

        if shares <= 0 {
            return Err(VaultError::ZeroAmount);
        }

        let user_shares = Self::balance_of(env.clone(), from.clone());
        if user_shares < shares {
            return Err(VaultError::InsufficientBalance);
        }

        let asset_address = Self::asset(env.clone())?;
        let assets = Self::convert_to_assets(env.clone(), shares);
        if assets <= 0 {
            return Err(VaultError::ZeroAssets);
        }

        let new_user_shares = user_shares - shares;
        let new_total_shares = Self::total_shares(env.clone()) - shares;
        let new_total_assets = Self::total_assets(env.clone()) - assets;

        if new_user_shares == 0 {
            env.storage().persistent().remove(&DataKey::Shares(from.clone()));
        } else {
            write_shares(&env, &from, new_user_shares);
        }

        env.storage()
            .instance()
            .set(&DataKey::TotalShares, &new_total_shares);
        env.storage()
            .instance()
            .set(&DataKey::TotalAssets, &new_total_assets);

        let token_client = token::Client::new(&env, &asset_address);
        token_client.transfer(&env.current_contract_address(), &from, &assets);

        env.events()
            .publish((symbol_short!("withdraw"), from), (assets, shares));

        Ok(assets)
    }

    /// Add yield to the vault by depositing assets without minting shares.
    ///
    /// This increases the total asset pool while keeping total shares constant,
    /// increasing the asset value per share for all existing shareholders.
    pub fn add_yield(env: Env, from: Address, yield_amount: i128) -> Result<(), VaultError> {
        from.require_auth();
        extend_instance_ttl(&env);

        if yield_amount <= 0 {
            return Err(VaultError::ZeroAmount);
        }

        if Self::total_shares(env.clone()) == 0 {
            return Err(VaultError::EmptyVaultYield);
        }

        let asset_address = Self::asset(env.clone())?;
        let token_client = token::Client::new(&env, &asset_address);
        token_client.transfer(&from, &env.current_contract_address(), &yield_amount);

        let new_total_assets = Self::total_assets(env.clone())
            .checked_add(yield_amount)
            .expect("overflow");
        env.storage()
            .instance()
            .set(&DataKey::TotalAssets, &new_total_assets);

        env.events()
            .publish((symbol_short!("yield"), from), yield_amount);

        Ok(())
    }

    /// Transfer `amount` of vault shares from the caller to `to`.
    pub fn transfer(env: Env, from: Address, to: Address, amount: i128) -> Result<(), VaultError> {
        from.require_auth();
        if amount <= 0 {
            return Err(VaultError::ZeroAmount);
        }

        let from_shares = Self::balance_of(env.clone(), from.clone());
        if from_shares < amount {
            return Err(VaultError::InsufficientBalance);
        }

        Self::write_shares(&env, &from, from_shares - amount);
        let to_shares = Self::balance_of(env.clone(), to.clone());
        Self::write_shares(&env, &to, to_shares + amount);

        env.events()
            .publish((symbol_short!("transfer"), from, to), amount);

        Ok(())
    }

    /// Allow `spender` to transfer up to `amount` of the caller's shares.
    pub fn approve(env: Env, from: Address, spender: Address, amount: i128) -> Result<(), VaultError> {
        from.require_auth();
        if amount < 0 {
            return Err(VaultError::ZeroAmount);
        }

        env.storage()
            .temporary()
            .set(&DataKey::Allowance(from.clone(), spender.clone()), &amount);

        env.events()
            .publish((symbol_short!("approve"), from, spender), amount);

        Ok(())
    }

    /// Return the amount `spender` is allowed to transfer from `from`'s shares.
    pub fn allowance(env: Env, from: Address, spender: Address) -> i128 {
        env.storage()
            .temporary()
            .get(&DataKey::Allowance(from, spender))
            .unwrap_or(0)
    }

    /// Transfer `amount` of shares from `from` to `to`, drawing on an allowance
    /// previously set by `from` for the caller (`spender`).
    pub fn transfer_from(
        env: Env,
        spender: Address,
        from: Address,
        to: Address,
        amount: i128,
    ) -> Result<(), VaultError> {
        spender.require_auth();
        if amount <= 0 {
            return Err(VaultError::ZeroAmount);
        }

        let key = DataKey::Allowance(from.clone(), spender.clone());
        let current_allowance: i128 = env.storage().temporary().get(&key).unwrap_or(0);
        if current_allowance < amount {
            return Err(VaultError::InsufficientAllowance);
        }

        let from_shares = Self::balance_of(env.clone(), from.clone());
        if from_shares < amount {
            return Err(VaultError::InsufficientBalance);
        }

        env.storage()
            .temporary()
            .set(&key, &(current_allowance - amount));

        Self::write_shares(&env, &from, from_shares - amount);
        let to_shares = Self::balance_of(env.clone(), to.clone());
        Self::write_shares(&env, &to, to_shares + amount);

        env.events()
            .publish((symbol_short!("transfer"), from, to), amount);

        Ok(())
    }

    fn write_shares(env: &Env, account: &Address, amount: i128) {
        if amount == 0 {
            env.storage().persistent().remove(&DataKey::Shares(account.clone()));
        } else {
            env.storage()
                .persistent()
                .set(&DataKey::Shares(account.clone()), &amount);
        }
    }
}

#[cfg(test)]
mod test;
