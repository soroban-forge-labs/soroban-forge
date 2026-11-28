#![cfg(test)]

use super::*;
use soroban_sdk::{
    contract, contractimpl, symbol_short,
    testutils::{Address as _, Ledger as _},
    vec, Address, BytesN, Env, IntoVal, Symbol, Val, Vec,
};

#[contract]
pub struct MockTargetContract;

#[contractimpl]
impl MockTargetContract {
    pub fn set_value(env: Env, new_val: u32) -> u32 {
        env.storage().instance().set(&symbol_short!("val"), &new_val);
        new_val
    }

    pub fn get_value(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&symbol_short!("val"))
            .unwrap_or(0)
    }
}

#[allow(dead_code)]
struct TestContext<'a> {
    contract_id: Address,
    timelock: TimelockContractClient<'a>,
    target_id: Address,
    target: MockTargetContractClient<'a>,
    admin: Address,
    proposer: Address,
    executor: Address,
    mallory: Address,
    min_delay: u64,
}

fn setup(env: &Env) -> TestContext<'_> {
    env.mock_all_auths();

    let admin = Address::generate(env);
    let proposer = Address::generate(env);
    let executor = Address::generate(env);
    let mallory = Address::generate(env);
    let min_delay = 86_400_u64; // 1 day

    let contract_id = env.register(TimelockContract, ());
    let timelock = TimelockContractClient::new(env, &contract_id);

    let proposers = vec![env, proposer.clone()];
    let executors = vec![env, executor.clone()];
    timelock.initialize(&admin, &proposers, &executors, &min_delay);

    let target_id = env.register(MockTargetContract, ());
    let target = MockTargetContractClient::new(env, &target_id);

    TestContext {
        contract_id,
        timelock,
        target_id,
        target,
        admin,
        proposer,
        executor,
        mallory,
        min_delay,
    }
}

#[test]
fn test_initialization() {
    let env = Env::default();
    let ctx = setup(&env);

    assert_eq!(ctx.timelock.get_min_delay(), ctx.min_delay);
    assert!(ctx.timelock.has_role(&Role::Admin, &ctx.admin));
    assert!(ctx.timelock.has_role(&Role::Proposer, &ctx.proposer));
    assert!(ctx.timelock.has_role(&Role::Executor, &ctx.executor));
    assert!(!ctx.timelock.has_role(&Role::Proposer, &ctx.mallory));
}

#[test]
fn test_queue_and_execute_after_delay() {
    let env = Env::default();
    let ctx = setup(&env);

    let start_ts = 1_000_000_u64;
    env.ledger().with_mut(|li| li.timestamp = start_ts);

    let fn_name = Symbol::new(&env, "set_value");
    let args: Vec<Val> = vec![&env, 42_u32.into_val(&env)];
    let salt: BytesN<32> = BytesN::from_array(&env, &[1u8; 32]);

    // 1. Proposer queues the call with 1 day delay
    let op_id = ctx.timelock.queue(
        &ctx.proposer,
        &ctx.target_id,
        &fn_name,
        &args,
        &ctx.min_delay,
        &salt,
    );

    // Verify initial status is Waiting
    assert_eq!(
        ctx.timelock.get_operation_state(&op_id),
        OperationStatus::Waiting
    );

    // 2. Advance time to half the delay (still waiting)
    env.ledger()
        .with_mut(|li| li.timestamp = start_ts + (ctx.min_delay / 2));
    assert_eq!(
        ctx.timelock.get_operation_state(&op_id),
        OperationStatus::Waiting
    );

    // 3. Advance time to exactly the ready timestamp
    env.ledger()
        .with_mut(|li| li.timestamp = start_ts + ctx.min_delay);
    assert_eq!(
        ctx.timelock.get_operation_state(&op_id),
        OperationStatus::Ready
    );

    // Target contract has initial value 0
    assert_eq!(ctx.target.get_value(), 0);

    // 4. Executor executes the operation
    let _ = ctx.timelock.execute(
        &ctx.executor,
        &ctx.target_id,
        &fn_name,
        &args,
        &salt,
    );

    // Operation is now Executed
    assert_eq!(
        ctx.timelock.get_operation_state(&op_id),
        OperationStatus::Executed
    );

    // Verify target contract received the call and updated state
    assert_eq!(ctx.target.get_value(), 42);
}

