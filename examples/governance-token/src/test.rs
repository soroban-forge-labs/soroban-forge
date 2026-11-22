use super::*;
use soroban_sdk::testutils::{Address as _, Ledger};
use soroban_sdk::{Address, Env, String};

fn setup(env: &Env) -> (GovernanceTokenContractClient<'_>, Address) {
    env.mock_all_auths();
    let admin = Address::generate(env);
    let contract_id = env.register(
        GovernanceTokenContract,
        (
            admin.clone(),
            7u32,
            String::from_str(env, "Governance Token"),
            String::from_str(env, "GOV"),
        ),
    );
    (
        GovernanceTokenContractClient::new(env, &contract_id),
        admin,
    )
}

#[test]
fn metadata_and_initial_state() {
    let env = Env::default();
    let (token, admin) = setup(&env);
    let alice = Address::generate(&env);

    assert_eq!(token.decimals(), 7);
    assert_eq!(token.name(), String::from_str(&env, "Governance Token"));
    assert_eq!(token.symbol(), String::from_str(&env, "GOV"));
    assert_eq!(token.admin(), admin);
    assert_eq!(token.total_supply(), 0);
    assert_eq!(token.balance(&alice), 0);
    assert_eq!(token.delegates(&alice), None);
    assert_eq!(token.get_votes(&alice), 0);
    assert_eq!(token.num_checkpoints(&alice), 0);
}

#[test]
fn mint_transfer_and_burn_sep41() {
    let env = Env::default();
    let (token, _admin) = setup(&env);
    let alice = Address::generate(&env);
    let bob = Address::generate(&env);

    token.mint(&alice, &1_000);
    assert_eq!(token.balance(&alice), 1_000);
    assert_eq!(token.total_supply(), 1_000);

    token.transfer(&alice, &bob, &400);
    assert_eq!(token.balance(&alice), 600);
    assert_eq!(token.balance(&bob), 400);
    assert_eq!(token.total_supply(), 1_000);

    token.burn(&alice, &250);
    assert_eq!(token.balance(&alice), 350);
    assert_eq!(token.total_supply(), 750);
}

#[test]
fn approve_transfer_from_and_burn_from() {
    let env = Env::default();
    env.ledger().set_sequence_number(100);
    let (token, _admin) = setup(&env);
    let alice = Address::generate(&env);
    let spender = Address::generate(&env);
    let carol = Address::generate(&env);

    token.mint(&alice, &1_000);
    token.approve(&alice, &spender, &600, &200);
    assert_eq!(token.allowance(&alice, &spender), 600);

    token.transfer_from(&spender, &alice, &carol, &350);
    assert_eq!(token.balance(&alice), 650);
    assert_eq!(token.balance(&carol), 350);
    assert_eq!(token.allowance(&alice, &spender), 250);

    token.burn_from(&spender, &alice, &150);
    assert_eq!(token.balance(&alice), 500);
    assert_eq!(token.allowance(&alice, &spender), 100);
    assert_eq!(token.total_supply(), 850);
}

#[test]
fn self_delegation_and_representative_delegation() {
    let env = Env::default();
    env.ledger().set_sequence_number(10);
    let (token, _admin) = setup(&env);
    let alice = Address::generate(&env);
    let bob = Address::generate(&env);

    token.mint(&alice, &1_000);
    // Undelegated tokens do not count toward any delegate's voting power.
    assert_eq!(token.get_votes(&alice), 0);
    assert_eq!(token.delegates(&alice), None);

    // Self-delegation activates Alice's 1,000 votes.
    token.delegate(&alice, &alice);
    assert_eq!(token.delegates(&alice), Some(alice.clone()));
    assert_eq!(token.get_votes(&alice), 1_000);
    assert_eq!(token.num_checkpoints(&alice), 1);
    assert_eq!(
        token.checkpoints(&alice, &0),
        Checkpoint {
            ledger: 10,
            votes: 1_000
        }
    );

    // Re-delegating to Bob moves Alice's 1,000 votes to Bob at ledger 20.
    env.ledger().set_sequence_number(20);
    token.delegate(&alice, &bob);
    assert_eq!(token.delegates(&alice), Some(bob.clone()));
    assert_eq!(token.get_votes(&alice), 0);
    assert_eq!(token.get_votes(&bob), 1_000);
    assert_eq!(token.num_checkpoints(&alice), 2);
    assert_eq!(
        token.checkpoints(&alice, &1),
        Checkpoint {
            ledger: 20,
            votes: 0
        }
    );
    assert_eq!(
        token.checkpoints(&bob, &0),
        Checkpoint {
            ledger: 20,
            votes: 1_000
        }
    );
}

