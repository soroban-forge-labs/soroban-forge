//! A soulbound (non-transferable) token contract: admin-gated minting,
//! self-service burning, and a `transfer` entrypoint that is always
//! rejected — tokens are permanently bound to the address they were
//! minted to.

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
/// Instance storage TTL (30 days): admin and collection metadata are bumped on
/// every state-changing call so the contract is not archived while in use.
pub const INSTANCE_TTL: u32 = 30 * DAY_IN_LEDGERS;
/// Threshold to trigger instance TTL extension.
pub const INSTANCE_TTL_THRESHOLD: u32 = INSTANCE_TTL - DAY_IN_LEDGERS;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum Error {
    NotAuthorized = 1,
    TokenNotFound = 2,
    TokenAlreadyExists = 3,
    NonTransferable = 4,
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

fn extend_instance_ttl(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(INSTANCE_TTL_THRESHOLD, INSTANCE_TTL);
}

#[contract]
pub struct SoulboundContract;

#[contractimpl]
impl SoulboundContract {
    /// Deploy-time setup for the soulbound token collection.
    ///
    /// * `admin` — authorized account allowed to mint new tokens
    /// * `name` — name of the token collection
    /// * `symbol` — symbol/ticker for the collection
    pub fn __constructor(env: Env, admin: Address, name: String, symbol: String) {
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::Name, &name);
        env.storage().instance().set(&DataKey::Symbol, &symbol);
        extend_instance_ttl(&env);
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

    /// Return the total number of tokens owned by `owner`.
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

    /// Mint a new soulbound token `token_id` with metadata `uri`, bound to `to`.
    /// Requires admin authorization.
    pub fn mint(env: Env, to: Address, token_id: u32, uri: String) {
        let admin: Address = env.storage().instance().get(&DataKey::Admin).unwrap();
        admin.require_auth();
        extend_instance_ttl(&env);

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

    /// Always rejected — soulbound tokens are permanently bound to the
    /// address they were minted to and can never change owner.
    pub fn transfer(env: Env, _from: Address, _to: Address, _token_id: u32) {
        panic_with_error!(&env, Error::NonTransferable);
    }

    /// Burn `token_id` owned by `from`.
    /// Requires `from` authorization.
    pub fn burn(env: Env, from: Address, token_id: u32) {
        from.require_auth();
        extend_instance_ttl(&env);

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
