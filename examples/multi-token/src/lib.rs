//! A multi-token contract (ERC1155-style) managing many token ids with
//! per-id, per-owner balances in a single contract instance.

#![no_std]

use soroban_sdk::{
    contract, contracterror, contractevent, contractimpl, contracttype, panic_with_error, symbol_short, Address,
    Env, String, Vec,
};

pub const DAY_IN_LEDGERS: u32 = 17280;
pub const BALANCE_TTL: u32 = 30 * DAY_IN_LEDGERS;
pub const BALANCE_TTL_THRESHOLD: u32 = BALANCE_TTL - DAY_IN_LEDGERS;
/// Instance storage (admin and metadata) is bumped to this many ledgers on
/// every state-changing call so the contract is not archived while in use.
pub const INSTANCE_TTL: u32 = 30 * DAY_IN_LEDGERS;
pub const INSTANCE_TTL_THRESHOLD: u32 = INSTANCE_TTL - DAY_IN_LEDGERS;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum Error {
    NegativeAmount = 1,
    InsufficientBalance = 2,
    MismatchedLengths = 3,
    /// `accept_admin` or `cancel_admin_proposal` called with no proposal pending.
    NoPendingAdmin = 4,
}

#[contracttype]
#[derive(Clone)]
pub struct BalanceKey {
    pub owner: Address,
    pub id: u32,
}

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    Admin,
    PendingAdmin,
    Name,
    Symbol,
    Balance(BalanceKey),
}

fn extend_instance(env: &Env) {
    env.storage().instance().extend_ttl(INSTANCE_TTL_THRESHOLD, INSTANCE_TTL);
}

fn check_nonnegative(env: &Env, amount: i128) {
    if amount < 0 {
        panic_with_error!(env, Error::NegativeAmount);
    }
}

fn read_balance(env: &Env, owner: &Address, id: u32) -> i128 {
    let key = DataKey::Balance(BalanceKey { owner: owner.clone(), id });
    if let Some(balance) = env.storage().persistent().get::<_, i128>(&key) {
        env.storage().persistent().extend_ttl(&key, BALANCE_TTL_THRESHOLD, BALANCE_TTL);
        balance
    } else {
        0
    }
}

fn write_balance(env: &Env, owner: &Address, id: u32, amount: i128) {
    let key = DataKey::Balance(BalanceKey { owner: owner.clone(), id });
    env.storage().persistent().set(&key, &amount);
    env.storage().persistent().extend_ttl(&key, BALANCE_TTL_THRESHOLD, BALANCE_TTL);
}

fn spend_balance(env: &Env, owner: &Address, id: u32, amount: i128) {
    let balance = read_balance(env, owner, id);
    if balance < amount {
        panic_with_error!(env, Error::InsufficientBalance);
    }
    write_balance(env, owner, id, balance - amount);
}

fn receive_balance(env: &Env, owner: &Address, id: u32, amount: i128) {
    write_balance(env, owner, id, read_balance(env, owner, id) + amount);
}

fn require_same_length(env: &Env, len_a: u32, len_b: u32) {
    if len_a != len_b {
        panic_with_error!(env, Error::MismatchedLengths);
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
pub struct MultiTokenContract;

#[contractimpl]
impl MultiTokenContract {
    pub fn __constructor(env: Env, admin: Address, name: String, symbol: String) {
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::Name, &name);
        env.storage().instance().set(&DataKey::Symbol, &symbol);
        extend_instance(&env);
    }

    pub fn admin(env: Env) -> Address {
        env.storage().instance().get(&DataKey::Admin).unwrap()
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

    pub fn name(env: Env) -> String {
        env.storage().instance().get(&DataKey::Name).unwrap()
    }

    pub fn symbol(env: Env) -> String {
        env.storage().instance().get(&DataKey::Symbol).unwrap()
    }

    pub fn balance_of(env: Env, owner: Address, id: u32) -> i128 {
        read_balance(&env, &owner, id)
    }

    pub fn balance_of_batch(env: Env, owners: Vec<Address>, ids: Vec<u32>) -> Vec<i128> {
        require_same_length(&env, owners.len(), ids.len());
        let mut balances = Vec::new(&env);
        for i in 0..owners.len() {
            balances.push_back(read_balance(&env, &owners.get(i).unwrap(), ids.get(i).unwrap()));
        }
        balances
    }

    pub fn mint(env: Env, to: Address, id: u32, amount: i128) {
        check_nonnegative(&env, amount);
        let admin: Address = env.storage().instance().get(&DataKey::Admin).unwrap();
        admin.require_auth();
        extend_instance(&env);
        receive_balance(&env, &to, id, amount);
        env.events().publish((symbol_short!("mint"), admin, to, id), amount);
    }

    pub fn mint_batch(env: Env, to: Address, ids: Vec<u32>, amounts: Vec<i128>) {
        require_same_length(&env, ids.len(), amounts.len());
        let admin: Address = env.storage().instance().get(&DataKey::Admin).unwrap();
        admin.require_auth();
        extend_instance(&env);
        for i in 0..ids.len() {
            let id = ids.get(i).unwrap();
            let amount = amounts.get(i).unwrap();
            check_nonnegative(&env, amount);
            receive_balance(&env, &to, id, amount);
        }
        env.events().publish((symbol_short!("mintbatch"), admin, to), (ids, amounts));
    }

    pub fn transfer(env: Env, from: Address, to: Address, id: u32, amount: i128) {
        from.require_auth();
        extend_instance(&env);
        check_nonnegative(&env, amount);
        spend_balance(&env, &from, id, amount);
        receive_balance(&env, &to, id, amount);
        env.events().publish((symbol_short!("transfer"), from, to, id), amount);
    }

    pub fn transfer_batch(env: Env, from: Address, to: Address, ids: Vec<u32>, amounts: Vec<i128>) {
        from.require_auth();
        extend_instance(&env);
        require_same_length(&env, ids.len(), amounts.len());
        for i in 0..ids.len() {
            let id = ids.get(i).unwrap();
            let amount = amounts.get(i).unwrap();
            check_nonnegative(&env, amount);
            spend_balance(&env, &from, id, amount);
            receive_balance(&env, &to, id, amount);
        }
        env.events().publish((symbol_short!("xferbatch"), from, to), (ids, amounts));
    }

    pub fn burn(env: Env, from: Address, id: u32, amount: i128) {
        from.require_auth();
        extend_instance(&env);
        check_nonnegative(&env, amount);
        spend_balance(&env, &from, id, amount);
        env.events().publish((symbol_short!("burn"), from, id), amount);
    }

    pub fn burn_batch(env: Env, from: Address, ids: Vec<u32>, amounts: Vec<i128>) {
        from.require_auth();
        extend_instance(&env);
        require_same_length(&env, ids.len(), amounts.len());
        for i in 0..ids.len() {
            let id = ids.get(i).unwrap();
            let amount = amounts.get(i).unwrap();
            check_nonnegative(&env, amount);
            spend_balance(&env, &from, id, amount);
        }
        env.events().publish((symbol_short!("burnbatch"), from), (ids, amounts));
    }
}

#[cfg(test)]
mod test;