#[test]
fn transfers_and_burns_move_delegated_voting_power() {
    let env = Env::default();
    env.ledger().set_sequence_number(10);
    let (token, _admin) = setup(&env);
    let alice = Address::generate(&env);
    let bob = Address::generate(&env);
    let spender = Address::generate(&env);
    let dave = Address::generate(&env);
    let erin = Address::generate(&env);

    // Alice delegates to Dave; Bob delegates to Erin.
    token.delegate(&alice, &dave);
    token.delegate(&bob, &erin);

    // Minting to Alice credits Dave's voting power automatically.
    token.mint(&alice, &1_000);
    token.mint(&bob, &500);
    assert_eq!(token.get_votes(&dave), 1_000);
    assert_eq!(token.get_votes(&erin), 500);

    // Direct transfer from Alice to Bob moves 400 votes from Dave to Erin.
    env.ledger().set_sequence_number(20);
    token.transfer(&alice, &bob, &400);
    assert_eq!(token.get_votes(&dave), 600);
    assert_eq!(token.get_votes(&erin), 900);

    // `transfer_from` also moves voting power between delegates.
    env.ledger().set_sequence_number(30);
    token.approve(&bob, &spender, &300, &100);
    token.transfer_from(&spender, &bob, &alice, &200);
    assert_eq!(token.get_votes(&dave), 800);
    assert_eq!(token.get_votes(&erin), 700);

    // `burn` and `burn_from` reduce the holder's delegate's voting power.
    env.ledger().set_sequence_number(40);
    token.burn(&alice, &300);
    assert_eq!(token.get_votes(&dave), 500);

    token.burn_from(&spender, &bob, &100);
    assert_eq!(token.get_votes(&erin), 600);
}

#[test]
fn checkpoints_and_binary_search_get_past_votes() {
    let env = Env::default();
    let (token, _admin) = setup(&env);
    let alice = Address::generate(&env);
    let bob = Address::generate(&env);

    env.ledger().set_sequence_number(10);
    token.delegate(&alice, &alice);
    token.delegate(&bob, &bob);
    token.mint(&alice, &1_000); // ledger 10: alice=1000, supply=1000

    env.ledger().set_sequence_number(20);
    token.transfer(&alice, &bob, &250); // ledger 20: alice=750, bob=250, supply=1000

    env.ledger().set_sequence_number(30);
    token.mint(&alice, &500); // ledger 30: alice=1250, bob=250, supply=1500

    env.ledger().set_sequence_number(40);
    token.burn(&alice, &450); // ledger 40: alice=800, bob=250, supply=1050

    env.ledger().set_sequence_number(50);
    token.transfer(&bob, &alice, &100); // ledger 50: alice=900, bob=150, supply=1050

    env.ledger().set_sequence_number(60);

    // Before first checkpoint (ledger < 10)
    assert_eq!(token.get_past_votes(&alice, &5), 0);
    assert_eq!(token.get_past_votes(&bob, &15), 0);
    assert_eq!(token.get_past_total_supply(&5), 0);

    // Exact checkpoint ledgers and intermediate ledgers via binary search
    assert_eq!(token.get_past_votes(&alice, &10), 1_000);
    assert_eq!(token.get_past_votes(&alice, &15), 1_000);
    assert_eq!(token.get_past_votes(&alice, &20), 750);
    assert_eq!(token.get_past_votes(&alice, &25), 750);
    assert_eq!(token.get_past_votes(&alice, &30), 1_250);
    assert_eq!(token.get_past_votes(&alice, &35), 1_250);
    assert_eq!(token.get_past_votes(&alice, &40), 800);
    assert_eq!(token.get_past_votes(&alice, &45), 800);
    assert_eq!(token.get_past_votes(&alice, &50), 900);
    assert_eq!(token.get_past_votes(&alice, &60), 900);

    assert_eq!(token.get_past_votes(&bob, &20), 250);
    assert_eq!(token.get_past_votes(&bob, &49), 250);
    assert_eq!(token.get_past_votes(&bob, &50), 150);

    // Past total supply binary search
    assert_eq!(token.get_past_total_supply(&10), 1_000);
    assert_eq!(token.get_past_total_supply(&25), 1_000);
    assert_eq!(token.get_past_total_supply(&30), 1_500);
    assert_eq!(token.get_past_total_supply(&39), 1_500);
    assert_eq!(token.get_past_total_supply(&40), 1_050);
    assert_eq!(token.get_past_total_supply(&60), 1_050);
}

