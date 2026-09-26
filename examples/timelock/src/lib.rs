#![no_std]

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, symbol_short, xdr::ToXdr, Address, Bytes,
    BytesN, Env, Symbol, Val, Vec,
};

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum TimelockError {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    Unauthorized = 3,
    InvalidDelay = 4,
    AlreadyQueued = 5,
    NotQueued = 6,
    NotReady = 7,
    AlreadyExecuted = 8,
    AlreadyCancelled = 9,
}

#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum Role {
    Admin = 1,
    Proposer = 2,
    Executor = 3,
}

#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum OperationStatus {
    Unset = 0,
    Waiting = 1,
    Ready = 2,
    Executed = 3,
    Cancelled = 4,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct Operation {
    pub target: Address,
    pub function_name: Symbol,
    pub args: Vec<Val>,
    pub ready_at: u64,
    pub executed: bool,
    pub cancelled: bool,
}

#[contracttype]
#[derive(Clone)]
enum DataKey {
    Admin,
    MinDelay,
    Role(Role, Address),
    Op(BytesN<32>),
}

#[contract]
pub struct TimelockContract;

#[contractimpl]
impl TimelockContract {
    /// Initialize the timelock controller with an admin, proposers, executors, and minimum delay.
    pub fn initialize(
        env: Env,
        admin: Address,
        proposers: Vec<Address>,
        executors: Vec<Address>,
        min_delay: u64,
    ) -> Result<(), TimelockError> {
        if env.storage().instance().has(&DataKey::Admin) {
            return Err(TimelockError::AlreadyInitialized);
        }

        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::MinDelay, &min_delay);

        // Grant Admin role to admin address
        env.storage()
            .persistent()
            .set(&DataKey::Role(Role::Admin, admin.clone()), &true);

        for proposer in proposers.iter() {
            env.storage()
                .persistent()
                .set(&DataKey::Role(Role::Proposer, proposer), &true);
        }

        for executor in executors.iter() {
            env.storage()
                .persistent()
                .set(&DataKey::Role(Role::Executor, executor), &true);
        }

