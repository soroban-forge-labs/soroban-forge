#![cfg(test)]

use super::*;
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    Address, Env,
};

// Minimal SEP-41 token mock for testing.
mod token_contract {
    soroban_sdk::contractimport!(
        file = "../target/wasm32v1-none/release/soroban_token_contract.wasm"
    );
}

/// Helper: set up a token, mint `amount` to `depositor`, and wire up escrow.
fn setup(
    env: &Env,
    depositor: &Address,
    recipient: &Address,
    approver: &Address,
    amount: i128,
    timeout: u32,
) -> (EscrowContractClient<'static>, Address) {
    env.mock_all_auths();

    let token_admin = Address::generate(env);
    let token_id = env.register_contract_wasm(None, token_contract::WASM);
    let token = token_contract::Client::new(env, &token_id);
    token.initialize(&token_admin, &7, &"TEST".into(), &"Test Token".into());
    token.mint(depositor, &amount);

    let escrow_id = env.register_contract(None, EscrowContract);
    let escrow = EscrowContractClient::new(env, &escrow_id);
    escrow.initialize(depositor, recipient, approver, &token_id, &amount, &timeout);
    (escrow, token_id)
}

#[test]
fn deposit_and_release() {
    let env = Env::default();
    let depositor = Address::generate(&env);
    let recipient = Address::generate(&env);
    let approver = Address::generate(&env);

    let (escrow, token_id) = setup(&env, &depositor, &recipient, &approver, 500, 1000);
    escrow.deposit();
    assert_eq!(escrow.get_state(), EscrowState::Funded);

    escrow.approve_release();
    assert_eq!(escrow.get_state(), EscrowState::Released);

    let token = token_contract::Client::new(&env, &token_id);
    assert_eq!(token.balance(&recipient), 500);
}

#[test]
fn refund_after_timeout() {
    let env = Env::default();
    let depositor = Address::generate(&env);
    let recipient = Address::generate(&env);
    let approver = Address::generate(&env);

    let (escrow, token_id) = setup(&env, &depositor, &recipient, &approver, 300, 100);
    escrow.deposit();

    env.ledger().with_mut(|li| li.sequence_number = 101);
    escrow.refund_on_timeout();
    assert_eq!(escrow.get_state(), EscrowState::Refunded);

    let token = token_contract::Client::new(&env, &token_id);
    assert_eq!(token.balance(&depositor), 300);
}

#[test]
#[should_panic(expected = "timeout has not been reached yet")]
fn refund_before_timeout_fails() {
    let env = Env::default();
    let depositor = Address::generate(&env);
    let recipient = Address::generate(&env);
    let approver = Address::generate(&env);

    let (escrow, _) = setup(&env, &depositor, &recipient, &approver, 100, 1000);
    escrow.deposit();
    escrow.refund_on_timeout(); // should panic — timeout not reached
}
