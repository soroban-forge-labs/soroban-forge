#![no_std]

use soroban_sdk::{
    contract, contracterror, contractevent, contractimpl, contracttype, Address, BytesN, Env,
};

#[contracttype]
pub enum DataKey {
    Admin,
    PendingAdmin,
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

fn read_admin(env: &Env) -> Address {
    env.storage().instance().get(&DataKey::Admin).unwrap()
}

#[contract]
pub struct UpgradeableContract;

#[contractimpl]
impl UpgradeableContract {
    pub fn __constructor(env: Env, admin: Address) {
        env.storage().instance().set(&DataKey::Admin, &admin);
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
        AdminProposalCancelled {
            admin: adm,
            pending,
        }
        .publish(&env);
        Ok(())
    }

    pub fn upgrade(env: Env, new_wasm_hash: BytesN<32>) {
        let adm: Address = read_admin(&env);
        adm.require_auth();
        env.deployer().update_current_contract_wasm(new_wasm_hash);
    }
}

#[cfg(test)]
mod test;