#[test]
#[should_panic(expected = "Error(Contract, #7)")]
fn test_early_execute_rejection() {
    let env = Env::default();
    let ctx = setup(&env);

    let start_ts = 1_000_000_u64;
    env.ledger().with_mut(|li| li.timestamp = start_ts);

    let fn_name = Symbol::new(&env, "set_value");
    let args: Vec<Val> = vec![&env, 100_u32.into_val(&env)];
    let salt: BytesN<32> = BytesN::from_array(&env, &[2u8; 32]);

    ctx.timelock.queue(
        &ctx.proposer,
        &ctx.target_id,
        &fn_name,
        &args,
        &ctx.min_delay,
        &salt,
    );

    // 1 second before ready timestamp
    env.ledger()
        .with_mut(|li| li.timestamp = start_ts + ctx.min_delay - 1);

    // Early execute must be rejected with TimelockError::NotReady (#7)
    ctx.timelock.execute(
        &ctx.executor,
        &ctx.target_id,
        &fn_name,
        &args,
        &salt,
    );
}

#[test]
fn test_cancel_operation() {
    let env = Env::default();
    let ctx = setup(&env);

    let start_ts = 1_000_000_u64;
    env.ledger().with_mut(|li| li.timestamp = start_ts);

    let fn_name = Symbol::new(&env, "set_value");
    let args: Vec<Val> = vec![&env, 999_u32.into_val(&env)];
    let salt: BytesN<32> = BytesN::from_array(&env, &[3u8; 32]);

    let op_id = ctx.timelock.queue(
        &ctx.proposer,
        &ctx.target_id,
        &fn_name,
        &args,
        &ctx.min_delay,
        &salt,
    );

    assert_eq!(
        ctx.timelock.get_operation_state(&op_id),
        OperationStatus::Waiting
    );

    // Proposer cancels the operation
    ctx.timelock.cancel(&ctx.proposer, &op_id);

    // Status is Cancelled
    assert_eq!(
        ctx.timelock.get_operation_state(&op_id),
        OperationStatus::Cancelled
    );
}

#[test]
#[should_panic(expected = "Error(Contract, #9)")]
fn test_cannot_execute_cancelled_operation() {
    let env = Env::default();
    let ctx = setup(&env);

    let start_ts = 1_000_000_u64;
    env.ledger().with_mut(|li| li.timestamp = start_ts);

    let fn_name = Symbol::new(&env, "set_value");
    let args: Vec<Val> = vec![&env, 999_u32.into_val(&env)];
    let salt: BytesN<32> = BytesN::from_array(&env, &[4u8; 32]);

    let op_id = ctx.timelock.queue(
        &ctx.proposer,
        &ctx.target_id,
        &fn_name,
        &args,
        &ctx.min_delay,
        &salt,
    );

    ctx.timelock.cancel(&ctx.proposer, &op_id);

    // Advance past delay
    env.ledger()
        .with_mut(|li| li.timestamp = start_ts + ctx.min_delay + 100);

    // Attempting to execute cancelled operation fails with AlreadyCancelled (#9)
    ctx.timelock.execute(
        &ctx.executor,
        &ctx.target_id,
        &fn_name,
        &args,
        &salt,
    );
}

#[test]
#[should_panic(expected = "Error(Contract, #4)")]
fn test_delay_below_minimum_rejected() {
    let env = Env::default();
    let ctx = setup(&env);

    let fn_name = Symbol::new(&env, "set_value");
    let args: Vec<Val> = vec![&env, 1_u32.into_val(&env)];
    let salt: BytesN<32> = BytesN::from_array(&env, &[5u8; 32]);

    // Delay 100 is less than min_delay 86_400
    ctx.timelock.queue(
        &ctx.proposer,
        &ctx.target_id,
        &fn_name,
        &args,
        &100,
        &salt,
    );
}

#[test]
#[should_panic(expected = "Error(Contract, #3)")]
fn test_unauthorized_proposer_rejected() {
    let env = Env::default();
    let ctx = setup(&env);

    let fn_name = Symbol::new(&env, "set_value");
    let args: Vec<Val> = vec![&env, 1_u32.into_val(&env)];
    let salt: BytesN<32> = BytesN::from_array(&env, &[6u8; 32]);

    // Mallory is not a proposer
    ctx.timelock.queue(
        &ctx.mallory,
        &ctx.target_id,
        &fn_name,
        &args,
        &ctx.min_delay,
        &salt,
    );
}

#[test]
#[should_panic(expected = "Error(Contract, #3)")]
fn test_unauthorized_executor_rejected() {
    let env = Env::default();
    let ctx = setup(&env);

    let start_ts = 1_000_000_u64;
    env.ledger().with_mut(|li| li.timestamp = start_ts);

    let fn_name = Symbol::new(&env, "set_value");
    let args: Vec<Val> = vec![&env, 1_u32.into_val(&env)];
    let salt: BytesN<32> = BytesN::from_array(&env, &[7u8; 32]);

    ctx.timelock.queue(
        &ctx.proposer,
        &ctx.target_id,
        &fn_name,
        &args,
        &ctx.min_delay,
        &salt,
    );

    env.ledger()
        .with_mut(|li| li.timestamp = start_ts + ctx.min_delay);

    // Mallory is not an executor
    ctx.timelock.execute(
        &ctx.mallory,
        &ctx.target_id,
        &fn_name,
        &args,
        &salt,
    );
}

