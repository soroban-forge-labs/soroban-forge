//! A non-fungible token (NFT) contract implementing ownership, per-token
//! metadata URIs, admin-gated minting, transfers, and burning.

#![no_std]

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, panic_with_error, Address, Env, String,
};

/// Ledgers per day, assuming ~5s per ledger.
pub const DAY_IN_LEDGERS: u32 = 17280;
/// Persistent data TTL (30 days).
pub const PERSISTENT_TTL: u32 = 30 * DAY_IN_LEDGERS;
/// Threshold to trigger TTL extension.
pub const PERSISTENT_TTL_THRESHOLD: u32 = PERSISTENT_TTL - DAY_IN_LEDGERS;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum Error {
    NotAuthorized = 1,
    TokenNotFound = 2,
    TokenAlreadyExists = 3,
}

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    Admin,
    Name,
    Symbol,
    Owner(u32),
    Balance(Address),
    Uri(u32),
}

fn extend_persistent_ttl(env: &Env, key: &DataKey) {
    env.storage()
        .persistent()
        .extend_ttl(key, PERSISTENT_TTL_THRESHOLD, PERSISTENT_TTL);
}

#[contract]
pub struct NftContract;

#[contractimpl]
impl NftContract {
    /// Deploy-time setup for the NFT collection.
    ///
    /// * `admin` — authorized account allowed to mint new tokens
    /// * `name` — name of the NFT collection
    /// * `symbol` — symbol/ticker for the collection
    pub fn __constructor(env: Env, admin: Address, name: String, symbol: String) {
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::Name, &name);
        env.storage().instance().set(&DataKey::Symbol, &symbol);
    }

    /// Return the admin address.
    pub fn admin(env: Env) -> Address {
        env.storage().instance().get(&DataKey::Admin).unwrap()
    }

    /// Return the collection name.
    pub fn name(env: Env) -> String {
        env.storage().instance().get(&DataKey::Name).unwrap()
    }

    /// Return the collection symbol.
    pub fn symbol(env: Env) -> String {
        env.storage().instance().get(&DataKey::Symbol).unwrap()
    }

    /// Return the owner of `token_id`. Panics if the token does not exist.
    pub fn owner_of(env: Env, token_id: u32) -> Address {
        let key = DataKey::Owner(token_id);
        if let Some(owner) = env.storage().persistent().get::<_, Address>(&key) {
            extend_persistent_ttl(&env, &key);
            owner
        } else {
            panic_with_error!(&env, Error::TokenNotFound);
        }
    }

    /// Return the total number of NFTs owned by `owner`.
    pub fn balance_of(env: Env, owner: Address) -> u32 {
        let key = DataKey::Balance(owner);
        if let Some(balance) = env.storage().persistent().get::<_, u32>(&key) {
            extend_persistent_ttl(&env, &key);
            balance
        } else {
            0
        }
    }

    /// Return the metadata URI for `token_id`. Panics if the token does not exist.
    pub fn token_uri(env: Env, token_id: u32) -> String {
        // Ensure token exists
        let _owner = Self::owner_of(env.clone(), token_id);
        let key = DataKey::Uri(token_id);
        if let Some(uri) = env.storage().persistent().get::<_, String>(&key) {
            extend_persistent_ttl(&env, &key);
            uri
        } else {
            String::from_str(&env, "")
        }
    }

    /// Mint a new NFT `token_id` with metadata `uri` to `to`.
    /// Requires admin authorization.
    pub fn mint(env: Env, to: Address, token_id: u32, uri: String) {
        let admin: Address = env.storage().instance().get(&DataKey::Admin).unwrap();
        admin.require_auth();

        let owner_key = DataKey::Owner(token_id);
        if env.storage().persistent().has(&owner_key) {
            panic_with_error!(&env, Error::TokenAlreadyExists);
        }

        env.storage().persistent().set(&owner_key, &to);
        extend_persistent_ttl(&env, &owner_key);

        let uri_key = DataKey::Uri(token_id);
        env.storage().persistent().set(&uri_key, &uri);
        extend_persistent_ttl(&env, &uri_key);

        let balance_key = DataKey::Balance(to.clone());
        let current_balance = Self::balance_of(env.clone(), to);
        env.storage()
            .persistent()
            .set(&balance_key, &(current_balance + 1));
        extend_persistent_ttl(&env, &balance_key);
    }

    /// Transfer `token_id` from `from` to `to`.
    /// Requires `from` authorization.
    pub fn transfer(env: Env, from: Address, to: Address, token_id: u32) {
        from.require_auth();

        let owner = Self::owner_of(env.clone(), token_id);
        if owner != from {
            panic_with_error!(&env, Error::NotAuthorized);
        }

        let owner_key = DataKey::Owner(token_id);
        env.storage().persistent().set(&owner_key, &to);
        extend_persistent_ttl(&env, &owner_key);

        let from_balance_key = DataKey::Balance(from.clone());
        let from_balance = Self::balance_of(env.clone(), from);
        if from_balance > 0 {
            env.storage()
                .persistent()
                .set(&from_balance_key, &(from_balance - 1));
            extend_persistent_ttl(&env, &from_balance_key);
        }

        let to_balance_key = DataKey::Balance(to.clone());
        let to_balance = Self::balance_of(env.clone(), to.clone());
        env.storage()
            .persistent()
            .set(&to_balance_key, &(to_balance + 1));
        extend_persistent_ttl(&env, &to_balance_key);
    }

    /// Burn `token_id` owned by `from`.
    /// Requires `from` authorization.
    pub fn burn(env: Env, from: Address, token_id: u32) {
        from.require_auth();

        let owner = Self::owner_of(env.clone(), token_id);
        if owner != from {
            panic_with_error!(&env, Error::NotAuthorized);
        }

        let owner_key = DataKey::Owner(token_id);
        env.storage().persistent().remove(&owner_key);

        let uri_key = DataKey::Uri(token_id);
        env.storage().persistent().remove(&uri_key);

        let balance_key = DataKey::Balance(from.clone());
        let balance = Self::balance_of(env.clone(), from);
        if balance > 0 {
            env.storage()
                .persistent()
                .set(&balance_key, &(balance - 1));
            extend_persistent_ttl(&env, &balance_key);
        }
    }
}

#[cfg(test)]
mod test;
