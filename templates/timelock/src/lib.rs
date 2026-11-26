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
    NoPendingAdmin = 10,
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
pub struct OperationCall {
    pub target: Address,
    pub function_name: Symbol,
    pub args: Vec<Val>,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct Operation {
    pub target: Address,
    pub function_name: Symbol,
    pub args: Vec<Val>,
    pub calls: Vec<OperationCall>,
    pub ready_at: u64,
    pub executed: bool,
    pub cancelled: bool,
    pub predecessor: Option<BytesN<32>>,
}

#[contracttype]
#[derive(Clone)]
enum DataKey {
    Admin,
    PendingAdmin,
    MinDelay,
    Role(Role, Address),
    Op(BytesN<32>),
}

const LEDGERS_PER_DAY: u32 = 17_280;
/// Renew the instance (admin, pending admin, minimum delay) once fewer than this many ledgers remain.
pub const INSTANCE_TTL_THRESHOLD: u32 = 30 * LEDGERS_PER_DAY;
/// Renew the instance to this many ledgers.
pub const INSTANCE_TTL_EXTEND_TO: u32 = 60 * LEDGERS_PER_DAY;
/// Renew a persistent entry (role grant or queued operation) once fewer than this many ledgers remain.
pub const PERSISTENT_TTL_THRESHOLD: u32 = 30 * LEDGERS_PER_DAY;
/// Renew a persistent entry to this many ledgers.
pub const PERSISTENT_TTL_EXTEND_TO: u32 = 60 * LEDGERS_PER_DAY;

fn extend_instance_ttl(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(INSTANCE_TTL_THRESHOLD, INSTANCE_TTL_EXTEND_TO);
}

fn extend_persistent_ttl(env: &Env, key: &DataKey) {
    env.storage()
        .persistent()
        .extend_ttl(key, PERSISTENT_TTL_THRESHOLD, PERSISTENT_TTL_EXTEND_TO);
}

/// Writes a role grant and keeps it alive.
fn grant(env: &Env, role: Role, account: Address) {
    let key = DataKey::Role(role, account);
    env.storage().persistent().set(&key, &true);
    extend_persistent_ttl(env, &key);
}

/// Writes an operation and keeps it alive.
fn write_operation(env: &Env, id: &BytesN<32>, operation: &Operation) {
    let key = DataKey::Op(id.clone());
    env.storage().persistent().set(&key, operation);
    extend_persistent_ttl(env, &key);
}

/// Reads an operation on a hot path and renews its TTL when present.
fn read_operation(env: &Env, id: &BytesN<32>) -> Option<Operation> {
    let key = DataKey::Op(id.clone());
    let operation: Option<Operation> = env.storage().persistent().get(&key);
    if operation.is_some() {
        extend_persistent_ttl(env, &key);
    }
    operation
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
        grant(&env, Role::Admin, admin.clone());

        for proposer in proposers.iter() {
            grant(&env, Role::Proposer, proposer);
        }

        for executor in executors.iter() {
            grant(&env, Role::Executor, executor);
        }

        extend_instance_ttl(&env);
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
        extend_instance_ttl(&env);
        Ok(())
    }

    /// Propose a new admin (current admin only). The handover completes when
    /// the proposed address calls `accept_admin`.
    pub fn propose_admin(env: Env, caller: Address, new_admin: Address) -> Result<(), TimelockError> {
        caller.require_auth();
        Self::require_current_admin(&env, &caller)?;

        env.storage()
            .instance()
            .set(&DataKey::PendingAdmin, &new_admin);
        extend_instance_ttl(&env);

        env.events()
            .publish((symbol_short!("adm_prop"), caller), new_admin);

        Ok(())
    }

    /// Accept a pending admin handover (pending admin only).
    ///
    /// Moves the stored admin and the Admin role from the old admin to the caller.
    pub fn accept_admin(env: Env, caller: Address) -> Result<(), TimelockError> {
        caller.require_auth();

        let pending: Address = env
            .storage()
            .instance()
            .get(&DataKey::PendingAdmin)
            .ok_or(TimelockError::NoPendingAdmin)?;
        if pending != caller {
            return Err(TimelockError::Unauthorized);
        }

        let old_admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .ok_or(TimelockError::NotInitialized)?;

        env.storage()
            .persistent()
            .remove(&DataKey::Role(Role::Admin, old_admin.clone()));
        grant(&env, Role::Admin, caller.clone());
        env.storage().instance().set(&DataKey::Admin, &caller);
        env.storage().instance().remove(&DataKey::PendingAdmin);
        extend_instance_ttl(&env);

        env.events()
            .publish((symbol_short!("adm_acc"), old_admin), caller);

        Ok(())
    }