#[test]
fn test_role_management_and_min_delay_update() {
    let env = Env::default();
    let ctx = setup(&env);

    let new_proposer = Address::generate(&env);
    assert!(!ctx.timelock.has_role(&Role::Proposer, &new_proposer));

    ctx.timelock
        .grant_role(&ctx.admin, &Role::Proposer, &new_proposer);
    assert!(ctx.timelock.has_role(&Role::Proposer, &new_proposer));

    ctx.timelock
        .revoke_role(&ctx.admin, &Role::Proposer, &new_proposer);
    assert!(!ctx.timelock.has_role(&Role::Proposer, &new_proposer));

    // Update min delay
    ctx.timelock.set_min_delay(&ctx.admin, &172_800);
    assert_eq!(ctx.timelock.get_min_delay(), 172_800);
}

#[test]
fn test_admin_handover_propose_and_accept() {
    let env = Env::default();
    let ctx = setup(&env);
    let new_admin = Address::generate(&env);

    ctx.timelock.propose_admin(&ctx.admin, &new_admin);
    assert_eq!(ctx.timelock.get_pending_admin(), Some(new_admin.clone()));

    ctx.timelock.accept_admin(&new_admin);
    assert_eq!(ctx.timelock.get_pending_admin(), None);
    assert!(ctx.timelock.has_role(&Role::Admin, &new_admin));
    assert!(!ctx.timelock.has_role(&Role::Admin, &ctx.admin));
}

#[test]
fn test_admin_handover_cancel() {
    let env = Env::default();
    let ctx = setup(&env);
    let new_admin = Address::generate(&env);

    ctx.timelock.propose_admin(&ctx.admin, &new_admin);
    ctx.timelock.cancel_admin_proposal(&ctx.admin);
    assert_eq!(ctx.timelock.get_pending_admin(), None);
    assert!(ctx.timelock.has_role(&Role::Admin, &ctx.admin));

    assert_eq!(
        ctx.timelock.try_accept_admin(&new_admin),
        Err(Ok(TimelockError::NoPendingAdmin))
    );
}

#[test]
fn test_admin_handover_rejects_non_pending_accept() {
    let env = Env::default();
    let ctx = setup(&env);
    let new_admin = Address::generate(&env);

    ctx.timelock.propose_admin(&ctx.admin, &new_admin);
    assert_eq!(
        ctx.timelock.try_accept_admin(&ctx.mallory),
        Err(Ok(TimelockError::Unauthorized))
    );
    assert!(ctx.timelock.has_role(&Role::Admin, &ctx.admin));
}

#[test]
fn test_admin_handover_propose_requires_admin() {
    let env = Env::default();
    let ctx = setup(&env);

    assert_eq!(
        ctx.timelock.try_propose_admin(&ctx.mallory, &ctx.mallory),
        Err(Ok(TimelockError::Unauthorized))
    );
}

#[test]
fn test_storage_survives_past_the_default_ttl() {
    use soroban_sdk::testutils::storage::{Instance as _, Persistent as _};

    let env = Env::default();
    let ctx = setup(&env);
    let default_ttl = env.ledger().get().min_persistent_entry_ttl;

    let start_ts = 1_000_000_u64;
    env.ledger().with_mut(|li| li.timestamp = start_ts);

    let fn_name = Symbol::new(&env, "set_value");
    let args: Vec<Val> = vec![&env, 7_u32.into_val(&env)];
    let salt: BytesN<32> = BytesN::from_array(&env, &[9u8; 32]);
    let op_id = ctx.timelock.queue(
        &ctx.proposer,
        &ctx.target_id,
        &fn_name,
        &args,
        &ctx.min_delay,
        &salt,
    );

    let instance_ttl = || env.as_contract(&ctx.contract_id, || env.storage().instance().get_ttl());
    let persistent_ttl = |key: DataKey| {
        env.as_contract(&ctx.contract_id, || env.storage().persistent().get_ttl(&key))
    };

    // Inspect before advancing so recording-mode auto-restore cannot mask an expired entry.
    assert!(instance_ttl() > default_ttl);
    assert!(persistent_ttl(DataKey::Role(Role::Admin, ctx.admin.clone())) > default_ttl);
    assert!(persistent_ttl(DataKey::Role(Role::Proposer, ctx.proposer.clone())) > default_ttl);
    assert!(persistent_ttl(DataKey::Role(Role::Executor, ctx.executor.clone())) > default_ttl);
    assert!(persistent_ttl(DataKey::Op(op_id.clone())) > default_ttl);

    // The mock target is test scaffolding, not part of the timelock; keep it alive.
    env.as_contract(&ctx.target_id, || {
        env.storage()
            .instance()
            .extend_ttl(PERSISTENT_TTL_THRESHOLD, PERSISTENT_TTL_EXTEND_TO)
    });

    env.ledger().with_mut(|li| {
        li.sequence_number += default_ttl + 1;
        li.timestamp = start_ts + ctx.min_delay;
    });

    assert_eq!(ctx.timelock.get_min_delay(), ctx.min_delay);
    assert!(ctx.timelock.has_role(&Role::Executor, &ctx.executor));
    assert_eq!(
        ctx.timelock.get_operation_state(&op_id),
        OperationStatus::Ready
    );

    ctx.timelock
        .execute(&ctx.executor, &ctx.target_id, &fn_name, &args, &salt);
    assert_eq!(ctx.target.get_value(), 7);
    assert_eq!(
        ctx.timelock.get_operation_state(&op_id),
        OperationStatus::Executed
    );
    assert!(instance_ttl() >= INSTANCE_TTL_THRESHOLD);
    assert!(persistent_ttl(DataKey::Op(op_id)) >= PERSISTENT_TTL_THRESHOLD);
}