#[test]
fn same_ledger_checkpoint_deduplication() {
    let env = Env::default();
    env.ledger().set_sequence_number(15);
    let (token, _admin) = setup(&env);
    let alice = Address::generate(&env);
    let bob = Address::generate(&env);

    token.delegate(&alice, &alice);
    token.mint(&alice, &500);
    token.mint(&alice, &300);
    token.transfer(&alice, &bob, &100);

    // All three operations happened at ledger 15, so only 1 checkpoint exists.
    assert_eq!(token.num_checkpoints(&alice), 1);
    assert_eq!(
        token.checkpoints(&alice, &0),
        Checkpoint {
            ledger: 15,
            votes: 700
        }
    );
}

#[test]
fn error_cases_coverage() {
    let env = Env::default();
    env.ledger().set_sequence_number(50);
    let (token, _admin) = setup(&env);
    let alice = Address::generate(&env);
    let bob = Address::generate(&env);
    let spender = Address::generate(&env);

    // 1. NegativeAmount
    assert_eq!(
        token.try_mint(&alice, &-1),
        Err(Ok(Error::NegativeAmount))
    );
    assert_eq!(
        token.try_approve(&alice, &spender, &-5, &100),
        Err(Ok(Error::NegativeAmount))
    );
    assert_eq!(
        token.try_transfer(&alice, &bob, &-10),
        Err(Ok(Error::NegativeAmount))
    );
    assert_eq!(
        token.try_transfer_from(&spender, &alice, &bob, &-10),
        Err(Ok(Error::NegativeAmount))
    );
    assert_eq!(
        token.try_burn(&alice, &-10),
        Err(Ok(Error::NegativeAmount))
    );
    assert_eq!(
        token.try_burn_from(&spender, &alice, &-10),
        Err(Ok(Error::NegativeAmount))
    );

    // 2. InsufficientBalance
    token.mint(&alice, &100);
    assert_eq!(
        token.try_transfer(&alice, &bob, &101),
        Err(Ok(Error::InsufficientBalance))
    );
    assert_eq!(
        token.try_burn(&alice, &101),
        Err(Ok(Error::InsufficientBalance))
    );

    // 3. InvalidExpiration (expiration_ledger < current_ledger 50)
    assert_eq!(
        token.try_approve(&alice, &spender, &50, &49),
        Err(Ok(Error::InvalidExpiration))
    );

    // 4. InsufficientAllowance (over limit or expired)
    token.approve(&alice, &spender, &40, &55);
    assert_eq!(
        token.try_transfer_from(&spender, &alice, &bob, &41),
        Err(Ok(Error::InsufficientAllowance))
    );
    assert_eq!(
        token.try_burn_from(&spender, &alice, &41),
        Err(Ok(Error::InsufficientAllowance))
    );

    // Advance past expiration ledger 55 -> allowance becomes 0
    env.ledger().set_sequence_number(60);
    assert_eq!(token.allowance(&alice, &spender), 0);
    assert_eq!(
        token.try_transfer_from(&spender, &alice, &bob, &10),
        Err(Ok(Error::InsufficientAllowance))
    );

    // 5. FutureLookup (ledger > current_ledger 60)
    assert_eq!(
        token.try_get_past_votes(&alice, &61),
        Err(Ok(Error::FutureLookup))
    );
    assert_eq!(
        token.try_get_past_total_supply(&61),
        Err(Ok(Error::FutureLookup))
    );

    // 6. Overflow
    assert_eq!(
        token.try_mint(&alice, &i128::MAX),
        Err(Ok(Error::Overflow))
    );

    // 7. CheckpointNotFound
    assert_eq!(
        token.try_checkpoints(&alice, &0),
        Err(Ok(Error::CheckpointNotFound))
    );
}
