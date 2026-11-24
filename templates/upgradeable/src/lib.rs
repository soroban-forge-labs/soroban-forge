#![no_std]

use soroban_sdk::{contract, contractevent, contractimpl, contracttype, Address, BytesN, Env};

#[contracttype]
pub enum DataKey {
    Admin,
    Version,
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
    }

    pub fn admin(env: Env) -> Address {
        read_admin(&env)
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

        Upgraded {
            new_wasm_hash,
            version,
        }
        .publish(&env);
    }
}

#[cfg(test)]
mod test;