#[test]
fn test_admin_handover_publishes_contractevents() {
    use soroban_sdk::testutils::Events as _;
    let env = Env::default();
    let ctx = setup(&env);
    let new_admin = Address::generate(&env);

    ctx.timelock.propose_admin(&ctx.admin, &new_admin);
    let expected = (
        ctx.contract_id.clone(),
        (symbol_short!("adm_prop"), ctx.admin.clone()).into_val(&env),
        new_admin.clone().into_val(&env),
    );
    assert_eq!(env.events().all(), vec![&env, expected]);

    ctx.timelock.cancel_admin_proposal(&ctx.admin);
    let expected = (
        ctx.contract_id.clone(),
        (symbol_short!("adm_canc"),).into_val(&env),
        ctx.admin.clone().into_val(&env),
    );
    assert_eq!(env.events().all(), vec![&env, expected]);

    ctx.timelock.propose_admin(&ctx.admin, &new_admin);
    ctx.timelock.accept_admin(&new_admin);
    let expected = (
        ctx.contract_id.clone(),
        (symbol_short!("adm_acc"), ctx.admin.clone()).into_val(&env),
        new_admin.clone().into_val(&env),
    );
    assert_eq!(env.events().all(), vec![&env, expected]);
}

#[test]
fn test_operation_lifecycle_publishes_contractevents() {
    use soroban_sdk::testutils::Events as _;
    let env = Env::default();
    let ctx = setup(&env);
    let start_ts = 1_000_000_u64;
    env.ledger().with_mut(|li| li.timestamp = start_ts);

    let fn_name = Symbol::new(&env, "set_value");
    let args: Vec<Val> = vec![&env, 7_u32.into_val(&env)];
    let salt: BytesN<32> = BytesN::from_array(&env, &[9u8; 32]);

    let op_id = ctx.timelock.queue(
        &ctx.proposer,
        &ctx.target_id,
        &fn_name,
        &args,
        &ctx.min_delay,
        &salt,
    );
    let expected = (
        ctx.contract_id.clone(),
        (symbol_short!("queue"), op_id.clone()).into_val(&env),
        (start_ts + ctx.min_delay, ctx.target_id.clone()).into_val(&env),
    );
    assert_eq!(env.events().all(), vec![&env, expected]);

    env.ledger()
        .with_mut(|li| li.timestamp = start_ts + ctx.min_delay);
    let _ = ctx
        .timelock
        .execute(&ctx.executor, &ctx.target_id, &fn_name, &args, &salt);
    let expected = (
        ctx.contract_id.clone(),
        (symbol_short!("exec"), op_id.clone()).into_val(&env),
        ctx.target_id.clone().into_val(&env),
    );
    assert_eq!(env.events().all(), vec![&env, expected]);

    let salt2: BytesN<32> = BytesN::from_array(&env, &[10u8; 32]);
    let op2 = ctx.timelock.queue(
        &ctx.proposer,
        &ctx.target_id,
        &fn_name,
        &args,
        &ctx.min_delay,
        &salt2,
    );
    ctx.timelock.cancel(&ctx.proposer, &op2);
    let expected = (
        ctx.contract_id.clone(),
        (symbol_short!("cancel"), op2).into_val(&env),
        ctx.proposer.clone().into_val(&env),
    );
    assert_eq!(env.events().all(), vec![&env, expected]);
}
