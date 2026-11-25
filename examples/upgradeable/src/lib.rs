#![no_std]

use soroban_sdk::{
    contract, contracterror, contractevent, contractimpl, contracttype, Address, BytesN, Env,
};

const LEDGERS_PER_DAY: u32 = 17_280;
/// Renew the instance once fewer than this many ledgers remain.
pub const INSTANCE_TTL_THRESHOLD: u32 = 30 * LEDGERS_PER_DAY;
/// Renew the instance to this many ledgers.
pub const INSTANCE_TTL_EXTEND_TO: u32 = 60 * LEDGERS_PER_DAY;

/// Keeps the instance entry (admin, pending admin, version) from being archived.
fn extend_instance_ttl(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(INSTANCE_TTL_THRESHOLD, INSTANCE_TTL_EXTEND_TO);
}

#[contracttype]
pub enum DataKey {
    Admin,
    PendingAdmin,
    Version,
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum Error {
    /// `accept_admin` or `cancel_admin_proposal` called with no proposal pending.
    NoPendingAdmin = 1,
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

#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Upgraded {
    #[topic]
    pub new_wasm_hash: BytesN<32>,
    pub version: u32,
}

fn read_admin(env: &Env) -> Address {
    env.storage().instance().get(&DataKey::Admin).unwrap()
}

fn read_version(env: &Env) -> u32 {
    env.storage().instance().get(&DataKey::Version).unwrap_or(0)
}

#[contract]
pub struct UpgradeableContract;

#[contractimpl]
impl UpgradeableContract {
    pub fn __constructor(env: Env, admin: Address) {
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::Version, &1u32);
        extend_instance_ttl(&env);
    }

    pub fn admin(env: Env) -> Address {
        read_admin(&env)
    }

    pub fn pending_admin(env: Env) -> Option<Address> {
        env.storage().instance().get(&DataKey::PendingAdmin)
    }

    /// Step one of a handover: the current admin nominates `new_admin`.
    pub fn propose_admin(env: Env, new_admin: Address) {
        let adm: Address = read_admin(&env);
        adm.require_auth();
        env.storage()
            .instance()
            .set(&DataKey::PendingAdmin, &new_admin);
        extend_instance_ttl(&env);
        AdminProposed {
            admin: adm,
            new_admin,
        }
        .publish(&env);
    }

    /// Step two: the pending admin accepts and becomes the admin.
    pub fn accept_admin(env: Env) -> Result<(), Error> {
        let pending: Address = env
            .storage()
            .instance()
            .get(&DataKey::PendingAdmin)
            .ok_or(Error::NoPendingAdmin)?;
        pending.require_auth();
        let old_admin = read_admin(&env);
        env.storage().instance().set(&DataKey::Admin, &pending);
        env.storage().instance().remove(&DataKey::PendingAdmin);
        extend_instance_ttl(&env);
        AdminAccepted {
            old_admin,
            new_admin: pending,
        }
        .publish(&env);
        Ok(())
    }

    /// The current admin withdraws a pending proposal.
    pub fn cancel_admin_proposal(env: Env) -> Result<(), Error> {
        let adm: Address = read_admin(&env);
        adm.require_auth();
        let pending: Address = env
            .storage()
            .instance()
            .get(&DataKey::PendingAdmin)
            .ok_or(Error::NoPendingAdmin)?;
        env.storage().instance().remove(&DataKey::PendingAdmin);
        extend_instance_ttl(&env);
        AdminProposalCancelled {
            admin: adm,
            pending,
        }
        .publish(&env);
        Ok(())
    }

    /// Returns the current on-chain version of this contract.
    ///
    /// The version starts at `1` when the contract is first deployed (set in
    /// `__constructor`) and is bumped by `1` on every successful `upgrade`.
    /// Each new Wasm implementation is expected to keep this bump policy so
    /// that indexers and `soroban-forge verify` can track which implementation
    /// is live.
    pub fn version(env: Env) -> u32 {
        read_version(&env)
    }

    pub fn upgrade(env: Env, new_wasm_hash: BytesN<32>) {
        let adm: Address = read_admin(&env);
        adm.require_auth();
        env.deployer()
            .update_current_contract_wasm(new_wasm_hash.clone());

        let version = read_version(&env) + 1;
        env.storage().instance().set(&DataKey::Version, &version);
        extend_instance_ttl(&env);

        Upgraded {
            new_wasm_hash,
            version,
        }
        .publish(&env);
    }
}

#[cfg(test)]
mod test;