    /// Cancel a pending admin handover (current admin only).
    pub fn cancel_admin_proposal(env: Env, caller: Address) -> Result<(), TimelockError> {
        caller.require_auth();
        Self::require_current_admin(&env, &caller)?;

        if !env.storage().instance().has(&DataKey::PendingAdmin) {
            return Err(TimelockError::NoPendingAdmin);
        }
        env.storage().instance().remove(&DataKey::PendingAdmin);
        extend_instance_ttl(&env);

        env.events().publish((symbol_short!("adm_canc"),), caller);

        Ok(())
    }

    /// Return the pending admin, if a handover has been proposed.
    pub fn get_pending_admin(env: Env) -> Option<Address> {
        env.storage().instance().get(&DataKey::PendingAdmin)
    }

    fn require_current_admin(env: &Env, caller: &Address) -> Result<(), TimelockError> {
        let admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .ok_or(TimelockError::NotInitialized)?;
        if &admin != caller {
            return Err(TimelockError::Unauthorized);
        }
        Ok(())
    }

    /// Check whether an account has a specific role.
    ///
    /// Every role-gated call reads its grant through here, so a present grant has its TTL renewed.
    pub fn has_role(env: Env, role: Role, account: Address) -> bool {
        let key = DataKey::Role(role, account);
        let granted = env.storage().persistent().get(&key).unwrap_or(false);
        if granted {
            extend_persistent_ttl(&env, &key);
        }
        granted
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

        grant(&env, role, account);
        extend_instance_ttl(&env);
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
        extend_instance_ttl(&env);
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

    /// Compute the unique ID hash for a batch of calls.
    pub fn hash_batch(
        env: Env,
        calls: Vec<OperationCall>,
        predecessor: Option<BytesN<32>>,
        salt: BytesN<32>,
    ) -> BytesN<32> {
        let mut bytes = Bytes::new(&env);
        bytes.append(&calls.to_xdr(&env));

        match predecessor {
            Some(pred) => {
                bytes.append(&1u32.to_xdr(&env));
                bytes.append(&pred.to_xdr(&env));
            }
            None => {
                bytes.append(&0u32.to_xdr(&env));
            }
        }

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
            salt.clone(),
        );

        if env.storage().persistent().has(&DataKey::Op(id.clone())) {
            let state = Self::get_operation_state(env.clone(), id.clone());
            if state != OperationStatus::Unset && state != OperationStatus::Cancelled {
                return Err(TimelockError::AlreadyQueued);
            }
        }

        let current_ts = env.ledger().timestamp();
        let ready_at = current_ts.checked_add(delay).expect("overflow");
        let mut single_call_batch = Vec::new(&env);
        single_call_batch.push_back(OperationCall {
            target: target.clone(),
            function_name: function_name.clone(),
            args: args.clone(),
        });

        let operation = Operation {
            target: target.clone(),
            function_name,
            args,
            calls: single_call_batch,
            ready_at,
            executed: false,
            cancelled: false,
            predecessor: None,
        };

        write_operation(&env, &id, &operation);
        extend_instance_ttl(&env);

        env.events()
            .publish((symbol_short!("queue"), id.clone()), (ready_at, target));

        Ok(id)
    }

