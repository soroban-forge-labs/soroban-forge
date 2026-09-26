#![cfg(test)]

use super::*;
use ed25519_dalek::{Signer, SigningKey};
use rand::thread_rng;
use soroban_sdk::auth::ContractContext;
use soroban_sdk::testutils::BytesN as _;
extern crate std;

use soroban_sdk::{symbol_short, vec, Address, IntoVal};

fn generate_keys(env: &Env, n: usize) -> Vec<BytesN<32>> {
    // Keys sorted by public key so tests can build correctly ordered
    // signature vectors easily.
    let mut raw: std::vec::Vec<SigningKey> = (0..n)
        .map(|_| SigningKey::generate(&mut thread_rng()))
        .collect();
    raw.sort_by_key(|k| k.verifying_key().to_bytes());
    KEYS.with(|k| *k.borrow_mut() = raw.clone());
    let mut out = Vec::new(env);
    for k in &raw {
        out.push_back(BytesN::from_array(env, &k.verifying_key().to_bytes()));
    }
    out
}

std::thread_local! {
    static KEYS: std::cell::RefCell<std::vec::Vec<SigningKey>> =
        std::cell::RefCell::new(std::vec::Vec::new());
}

fn sign(env: &Env, key_index: usize, payload: &BytesN<32>) -> AccSignature {
    KEYS.with(|k| {
        let keys = k.borrow();
        let key = &keys[key_index];
        AccSignature {
            public_key: BytesN::from_array(env, &key.verifying_key().to_bytes()),
            signature: BytesN::from_array(
                env,
                &key.sign(payload.to_array().as_slice()).to_bytes(),
            ),
        }
    })
}

fn token_transfer_context(env: &Env) -> Context {
    Context::Contract(ContractContext {
        contract: Address::generate(env),
        fn_name: symbol_short!("transfer"),
        args: ((), (), 100_i128).into_val(env),
    })
}

fn self_admin_context(env: &Env, account: &Address) -> Context {
    Context::Contract(ContractContext {
        contract: account.clone(),
        fn_name: symbol_short!("set_thres"),
        args: (2u32,).into_val(env),
    })
}

fn setup(env: &Env, n: usize, threshold: u32) -> Address {
    let signers = generate_keys(env, n);
    env.register(MultisigContract, (signers, threshold))
}

use soroban_sdk::testutils::Address as _;

#[test]
fn meets_threshold_passes() {
    let env = Env::default();
    let account = setup(&env, 3, 2);
    let payload = BytesN::random(&env);
    let signatures = vec![&env, sign(&env, 0, &payload), sign(&env, 1, &payload)];
    env.try_invoke_contract_check_auth::<AccError>(
        &account,
        &payload,
        signatures.into_val(&env),
        &vec![&env, token_transfer_context(&env)],
    )
    .unwrap();
}

#[test]
fn below_threshold_fails() {
    let env = Env::default();
    let account = setup(&env, 3, 2);
    let payload = BytesN::random(&env);
    let signatures = vec![&env, sign(&env, 0, &payload)];
    assert_eq!(
        env.try_invoke_contract_check_auth::<AccError>(
            &account,
            &payload,
            signatures.into_val(&env),
            &vec![&env, token_transfer_context(&env)],
        )
        .err()
        .unwrap()
        .unwrap(),
        AccError::NotEnoughSigners
    );
}

#[test]
fn unknown_signer_fails() {
    let env = Env::default();
    let account = setup(&env, 2, 1);
    // Overwrite the key registry with a stranger's key.
    let _other_keys = generate_keys(&env, 1);
    let payload = BytesN::random(&env);
    let signatures = vec![&env, sign(&env, 0, &payload)];
    assert_eq!(
        env.try_invoke_contract_check_auth::<AccError>(
            &account,
            &payload,
            signatures.into_val(&env),
            &vec![&env, token_transfer_context(&env)],
        )
        .err()
        .unwrap()
        .unwrap(),
        AccError::UnknownSigner
    );
}

#[test]
fn wrong_signature_order_fails() {
    let env = Env::default();
    let account = setup(&env, 3, 2);
    let payload = BytesN::random(&env);
    // Reversed order: keys are sorted ascending, so [1, 0] violates it.
    let signatures = vec![&env, sign(&env, 1, &payload), sign(&env, 0, &payload)];
    assert_eq!(
        env.try_invoke_contract_check_auth::<AccError>(
            &account,
            &payload,
            signatures.into_val(&env),
            &vec![&env, token_transfer_context(&env)],
        )
        .err()
        .unwrap()
        .unwrap(),
        AccError::BadSignatureOrder
    );
}

#[test]
fn self_admin_needs_all_signers() {
    let env = Env::default();
    let account = setup(&env, 3, 2);
    let payload = BytesN::random(&env);

    // Threshold (2 of 3) is NOT enough for the account's own functions.
    let two = vec![&env, sign(&env, 0, &payload), sign(&env, 1, &payload)];
    assert_eq!(
        env.try_invoke_contract_check_auth::<AccError>(
            &account,
            &payload,
            two.into_val(&env),
            &vec![&env, self_admin_context(&env, &account)],
        )
        .err()
        .unwrap()
        .unwrap(),
        AccError::NotEnoughSigners
    );

    // All three signers pass.
    let all = vec![
        &env,
        sign(&env, 0, &payload),
        sign(&env, 1, &payload),
        sign(&env, 2, &payload),
    ];
    env.try_invoke_contract_check_auth::<AccError>(
        &account,
        &payload,
        all.into_val(&env),
        &vec![&env, self_admin_context(&env, &account)],
    )
    .unwrap();
}

#[test]
#[should_panic]
fn zero_threshold_rejected() {
    let env = Env::default();
    setup(&env, 2, 0);
}

#[test]
#[should_panic]
fn threshold_above_signer_count_rejected() {
    let env = Env::default();
    setup(&env, 2, 3);
}

#[test]
fn threshold_getter_works() {
    let env = Env::default();
    let account = setup(&env, 3, 2);
    let client = MultisigContractClient::new(&env, &account);
    assert_eq!(client.threshold(), 2);
}
