extern crate std;

use super::*;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{token::StellarAssetClient, token::TokenClient, vec, Address, Env};

/// Amounts in the allowlist, one per generated claimant.
const AMOUNTS: [i128; 4] = [100, 200, 300, 400];
const TOTAL: i128 = 1_000;

struct Tok<'a> {
    address: Address,
    client: TokenClient<'a>,
    admin: StellarAssetClient<'a>,
}

fn make_token(env: &Env) -> Tok<'_> {
    let issuer = Address::generate(env);
    let sac = env.register_stellar_asset_contract_v2(issuer);
    let address = sac.address();
    Tok {
        client: TokenClient::new(env, &address),
        admin: StellarAssetClient::new(env, &address),
        address,
    }
}

/// A four-leaf merkle tree over `(claimant, amount)` entries.
///
/// Built with the contract's *own* `leaf` and `hash_pair` (run inside the
/// contract's frame via `as_contract`), so the test can never drift from the
/// hashing rules it is meant to exercise — which is exactly what an off-chain
/// tree builder has to reproduce.
struct Tree {
    root: BytesN<32>,
    leaves: std::vec::Vec<BytesN<32>>,
    n01: BytesN<32>,
    n23: BytesN<32>,
}

impl Tree {
    fn build(env: &Env, contract_id: &Address, entries: &[(Address, i128)]) -> Tree {
        assert_eq!(entries.len(), 4, "this helper builds a four-leaf tree");
        env.as_contract(contract_id, || {
            let leaves: std::vec::Vec<BytesN<32>> = entries
                .iter()
                .map(|(claimant, amount)| {
                    MerkleAirdropContract::leaf(env.clone(), claimant.clone(), *amount)
                })
                .collect();
            let n01 = hash_pair(env, &leaves[0], &leaves[1]);
            let n23 = hash_pair(env, &leaves[2], &leaves[3]);
            let root = hash_pair(env, &n01, &n23);
            Tree {
                root,
                leaves,
                n01,
                n23,
            }
        })
    }

    /// Sibling hashes from leaf `index` up to the root.
    fn proof(&self, env: &Env, index: usize) -> Vec<BytesN<32>> {
        let sibling = self.leaves[index ^ 1].clone();
        let uncle = if index < 2 {
            self.n23.clone()
        } else {
            self.n01.clone()
        };
        vec![env, sibling, uncle]
    }
}

struct Fixture<'a> {
    contract: MerkleAirdropContractClient<'a>,
    contract_id: Address,
    token: Tok<'a>,
    admin: Address,
    claimants: std::vec::Vec<Address>,
    tree: Tree,
}

fn setup(env: &Env) -> Fixture<'_> {
    env.mock_all_auths();

    let token = make_token(env);
    let admin = Address::generate(env);
    token.admin.mint(&admin, &TOTAL);

    let claimants: std::vec::Vec<Address> =
        (0..AMOUNTS.len()).map(|_| Address::generate(env)).collect();
    let entries: std::vec::Vec<(Address, i128)> = claimants
        .iter()
        .cloned()
        .zip(AMOUNTS.iter().copied())
        .collect();

    let contract_id = env.register(MerkleAirdropContract, ());
    let contract = MerkleAirdropContractClient::new(env, &contract_id);

    let tree = Tree::build(env, &contract_id, &entries);
    contract.initialize(&admin, &token.address, &tree.root);
    contract.fund(&TOTAL);

    Fixture {
        contract,
        contract_id,
        token,
        admin,
        claimants,
        tree,
    }
}

#[test]
fn eligible_address_claims_its_amount() {
    let env = Env::default();
    let f = setup(&env);
    let claimant = f.claimants[0].clone();
    let proof = f.tree.proof(&env, 0);

    assert!(!f.contract.has_claimed(&claimant));
    assert!(f.contract.verify(&claimant, &AMOUNTS[0], &proof));

    assert_eq!(f.contract.claim(&claimant, &AMOUNTS[0], &proof), AMOUNTS[0]);

    assert_eq!(f.token.client.balance(&claimant), AMOUNTS[0]);
    assert!(f.contract.has_claimed(&claimant));
    assert_eq!(f.contract.claimed_amount(&claimant), AMOUNTS[0]);
    assert_eq!(f.token.client.balance(&f.contract_id), TOTAL - AMOUNTS[0]);
}

#[test]
fn every_eligible_address_can_claim_exactly_its_entry() {
    let env = Env::default();
    let f = setup(&env);

    for (index, amount) in AMOUNTS.iter().copied().enumerate() {
        let claimant = f.claimants[index].clone();
        let proof = f.tree.proof(&env, index);
        assert_eq!(f.contract.claim(&claimant, &amount, &proof), amount);
        assert_eq!(f.token.client.balance(&claimant), amount);
    }

    // The allowlist sums to exactly the funded amount.
    assert_eq!(f.token.client.balance(&f.contract_id), 0);
}

#[test]
#[should_panic(expected = "already claimed")]
fn claiming_twice_is_rejected() {
    let env = Env::default();
    let f = setup(&env);
    let claimant = f.claimants[1].clone();
    let proof = f.tree.proof(&env, 1);

    f.contract.claim(&claimant, &AMOUNTS[1], &proof);
    f.contract.claim(&claimant, &AMOUNTS[1], &proof);
}

#[test]
#[should_panic(expected = "invalid proof")]
fn a_proof_from_another_leaf_is_rejected() {
    let env = Env::default();
    let f = setup(&env);
    let claimant = f.claimants[0].clone();
    // Leaf 2's proof does not reconstruct the root from leaf 0.
    let wrong_proof = f.tree.proof(&env, 2);

    f.contract.claim(&claimant, &AMOUNTS[0], &wrong_proof);
}

#[test]
#[should_panic(expected = "invalid proof")]
fn inflating_the_amount_invalidates_the_proof() {
    let env = Env::default();
    let f = setup(&env);
    let claimant = f.claimants[0].clone();
    let proof = f.tree.proof(&env, 0);

    // The amount is hashed into the leaf, so asking for more breaks the proof.
    f.contract.claim(&claimant, &(AMOUNTS[0] + 1), &proof);
}

#[test]
fn an_address_outside_the_allowlist_is_not_eligible() {
    let env = Env::default();
    let f = setup(&env);
    let stranger = Address::generate(&env);
    let proof = f.tree.proof(&env, 0);

    assert!(!f.contract.verify(&stranger, &AMOUNTS[0], &proof));
    assert!(!f.contract.has_claimed(&stranger));
}

#[test]
fn admin_sweeps_what_is_left_after_the_window() {
    let env = Env::default();
    let f = setup(&env);
    let claimant = f.claimants[0].clone();
    let proof = f.tree.proof(&env, 0);
    f.contract.claim(&claimant, &AMOUNTS[0], &proof);

    let left = TOTAL - AMOUNTS[0];
    assert_eq!(f.contract.sweep(), left);
    assert_eq!(f.token.client.balance(&f.admin), left);
    assert_eq!(f.token.client.balance(&f.contract_id), 0);
}
