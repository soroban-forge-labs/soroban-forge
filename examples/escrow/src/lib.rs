#![no_std]

use soroban_sdk::{contract, contractimpl, contracttype, token, Address, Env};

/// State of the escrow.
#[contracttype]
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum EscrowState {
    /// Awaiting deposit.
    Pending,
    /// Funds have been deposited and are held.
    Funded,
    /// Funds were released to the recipient.
    Released,
    /// Funds were refunded to the depositor.
    Refunded,
}

#[contracttype]
pub enum DataKey {
    Depositor,
    Recipient,
    Approver,
    TokenId,
    Amount,
    Timeout,
    State,
}

#[contract]
pub struct EscrowContract;

#[contractimpl]
impl EscrowContract {
    /// Initialize the escrow.
    ///
    /// * `depositor`  — address that will deposit the funds
    /// * `recipient`  — address that receives funds on successful release
    /// * `approver`   — address authorized to call `approve_release`
    /// * `token_id`   — contract address of the SEP-41 token held in escrow
    /// * `amount`     — token amount to be escrowed
    /// * `timeout`    — absolute ledger sequence after which the depositor may
    ///                  claim a refund via `refund_on_timeout`
    pub fn initialize(
        env: Env,
        depositor: Address,
        recipient: Address,
        approver: Address,
        token_id: Address,
        amount: i128,
        timeout: u32,
    ) {
        depositor.require_auth();
        if env.storage().instance().has(&DataKey::Depositor) {
            panic!("already initialized");
        }
        env.storage().instance().set(&DataKey::Depositor, &depositor);
        env.storage().instance().set(&DataKey::Recipient, &recipient);
        env.storage().instance().set(&DataKey::Approver, &approver);
        env.storage().instance().set(&DataKey::TokenId, &token_id);
        env.storage().instance().set(&DataKey::Amount, &amount);
        env.storage().instance().set(&DataKey::Timeout, &timeout);
        env.storage()
            .instance()
            .set(&DataKey::State, &EscrowState::Pending);
    }

    /// Deposit the escrowed amount into this contract.
    ///
    /// Must be called by the depositor after `initialize`.
    pub fn deposit(env: Env) {
        let state: EscrowState = env
            .storage()
            .instance()
            .get(&DataKey::State)
            .expect("escrow not initialized");
        if state != EscrowState::Pending {
            panic!("deposit can only be made in Pending state");
        }

        let depositor: Address = env.storage().instance().get(&DataKey::Depositor).unwrap();
        depositor.require_auth();

        let token_id: Address = env.storage().instance().get(&DataKey::TokenId).unwrap();
        let amount: i128 = env.storage().instance().get(&DataKey::Amount).unwrap();

        let token = token::Client::new(&env, &token_id);
        token.transfer(&depositor, &env.current_contract_address(), &amount);

        env.storage()
            .instance()
            .set(&DataKey::State, &EscrowState::Funded);
    }

    /// Release funds to the recipient.
    ///
    /// Must be called by the approver while the escrow is in `Funded` state.
    pub fn approve_release(env: Env) {
        let state: EscrowState = env
            .storage()
            .instance()
            .get(&DataKey::State)
            .expect("escrow not initialized");
        if state != EscrowState::Funded {
            panic!("funds not deposited yet");
        }

        let approver: Address = env.storage().instance().get(&DataKey::Approver).unwrap();
        approver.require_auth();

        let token_id: Address = env.storage().instance().get(&DataKey::TokenId).unwrap();
        let amount: i128 = env.storage().instance().get(&DataKey::Amount).unwrap();
        let recipient: Address = env.storage().instance().get(&DataKey::Recipient).unwrap();

        let token = token::Client::new(&env, &token_id);
        token.transfer(&env.current_contract_address(), &recipient, &amount);

        env.storage()
            .instance()
            .set(&DataKey::State, &EscrowState::Released);
    }

    /// Refund the depositor after the timeout has passed.
    ///
    /// Anyone may call this, but the funds always go back to the depositor.
    pub fn refund_on_timeout(env: Env) {
        let state: EscrowState = env
            .storage()
            .instance()
            .get(&DataKey::State)
            .expect("escrow not initialized");
        if state != EscrowState::Funded {
            panic!("funds not deposited or already settled");
        }

        let timeout: u32 = env.storage().instance().get(&DataKey::Timeout).unwrap();
        if env.ledger().sequence() <= timeout {
            panic!("timeout has not been reached yet");
        }

        let token_id: Address = env.storage().instance().get(&DataKey::TokenId).unwrap();
        let amount: i128 = env.storage().instance().get(&DataKey::Amount).unwrap();
        let depositor: Address = env.storage().instance().get(&DataKey::Depositor).unwrap();

        let token = token::Client::new(&env, &token_id);
        token.transfer(&env.current_contract_address(), &depositor, &amount);

        env.storage()
            .instance()
            .set(&DataKey::State, &EscrowState::Refunded);
    }

    /// Return the current escrow state.
    pub fn get_state(env: Env) -> EscrowState {
        env.storage()
            .instance()
            .get(&DataKey::State)
            .expect("escrow not initialized")
    }
}

#[cfg(test)]
mod test;