        Ok(())
    }

    /// Return the minimum execution delay in seconds.
    pub fn get_min_delay(env: Env) -> u64 {
        env.storage()
            .instance()
            .get(&DataKey::MinDelay)
            .unwrap_or(0)
    }

    /// Update the minimum delay (admin only or timelock self-call).
    pub fn set_min_delay(env: Env, caller: Address, new_delay: u64) -> Result<(), TimelockError> {
        caller.require_auth();

        if !Self::has_role(env.clone(), Role::Admin, caller) {
            return Err(TimelockError::Unauthorized);
        }

        env.storage().instance().set(&DataKey::MinDelay, &new_delay);
        Ok(())
    }

    /// Check whether an account has a specific role.
    pub fn has_role(env: Env, role: Role, account: Address) -> bool {
        env.storage()
            .persistent()
            .get(&DataKey::Role(role, account))
            .unwrap_or(false)
    }

    /// Grant a role to an account (admin only).
    pub fn grant_role(
        env: Env,
        admin: Address,
        role: Role,
        account: Address,
    ) -> Result<(), TimelockError> {
        admin.require_auth();

        if !Self::has_role(env.clone(), Role::Admin, admin) {
            return Err(TimelockError::Unauthorized);
        }

        env.storage()
            .persistent()
            .set(&DataKey::Role(role, account), &true);
        Ok(())
    }

    /// Revoke a role from an account (admin only).
    pub fn revoke_role(
        env: Env,
        admin: Address,
        role: Role,
        account: Address,
    ) -> Result<(), TimelockError> {
        admin.require_auth();

        if !Self::has_role(env.clone(), Role::Admin, admin) {
            return Err(TimelockError::Unauthorized);
        }

        env.storage()
            .persistent()
            .remove(&DataKey::Role(role, account));
        Ok(())
    }

    /// Compute the unique operation ID hash for a call specification.
    pub fn hash_operation(
        env: Env,
        target: Address,
        function_name: Symbol,
        args: Vec<Val>,
        salt: BytesN<32>,
    ) -> BytesN<32> {
        let mut bytes = Bytes::new(&env);
        bytes.append(&target.to_xdr(&env));
        bytes.append(&function_name.to_xdr(&env));
        bytes.append(&args.to_xdr(&env));
        bytes.append(&salt.to_xdr(&env));
        env.crypto().sha256(&bytes).into()
    }

    /// Return the current lifecycle state of an operation.
    pub fn get_operation_state(env: Env, id: BytesN<32>) -> OperationStatus {
        let op: Option<Operation> = env.storage().persistent().get(&DataKey::Op(id));
        match op {
            None => OperationStatus::Unset,
            Some(o) => {
                if o.cancelled {
                    OperationStatus::Cancelled
                } else if o.executed {
                    OperationStatus::Executed
                } else if env.ledger().timestamp() >= o.ready_at {
                    OperationStatus::Ready
                } else {
                    OperationStatus::Waiting
                }
            }
        }
    }

    /// Return the full metadata for a queued operation.
    pub fn get_operation(env: Env, id: BytesN<32>) -> Option<Operation> {
        env.storage().persistent().get(&DataKey::Op(id))
    }

    /// Queue a call for delayed execution (proposer only).
    ///
    /// Requires `delay >= min_delay`.
    pub fn queue(
        env: Env,
        caller: Address,
        target: Address,
        function_name: Symbol,
        args: Vec<Val>,
        delay: u64,
        salt: BytesN<32>,
    ) -> Result<BytesN<32>, TimelockError> {
        caller.require_auth();

        if !Self::has_role(env.clone(), Role::Proposer, caller.clone())
            && !Self::has_role(env.clone(), Role::Admin, caller)
        {
            return Err(TimelockError::Unauthorized);
        }

        let min_delay = Self::get_min_delay(env.clone());
        if delay < min_delay {
            return Err(TimelockError::InvalidDelay);
        }

        let id = Self::hash_operation(
            env.clone(),
            target.clone(),
            function_name.clone(),
            args.clone(),
            salt,
        );

        if env.storage().persistent().has(&DataKey::Op(id.clone())) {
            let state = Self::get_operation_state(env.clone(), id.clone());
            if state != OperationStatus::Unset && state != OperationStatus::Cancelled {
                return Err(TimelockError::AlreadyQueued);
            }
        }

        let current_ts = env.ledger().timestamp();
        let ready_at = current_ts.checked_add(delay).expect("overflow");

        let operation = Operation {
            target: target.clone(),
            function_name,
            args,
            ready_at,
            executed: false,
            cancelled: false,
        };

        env.storage()
            .persistent()
            .set(&DataKey::Op(id.clone()), &operation);

        env.events()
            .publish((symbol_short!("queue"), id.clone()), (ready_at, target));

        Ok(id)
    }

    /// Execute a ready queued operation (executor only).
    ///
    /// The current timestamp must be greater than or equal to the operation's `ready_at`.
    pub fn execute(
        env: Env,
        caller: Address,
        target: Address,
        function_name: Symbol,
        args: Vec<Val>,
        salt: BytesN<32>,
    ) -> Result<Val, TimelockError> {
        caller.require_auth();

        if !Self::has_role(env.clone(), Role::Executor, caller.clone())
            && !Self::has_role(env.clone(), Role::Admin, caller)
        {
            return Err(TimelockError::Unauthorized);
        }

        let id = Self::hash_operation(
            env.clone(),
            target.clone(),
            function_name.clone(),
            args.clone(),
            salt,
        );

        let mut operation: Operation = env
            .storage()
            .persistent()
            .get(&DataKey::Op(id.clone()))
            .ok_or(TimelockError::NotQueued)?;

        if operation.cancelled {
            return Err(TimelockError::AlreadyCancelled);
        }

        if operation.executed {
            return Err(TimelockError::AlreadyExecuted);
        }

        if env.ledger().timestamp() < operation.ready_at {
            return Err(TimelockError::NotReady);
        }

        operation.executed = true;
        env.storage()
            .persistent()
            .set(&DataKey::Op(id.clone()), &operation);

        env.events()
            .publish((symbol_short!("exec"), id), target.clone());

        // Invoke the target contract call
        let result = env.invoke_contract::<Val>(&target, &function_name, args);
        Ok(result)
    }

    /// Cancel a queued operation before it is executed (proposer or admin only).
    pub fn cancel(env: Env, caller: Address, id: BytesN<32>) -> Result<(), TimelockError> {
        caller.require_auth();

        if !Self::has_role(env.clone(), Role::Proposer, caller.clone())
            && !Self::has_role(env.clone(), Role::Admin, caller.clone())
        {
            return Err(TimelockError::Unauthorized);
        }

        let mut operation: Operation = env
            .storage()
            .persistent()
            .get(&DataKey::Op(id.clone()))
            .ok_or(TimelockError::NotQueued)?;

        if operation.executed {
            return Err(TimelockError::AlreadyExecuted);
        }

        if operation.cancelled {
            return Err(TimelockError::AlreadyCancelled);
        }

        operation.cancelled = true;
        env.storage()
            .persistent()
            .set(&DataKey::Op(id.clone()), &operation);

        env.events()
            .publish((symbol_short!("cancel"), id), caller);

        Ok(())
    }
}

#[cfg(test)]
mod test;