    /// Queue several calls for delayed execution as a single atomic batch.
    pub fn queue_batch(
        env: Env,
        caller: Address,
        calls: Vec<OperationCall>,
        delay: u64,
        predecessor: Option<BytesN<32>>,
        salt: BytesN<32>,
    ) -> Result<BytesN<32>, TimelockError> {
        caller.require_auth();

        if !Self::has_role(env.clone(), Role::Proposer, caller.clone())
            && !Self::has_role(env.clone(), Role::Admin, caller)
        {
            return Err(TimelockError::Unauthorized);
        }

        if calls.is_empty() {
            return Err(TimelockError::NotQueued);
        }

        let min_delay = Self::get_min_delay(env.clone());
        if delay < min_delay {
            return Err(TimelockError::InvalidDelay);
        }

        let first = calls.iter().next().unwrap();
        let id = Self::hash_batch(env.clone(), calls.clone(), predecessor.clone(), salt.clone());

        if env.storage().persistent().has(&DataKey::Op(id.clone())) {
            let state = Self::get_operation_state(env.clone(), id.clone());
            if state != OperationStatus::Unset && state != OperationStatus::Cancelled {
                return Err(TimelockError::AlreadyQueued);
            }
        }

        let current_ts = env.ledger().timestamp();
        let ready_at = current_ts.checked_add(delay).expect("overflow");
        let operation = Operation {
            target: first.target.clone(),
            function_name: first.function_name.clone(),
            args: first.args.clone(),
            calls: calls.clone(),
            ready_at,
            executed: false,
            cancelled: false,
            predecessor: predecessor.clone(),
        };

        write_operation(&env, &id, &operation);
        extend_instance_ttl(&env);

        env.events()
            .publish((symbol_short!("queue"), id.clone()), (ready_at, first.target.clone()));

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

        let mut operation: Operation =
            read_operation(&env, &id).ok_or(TimelockError::NotQueued)?;

        if operation.cancelled {
            return Err(TimelockError::AlreadyCancelled);
        }

        if operation.executed {
            return Err(TimelockError::AlreadyExecuted);
        }

        if env.ledger().timestamp() < operation.ready_at {
            return Err(TimelockError::NotReady);
        }

        if let Some(predecessor) = operation.predecessor.clone() {
            let predecessor_op: Operation =
                read_operation(&env, &predecessor).ok_or(TimelockError::NotQueued)?;
            if predecessor_op.cancelled || !predecessor_op.executed {
                return Err(TimelockError::NotReady);
            }
        }

        operation.executed = true;
        write_operation(&env, &id, &operation);
        extend_instance_ttl(&env);

        env.events()
            .publish((symbol_short!("exec"), id), target.clone());

        // Invoke the target contract call
        let result = env.invoke_contract::<Val>(&target, &function_name, args);
        Ok(result)
    }

    /// Execute a ready queued batch of calls atomically (executor only).
    pub fn execute_batch(
        env: Env,
        caller: Address,
        calls: Vec<OperationCall>,
        predecessor: Option<BytesN<32>>,
        salt: BytesN<32>,
    ) -> Result<Val, TimelockError> {
        caller.require_auth();

        if !Self::has_role(env.clone(), Role::Executor, caller.clone())
            && !Self::has_role(env.clone(), Role::Admin, caller)
        {
            return Err(TimelockError::Unauthorized);
        }

        if calls.is_empty() {
            return Err(TimelockError::NotQueued);
        }

        let id = Self::hash_batch(env.clone(), calls.clone(), predecessor.clone(), salt);

        let mut operation: Operation =
            read_operation(&env, &id).ok_or(TimelockError::NotQueued)?;

        if operation.cancelled {
            return Err(TimelockError::AlreadyCancelled);
        }

        if operation.executed {
            return Err(TimelockError::AlreadyExecuted);
        }

        if env.ledger().timestamp() < operation.ready_at {
            return Err(TimelockError::NotReady);
        }

        if let Some(predecessor_id) = predecessor.clone() {
            let predecessor_op: Operation =
                read_operation(&env, &predecessor_id).ok_or(TimelockError::NotQueued)?;
            if predecessor_op.cancelled || !predecessor_op.executed {
                return Err(TimelockError::NotReady);
            }
        }

        operation.executed = true;
        write_operation(&env, &id, &operation);
        extend_instance_ttl(&env);

        env.events().publish((symbol_short!("exec"), id), calls.clone());

        let mut iter = calls.iter();
        let first_call = iter.next().unwrap();
        let mut result = env.invoke_contract::<Val>(
            &first_call.target,
            &first_call.function_name,
            first_call.args.clone(),
        );

        for call in iter {
            result = env.invoke_contract::<Val>(&call.target, &call.function_name, call.args.clone());
        }

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

        let mut operation: Operation =
            read_operation(&env, &id).ok_or(TimelockError::NotQueued)?;

        if operation.executed {
            return Err(TimelockError::AlreadyExecuted);
        }

        if operation.cancelled {
            return Err(TimelockError::AlreadyCancelled);
        }

        operation.cancelled = true;
        write_operation(&env, &id, &operation);
        extend_instance_ttl(&env);

        env.events()
            .publish((symbol_short!("cancel"), id), caller);

        Ok(())
    }
}

#[cfg(test)]
mod test;
