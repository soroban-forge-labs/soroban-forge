//! An M-of-N multisig account contract.
//!
//! Based on the official custom-account example in stellar/soroban-examples.
//! The contract stores a set of ed25519 signer public keys and a threshold
//! `M`. Any `require_auth` for this contract's address passes when at least
//! `M` valid, correctly ordered signatures over the signature payload are
//! provided. Operations on the account contract itself (such as
//! `set_threshold`) additionally require signatures from *every* signer.
#![no_std]

use soroban_sdk::{
    auth::{Context, CustomAccountInterface},
    contract, contracterror, contractimpl, contracttype,
    crypto::Hash,
    BytesN, Env, Vec,
};

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum AccError {
    NotEnoughSigners = 1,
    UnknownSigner = 2,
    BadSignatureOrder = 3,
    InvalidThreshold = 4,
    DuplicateSigner = 5,
}

#[contracttype]
#[derive(Clone)]
enum DataKey {
    /// Number of configured signers (`N`).
    SignerCnt,
    /// Required number of signatures (`M`).
    Threshold,
    /// Marker entry per signer public key.
    Signer(BytesN<32>),
}

/// One signature over the signature payload, paired with the signing key.
#[contracttype]
#[derive(Clone)]
pub struct AccSignature {
    pub public_key: BytesN<32>,
    pub signature: BytesN<64>,
}

#[contract]
pub struct MultisigContract;

#[contractimpl]
impl MultisigContract {
    /// Initialize the account with ed25519 public keys and a threshold.
    ///
    /// A constructor makes creation and initialization atomic, so nobody can
    /// front-run the setup with their own keys.
    pub fn __constructor(env: Env, signers: Vec<BytesN<32>>, threshold: u32) {
        if threshold == 0 || threshold > signers.len() {
            panic_with_error(&env, AccError::InvalidThreshold);
        }
        for signer in signers.iter() {
            if env.storage().instance().has(&DataKey::Signer(signer.clone())) {
                panic_with_error(&env, AccError::DuplicateSigner);
            }
            env.storage().instance().set(&DataKey::Signer(signer), &());
        }
        env.storage()
            .instance()
            .set(&DataKey::SignerCnt, &signers.len());
        env.storage().instance().set(&DataKey::Threshold, &threshold);
    }

    /// Change the signature threshold.
    ///
    /// `require_auth` on the contract's own address routes back through
    /// `__check_auth`, which demands signatures from every signer for
    /// operations on the account itself — no duplicate auth logic needed.
    pub fn set_threshold(env: Env, threshold: u32) {
        env.current_contract_address().require_auth();
        let signer_cnt: u32 = env
            .storage()
            .instance()
            .get(&DataKey::SignerCnt)
            .unwrap();
        if threshold == 0 || threshold > signer_cnt {
            panic_with_error(&env, AccError::InvalidThreshold);
        }
        env.storage().instance().set(&DataKey::Threshold, &threshold);
    }

    /// Current threshold (`M`).
    pub fn threshold(env: Env) -> u32 {
        env.storage().instance().get(&DataKey::Threshold).unwrap()
    }
}

#[contractimpl]
impl CustomAccountInterface for MultisigContract {
    type Signature = Vec<AccSignature>;
    type Error = AccError;

    /// Called by the Soroban host for every `require_auth` on this
    /// contract's address. Never callable directly.
    #[allow(non_snake_case)]
    fn __check_auth(
        env: Env,
        signature_payload: Hash<32>,
        signatures: Vec<AccSignature>,
        auth_contexts: Vec<Context>,
    ) -> Result<(), AccError> {
        // 1. Authentication: every provided signature must be valid, from a
        //    known signer, in strictly ascending key order (cheap dedup).
        authenticate(&env, &signature_payload, &signatures)?;

        let signer_cnt: u32 = env
            .storage()
            .instance()
            .get(&DataKey::SignerCnt)
            .unwrap();
        let threshold: u32 = env.storage().instance().get(&DataKey::Threshold).unwrap();
        let all_signed = signatures.len() == signer_cnt;

        // 2. Authorization policy.
        let curr_contract = env.current_contract_address();
        for context in auth_contexts.iter() {
            let needs_all = match &context {
                // Operations on the account contract itself need every signer.
                Context::Contract(c) => c.contract == curr_contract,
                // So does deploying new contracts on the account's behalf.
                Context::CreateContractHostFn(_)
                | Context::CreateContractWithCtorHostFn(_) => true,
            };
            if needs_all && !all_signed {
                return Err(AccError::NotEnoughSigners);
            }
        }
        if signatures.len() < threshold {
            return Err(AccError::NotEnoughSigners);
        }
        Ok(())
    }
}

fn authenticate(
    env: &Env,
    signature_payload: &Hash<32>,
    signatures: &Vec<AccSignature>,
) -> Result<(), AccError> {
    for i in 0..signatures.len() {
        let signature = signatures.get_unchecked(i);
        if i > 0 {
            let prev = signatures.get_unchecked(i - 1);
            if prev.public_key >= signature.public_key {
                return Err(AccError::BadSignatureOrder);
            }
        }
        if !env
            .storage()
            .instance()
            .has(&DataKey::Signer(signature.public_key.clone()))
        {
            return Err(AccError::UnknownSigner);
        }
        env.crypto().ed25519_verify(
            &signature.public_key,
            &signature_payload.clone().into(),
            &signature.signature,
        );
    }
    Ok(())
}

fn panic_with_error(env: &Env, err: AccError) -> ! {
    soroban_sdk::panic_with_error!(env, err)
}

#[cfg(test)]
mod test;
