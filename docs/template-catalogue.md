# Template catalogue

Every template bundled under [`templates/`](../templates), generated from the CLI's own template list (`soroban-forge new --list-templates --json`) so this page cannot drift from what `soroban-forge new` actually offers.
Regenerate it after adding or changing a template:

```sh
cargo build --release --bin soroban-forge
scripts/gen-template-catalogue.py
```

| template | what it demonstrates |
|----------|----------------------|
| [`access-control`](#access-control) | role-based access control — grant/revoke/has-role with an admin role that administers other roles |
| [`allowlist-token`](#allowlist-token) | allowlist-gated token with admin-managed transfer restrictions |
| [`amm`](#amm) | constant-product AMM / liquidity pool (x*y=k, 0.3% fee) |
| [`atomic-swap`](#atomic-swap) | atomic two-party token swap with dual authorization |
| [`cross-contract`](#cross-contract) | two-contract workspace demonstrating cross-contract calls with authorization |
| [`crowdfund`](#crowdfund) | escrow/deadline crowdfunding contract |
| [`dutch-auction`](#dutch-auction) | descending-price auction with linear price decay and immediate settlement |
| [`english-auction`](#english-auction) | an ascending-price auction where bidders place successively higher bids within a fixed time window |
| [`escrow`](#escrow) | token escrow with approval or timeout-based refund path |
| [`faucet`](#faucet) | token faucet dispensing a fixed amount per address with a cooldown |
| [`flash-loan`](#flash-loan) | uncollateralized single-transaction loan repaid via a borrower callback |
| [`governance`](#governance) | DAO governance with one-member-one-vote, quorum, and proposal execution |
| [`governance-token`](#governance-token) | SEP-41 governance token with vote delegation and checkpointed past-votes lookup |
| [`hello-world`](#hello-world) | minimal greeter contract (recommended starting point) |
| [`lottery`](#lottery) | randomized lottery with ticket purchases and prize pool distribution |
| [`merkle-airdrop`](#merkle-airdrop) | one-claim-per-address airdrop verified against a merkle root |
| [`multi-token`](#multi-token) | a multi-token contract (erc1155-style) managing many token ids with per-id, per-owner balances in a single contract instance |
| [`multisig`](#multisig) | M-of-N multisig account contract (CustomAccountInterface) |
| [`nft`](#nft) | NFT (non-fungible token) with per-token metadata and minting |
| [`nft-marketplace`](#nft-marketplace) | NFT marketplace for listing, buying, and cancelling sales with configurable fees |
| [`oracle-consumer`](#oracle-consumer) | consumes price data from an external oracle (e.g. Reflector) |
| [`pausable`](#pausable) | admin-controlled circuit breaker gating guarded entrypoints |
| [`payment-splitter`](#payment-splitter) | splits received funds between payees by fixed shares |
| [`prediction-market`](#prediction-market) | binary outcome market with oracle resolution and parimutuel payouts |
| [`soulbound`](#soulbound) | soulbound (non-transferable) token contract |
| [`staking`](#staking) | proportional reward staking with O(1) acc_reward_per_share accumulator |
| [`storage-migration`](#storage-migration) | storage layout migration from v1 to v2 with version-marker pattern |
| [`streaming`](#streaming) | streams tokens linearly over time with cancels and withdrawals |
| [`subscription`](#subscription) | recurring payment charged once per elapsed interval |
| [`timelock`](#timelock) | timelock controller for delayed execution and cancellation of queued calls |
| [`token`](#token) | SEP-41 fungible token (soroban_sdk::token::TokenInterface) |
| [`upgradeable`](#upgradeable) | admin-gated upgradeable contract (update_current_contract_wasm) |
| [`vesting`](#vesting) | token vesting with cliff + linear release schedule |
| [`wrapped-asset`](#wrapped-asset) | mints a wrapper token on deposit and burns it on withdraw 1:1 |
| [`yield-vault`](#yield-vault) | ERC-4626-style yield vault with proportional shares and vault-favoured rounding |

## access-control

Role-based access control: grant, revoke and check roles, with an admin
role that can administer the other roles.

The access-control primitives are the reusable part — [`require_role`] is
the whole gate. The `mint`/`burn` ledger at the bottom of this file exists
only to give those roles something real to protect; replace it with your
own entrypoints.

**Entrypoints:** `set_fee`, `has_role`, `get_role_admin`, `role_member_count`, `grant_role`, `revoke_role`, `renounce_role`, `set_role_admin`, `mint`, `burn`, `force_burn`, `balance`, `total_supply`

**When to pick it:** you need reusable grant/revoke/has-role primitives with an admin role, to gate entrypoints you add yourself

## allowlist-token

> `templates/allowlist-token/` has no `template.toml` manifest — variables and post-generate hints for it, if any, are not declared.

An allowlist-gated token where transfers are restricted to addresses
approved by an admin.

**Entrypoints:** `add_to_allowlist`, `remove_from_allowlist`, `is_allowed`, `mint`, `transfer`, `balance`, `total_supply`

**When to pick it:** regulatory or KYC'd transfers where only admin-approved addresses may hold or move the token

## amm

> `templates/amm/` has no `template.toml` manifest — variables and post-generate hints for it, if any, are not declared.

A two-token constant-product automated market maker (`x * y = k`).

Liquidity providers `deposit` a pair of tokens and receive pool shares in
return; they `withdraw` those shares later for a proportional slice of the
reserves (plus accrued fees). Traders `swap` one token for the other at a
price set by the pool's reserves, paying a 0.3% fee that stays in the pool
and grows the constant product `k` over time.

Pool shares are tracked internally as a simple `i128` ledger — this pool
does not issue a separate share-token contract, keeping the example
self-contained. The two reserve tokens are any contracts implementing the
standard Soroban token interface (e.g. Stellar Asset Contracts).

**Entrypoints:** `deposit_with_deadline`, `deposit`, `swap_with_deadline`, `swap`, `withdraw`, `get_reserves`, `shares`, `total_shares`, `tokens`, `get_amount_in`, `get_amount_out`

**When to pick it:** a self-contained two-token constant-product pool, when you don't need a separate LP-share token contract

## atomic-swap

> `templates/atomic-swap/` has no `template.toml` manifest — variables and post-generate hints for it, if any, are not declared.

Atomic swap: exchange two token amounts between two parties in a single
authorized transaction. Both parties must authorize the swap; if either
side fails, neither transfer executes.

**Entrypoints:** `swap`

**When to pick it:** trustless two-party token-for-token trades that must settle both legs or neither

## cross-contract

A caller contract that invokes another contract across the boundary.

This contract demonstrates:
1. How to generate a client for the called contract
2. How to invoke the contract via that client
3. How authorization requirements propagate from the called contract back
   to the original transaction's authorization entries

A simple receiver contract that enforces authorization.

This contract has a single entry point that requires the caller to be authorized.
When another contract calls this with authorization, the caller's address is
validated through `require_auth()`.

**Entrypoints:** `caller::invoke_receiver`, `receiver::authorized_action`

**When to pick it:** learning or bootstrapping the caller/receiver pattern and how authorization propagates across a contract call

## crowdfund

A crowdfunding escrow contract with a funding target and deadline.

Backers `pledge` a token until the deadline. Afterwards, either the owner
`claim`s the funds (target reached) or every backer gets a `refund`
(target missed). Funds are held by the contract itself in escrow.

**Entrypoints:** `pledge`, `claim`, `refund`, `get_pledge`, `get_total_pledged`, `get_target`, `get_deadline`, `get_token`, `get_owner`

**When to pick it:** deadline-based fundraising that must refund automatically if a goal is missed

## dutch-auction

> `templates/dutch-auction/` has no `template.toml` manifest — variables and post-generate hints for it, if any, are not declared.

Dutch Auction contract — descending price auction.

A descending-price auction where the price decays linearly from a start
price to a floor price over a fixed window. The first buyer settles at
the current price.

**Entrypoints:** `initialize`, `fund`, `get_current_price`, `buy`, `cancel`, `get_state`

**When to pick it:** a single-item sale that should clear quickly via a falling price instead of a bidding war

## english-auction

> `templates/english-auction/` has no `template.toml` manifest — variables and post-generate hints for it, if any, are not declared.

An ascending-price auction where bidders place successively higher bids
within a fixed time window. The highest bidder at auction end wins the asset.

**Entrypoints:** `initialize`, `fund`, `place_bid`, `settle`, `cancel`, `get_highest_bid`, `get_state`, `get_end_time`, `get_min_increment_bps`, `withdraw_refund`

**When to pick it:** **do not use yet** — this template is an unfinished stub (see note above); pick a different auction pattern for now

## escrow

> `templates/escrow/` has no `template.toml` manifest — variables and post-generate hints for it, if any, are not declared.

token escrow with approval or timeout-based refund path

**Entrypoints:** `initialize`, `deposit`, `approve_release`, `refund_on_timeout`, `get_state`

**When to pick it:** a single buyer/seller trade that needs an approval-or-timeout release path, without building a marketplace

## faucet

> `templates/faucet/` has no `template.toml` manifest — variables and post-generate hints for it, if any, are not declared.

Token faucet dispensing a fixed amount per address with a cooldown.

* **Dispense** — any address may call `claim` to receive a fixed
  `amount_per_claim` of the configured token.
* **Cooldown** — an address must wait `cooldown_seconds` between
  successive claims; claiming before the cooldown elapses panics.

**Entrypoints:** `initialize`, `fund`, `withdraw`, `set_params`, `claim`, `next_claim_time`

**When to pick it:** testnet/devnet token distribution with a per-address cooldown, e.g. for demos or QA environments

## flash-loan

Flash loan pool — lends without collateral inside a single transaction and
requires principal + fee back before the call returns.

Nothing secures the loan except *atomicity*. The pool transfers the funds
out, calls back into the borrower, and then re-reads its own balance. If
the borrower did not make the pool whole, the check panics — and that panic
unwinds the transfer that funded the loan along with everything the
borrower did in between. The pool either ends the call richer by the fee,
or the call never happened.

Read the security caveats in `README.md` before adapting this. Flash loans
are a well-understood attack primitive as much as a useful one, and most of
what can go wrong goes wrong in the *contracts around* the pool.

Liquidity is owned, not pooled: every deposit mints LP shares to the
depositor, and every withdrawal redeems shares at the current share price,
so fees earned while a depositor held liquidity accrue to them. The `admin`
is only the initial LP and the fee setter.

**Entrypoints:** `deposit`, `withdraw`, `flash_loan`, `fee_for`, `balance`, `token`, `fee_bps`, `admin`, `shares`, `total_shares`

**When to pick it:** uncollateralized same-transaction borrowing, or as a reference for writing a callback-repaid pool

## governance

> `templates/governance/` has no `template.toml` manifest — variables and post-generate hints for it, if any, are not declared.

DAO-style governance: members open proposals and vote on them; a proposal
executes once its voting period ends with quorum and a majority in favour.

**Voting power.** Every registered member has exactly one vote per
proposal. Callers cannot choose their weight: `cast_vote` takes no weight
argument and always counts `VOTE_WEIGHT` (1), so a single voter can never
outvote everyone else, and negative or inflated votes are impossible by
construction.

Members are registered by the admin. Eligibility is snapshotted per
proposal: only members registered at or before the ledger a proposal was
created in may vote on it, so adding members mid-vote cannot swing an open
proposal. Removing a member revokes their ability to vote from then on.

To weight votes by token holdings instead, replace `voting_power` with a
balance read through `token::Client` — and snapshot balances when the
proposal is created, or the same tokens can be moved between accounts and
voted with repeatedly.

**Entrypoints:** `initialize`, `add_member`, `remove_member`, `is_member`, `member_count`, `create_proposal`, `cast_vote`, `execute_proposal`, `get_proposal`

**When to pick it:** on-chain DAO decisions by a known set of members, with one vote each, quorum, and automatic execution of the winning proposal

## governance-token

A SEP-41 governance token (`soroban_sdk::token::TokenInterface`) with vote
delegation and historical ledger checkpoints queried via binary search.

Holders delegate their voting power to a representative (or to themselves)
with `delegate`. Every mint, transfer, burn, or delegation change updates
the affected delegates' voting power and appends (or overwrites in the same
ledger) a [`Checkpoint`] recording `(ledger, votes)`. Governance contracts
query `get_past_votes(account, proposal_snapshot_ledger)` and
`get_past_total_supply(proposal_snapshot_ledger)` using binary search so
tokens acquired or moved after a proposal opens cannot influence that vote.

**Entrypoints:** `mint`, `admin`, `total_supply`, `delegate`, `delegates`, `get_votes`, `get_past_votes`, `get_past_total_supply`, `num_checkpoints`, `checkpoints`

**When to pick it:** token-weighted DAO voting where holders can delegate votes and governance reads past voting power at a proposal's snapshot ledger

## hello-world

minimal greeter contract (recommended starting point)

**Entrypoints:** `hello`

**When to pick it:** your very first soroban-forge project, or a clean slate before writing real contract logic

## lottery

> `templates/lottery/` has no `template.toml` manifest — variables and post-generate hints for it, if any, are not declared.

Lottery / raffle using a commit-reveal scheme for randomness.

* **Enter** — during the commit phase, entrants submit a commitment
  (`sha256(entrant_xdr || nonce)`) that hides a secret nonce they will reveal later.
* **Reveal** — during the reveal phase, entrants reveal their nonce; the
  contract checks it hashes to their commitment.
* **Draw** — after the reveal phase ends, anyone can call `draw`, which
  XORs every revealed nonce into a single seed and picks the winner from
  the entrants who successfully revealed. Entrants who never reveal
  forfeit the draw — this is what makes the scheme resistant to a single
  party biasing the outcome, since no one controls the final seed alone.

### Security & Limitations: Last-Revealer Advantage
Plain commit-reveal schemes suffer from a known structural limitation:
the **last participant to reveal can calculate the final seed both with
and without their nonce before deciding whether to reveal**.

If withholding their reveal produces a winning outcome for themselves or
a colluding party, they can simply choose not to reveal. Because entrants
who do not reveal are simply excluded from the draw without penalty, the
last revealer can steer or bias the outcome.

**When not to use this template:**
Do not use this plain commit-reveal template for high-value raffles,
production lotteries, or gaming scenarios where participants have a financial
incentive to withhold their secret nonce. For high-stakes applications on
Stellar/Soroban, use a Verifiable Random Function (VRF) oracle instead of
plain commit-reveal.

**Entrypoints:** `initialize`, `commitment`, `enter`, `reveal`, `draw`, `get_winner`

**When to pick it:** on-chain randomness where no single party (including the drawer) should be able to bias the outcome

## merkle-airdrop

> `templates/merkle-airdrop/` has no `template.toml` manifest — variables and post-generate hints for it, if any, are not declared.

Merkle airdrop — one claim per eligible address, proven against a root.

An allowlist of `(address, amount)` entries can be arbitrarily large
without paying to store it on-chain: the tree is built off-chain and only
its 32-byte root lives in the contract.

* **Leaf** — `sha256(xdr(address) || be_bytes(amount))`, so a proof binds
  the claimant *and* the amount; neither can be swapped for another.
* **Proof** — the sibling hashes from the leaf up to the root. Pairs are
  hashed in sorted order (`sha256(min || max)`), which makes the proof
  order-independent so no left/right flags are needed.
* **One claim** — a marker per claimant is written before the transfer, so
  a second claim is rejected even with a valid proof.

Build the same tree off-chain with the `leaf` and pair-hash rules above —
`leaf` is exposed as an entrypoint so a script can cross-check its own
hashing against the contract's.

**Entrypoints:** `initialize`, `set_deadline`, `fund`, `set_root`, `root`, `admin`, `leaf`, `has_claimed`, `claimed_amount`, `verify`, `claim`, `sweep`

**When to pick it:** distributing tokens to a large, fixed allowlist without storing every address on-chain

## multi-token

> `templates/multi-token/` has no `template.toml` manifest — variables and post-generate hints for it, if any, are not declared.

A multi-token contract (ERC1155-style) managing many token ids with
per-id, per-owner balances in a single contract instance.

**Entrypoints:** `admin`, `name`, `symbol`, `balance_of`, `balance_of_batch`, `mint`, `mint_batch`, `transfer`, `transfer_batch`, `burn`, `burn_batch`

**When to pick it:** one contract instance needs to manage many distinct token ids (ERC1155-style), not a single fungible/NFT asset

## multisig

> `templates/multisig/` has no `template.toml` manifest — variables and post-generate hints for it, if any, are not declared.

An M-of-N multisig account contract.

Based on the official custom-account example in stellar/soroban-examples.
The contract stores a set of ed25519 signer public keys and a threshold
`M`. Any `require_auth` for this contract's address passes when at least
`M` valid, correctly ordered signatures over the signature payload are
provided. Operations on the account contract itself (such as
`set_threshold`) additionally require signatures from *every* signer.

**Entrypoints:** `rotate_signers`, `set_threshold`, `threshold`

**When to pick it:** an account that should require M-of-N co-signers for every outgoing action

## nft

> `templates/nft/` has no `template.toml` manifest — variables and post-generate hints for it, if any, are not declared.

A non-fungible token (NFT) contract implementing ownership, per-token
metadata URIs, admin-gated minting, transfers, and burning.

**Entrypoints:** `admin`, `name`, `symbol`, `owner_of`, `balance_of`, `token_uri`, `mint`, `transfer`, `burn`, `approve`, `get_approved`, `set_approval_for_all`, `is_approved_for_all`, `transfer_from`

**When to pick it:** a standalone non-fungible token with per-token metadata and admin-gated minting

## nft-marketplace

> `templates/nft-marketplace/` has no `template.toml` manifest — variables and post-generate hints for it, if any, are not declared.

NFT Marketplace contract.

Allows listing, buying, and cancelling NFT sales with a configurable
protocol fee sent to a treasury address. Built to interact with NFTs
conforming to the `nft` template's interface.

The protocol fee is **snapshotted into each listing** when it is created,
and `buy` always uses that stored fee. Changing the fee later only affects
new listings, so the admin cannot reduce a seller's payout after they
list. The fee is also capped at `MAX_FEE_BPS` (10%).

**Entrypoints:** `admin`, `treasury`, `fee_bps`, `set_admin`, `set_treasury`, `set_fee_bps`, `list`, `buy`, `cancel`, `get_listing`

**When to pick it:** listing and buying/selling NFTs (from an `nft`-shaped contract) with a protocol fee, without writing escrow logic yourself

## oracle-consumer

> `templates/oracle-consumer/` has no `template.toml` manifest — variables and post-generate hints for it, if any, are not declared.

An oracle-consumer contract: reads the latest price for a configured
asset from an external oracle contract and uses it to convert amounts.

The oracle interface (`lastprice`) matches the shape published by
[Reflector](https://reflector.network/), a common Stellar price oracle,
so this contract can be pointed at a real deployed Reflector instance —
or, as in the tests, at a small mock oracle contract.

**Entrypoints:** `get_price`, `convert`

**When to pick it:** pricing logic that needs a live external price feed, in a shape compatible with Reflector

## pausable

> `templates/pausable/` has no `template.toml` manifest — variables and post-generate hints for it, if any, are not declared.

A minimal circuit breaker: the admin fixed at deploy time can `pause` and
`unpause` the contract, and every guarded entrypoint refuses to run while
it is paused.

`increment` and `reset` stand in for whatever your contract actually does —
they call [`require_not_paused`] first, which is the whole pattern. Read-only
entrypoints (`count`, `is_paused`, `admin`) are deliberately left unguarded
so state stays inspectable during an incident.

**Entrypoints:** `admin`, `is_paused`, `pause`, `unpause`, `increment`, `reset`, `count`

**When to pick it:** a circuit breaker you can drop into any contract to freeze guarded entrypoints during an incident

## payment-splitter

> `templates/payment-splitter/` has no `template.toml` manifest — variables and post-generate hints for it, if any, are not declared.

Payment splitter — distributes received funds to payees by fixed shares.

Payees and their shares are fixed at initialization, which can only run
once — afterwards nobody, including the deployer, can rewrite the payee
table. Every deposit is
credited to the pool, and each payee can withdraw their proportional cut
at any time:

* **Shares** — each payee holds a fixed number of shares out of
  `total_shares`; a payee is entitled to
  `total_received * share / total_shares` over the contract's lifetime.
* **Pull payments** — nothing is pushed on deposit. `release` transfers
  what a payee has earned but not yet withdrawn, so a failing payee can
  never block a deposit.
* **Rounding** — entitlements are floor-divided, so up to
  `total_shares - 1` stray units can stay in the contract. `undistributed`
  reports that dust; it is released as later deposits push each payee's
  entitlement over the next whole unit.

**Entrypoints:** `initialize`, `deposit`, `shares`, `released`, `earned`, `releasable`, `release`, `payees`, `total_shares`, `total_received`, `total_released`, `undistributed`

**When to pick it:** dividing incoming payments among fixed payees by share, with pull-based withdrawals

## prediction-market

A binary (YES/NO) prediction market with parimutuel payouts.

Stakers back one of two outcomes with a SEP-41 token; a single designated
oracle reports which one happened; winners then split the whole pool in
proportion to what they staked:

* **One pool** — YES and NO stakes are escrowed together by the contract.
  A winner is entitled to `stake * total_pool / winning_pool`, so the
  losing side's stakes are what pays the winning side's profit.
* **Betting close** — the market has a close time (a ledger timestamp)
  fixed at deploy time. `stake` only works before it and `resolve` only
  works from it on, so nobody can still buy in once the outcome can be
  reported. The close instant itself counts as closed.
* **Oracle** — `resolve` is gated on one address fixed at deploy time.
  Nobody else can report an outcome, and it can only be reported once.
* **Pull payments** — nothing is pushed on resolution. Each winner claims
  their own payout, exactly once.
* **Rounding** — payouts are floor-divided, so up to `winning_pool - 1`
  units can stay behind in the contract as dust.

* **Refund when nobody wins** — if the winning outcome has no stakes, there
  is no winner to pay, so `claim` refunds each participant their own stake
  instead of locking the pool.
* **Resolution deadline** — the oracle must resolve before a deadline fixed
  at deploy time (after the close time). If it never does (a lost key, an
  abandoned market), `refund` returns each participant everything they
  staked, on both sides. The deadline instant itself counts as expired.

**Entrypoints:** `stake`, `resolve`, `claim`, `refund`, `get_oracle`, `get_token`, `get_close_time`, `get_deadline`, `get_outcome`, `get_pool`, `get_total_pool`, `get_stake`, `has_claimed`

**When to pick it:** a binary-outcome market settled by a single trusted oracle, paid out parimutuel-style

## soulbound

> `templates/soulbound/` has no `template.toml` manifest — variables and post-generate hints for it, if any, are not declared.

A soulbound (non-transferable) token contract: admin-gated minting,
self-service burning, and a `transfer` entrypoint that is always
rejected — tokens are permanently bound to the address they were
minted to.

**Entrypoints:** `admin`, `name`, `symbol`, `owner_of`, `balance_of`, `token_uri`, `mint`, `transfer`, `burn`

**When to pick it:** non-transferable tokens — reputation, credentials, or membership marks that must stay with the original holder

## staking

> `templates/staking/` has no `template.toml` manifest — variables and post-generate hints for it, if any, are not declared.

proportional reward staking with O(1) acc_reward_per_share accumulator

**Entrypoints:** `deposit`, `withdraw`, `distribute`, `claim`, `get_staked`, `get_total_staked`, `get_acc_reward_per_share`, `get_pending_reward`, `get_stake_token`, `get_reward_token`

**When to pick it:** reward-per-share style staking where depositors accrue a proportional share of periodically distributed rewards

## storage-migration

A contract demonstrating storage migration from v1 to v2.

This contract uses a version-marker pattern to safely migrate persistent
storage layouts across contract upgrades. The migration runs exactly once
and is idempotent.

**Entrypoints:** `init`, `get_counter`, `get_name`, `increment`, `set_name`, `version`

**When to pick it:** learning or bootstrapping the version-marker pattern for safely migrating storage layouts across upgrades

## streaming

> `templates/streaming/` has no `template.toml` manifest — variables and post-generate hints for it, if any, are not declared.

Streaming payments — a token released continuously over time.

Tokens are locked in the contract and become claimable by a recipient
pro-rata to elapsed time since the stream started:

* **Linear release** — tokens vest continuously at a constant rate from
  `initialize` until `duration_seconds` has elapsed.
* **Claim** — the recipient may call `claim` at any point, including
  mid-stream; the contract transfers the newly streamed amount since the
  last claim.

**Entrypoints:** `initialize`, `fund`, `cancel`, `get_streamed_amount`, `get_claimable_amount`, `claim`

**When to pick it:** continuous, claimable-anytime payment release over a fixed duration (vesting without a cliff)

## subscription

> `templates/subscription/` has no `template.toml` manifest — variables and post-generate hints for it, if any, are not declared.

Subscription — a recurring payment charged once per elapsed interval.

A merchant configures one plan (token, amount, interval). Subscribers opt
in and the merchant pulls the fee once per interval:

* **Allowance, not custody** — the contract never holds subscriber funds.
  The subscriber `approve`s this contract on the token for as many periods
  as they want to pre-pay, and `charge` moves one period's amount straight
  to the merchant with `transfer_from`. Revoking the allowance is enough to
  stop payments even without calling `cancel`.
* **One charge per interval** — the due date advances by exactly `interval`
  per charge, so a merchant calling twice in a row is rejected and a late
  charge does not silently skip a period.
* **Cancel any time** — the subscriber deactivates their subscription;
  further charges are rejected.

**Entrypoints:** `initialize`, `subscribe`, `charge`, `cancel`, `is_active`, `subscription`, `is_due`, `merchant`, `amount`, `interval`

**When to pick it:** recurring merchant billing against a payer's standing allowance, charged at most once per interval

## timelock

> `templates/timelock/` has no `template.toml` manifest — variables and post-generate hints for it, if any, are not declared.

timelock controller for delayed execution and cancellation of queued calls

**Entrypoints:** `initialize`, `get_min_delay`, `set_min_delay`, `has_role`, `grant_role`, `revoke_role`, `hash_operation`, `hash_batch`, `get_operation_state`, `get_operation`, `queue`, `queue_batch`, `execute`, `execute_batch`, `cancel`

**When to pick it:** any privileged action (upgrades, parameter changes) that must be queued and delayed before it can execute

## token

A fungible token implementing the standard Soroban token interface
(`soroban_sdk::token::TokenInterface`, SEP-41), plus an admin-gated `mint`.

Based on the patterns in the official `stellar/soroban-examples` token.

**Entrypoints:** `mint`, `admin`

**When to pick it:** a standard SEP-41 fungible token with no extra restrictions

## upgradeable

> `templates/upgradeable/` has no `template.toml` manifest — variables and post-generate hints for it, if any, are not declared.

admin-gated upgradeable contract (update_current_contract_wasm)

**Entrypoints:** `admin`, `upgrade`

**When to pick it:** learning or bootstrapping the minimal admin-gated `upgrade(new_wasm_hash)` pattern

## vesting

> `templates/vesting/` has no `template.toml` manifest — variables and post-generate hints for it, if any, are not declared.

Token vesting with cliff + linear release schedule.

Tokens are locked in the contract and become claimable by a beneficiary
over time according to a configurable vesting schedule:

* **Cliff** — no tokens are claimable until a duration (in seconds) has
  passed since the contract was initialized.
* **Linear release** — after the cliff, tokens vest continuously at a
  constant rate over the remaining vesting duration.
* **Claim** — the beneficiary may call `claim` at any point; the contract
  transfers the newly vested amount since the last claim.

**Entrypoints:** `initialize`, `fund`, `revoke`, `get_vested_amount`, `get_claimable_amount`, `claim`

**When to pick it:** token grants that unlock linearly after a cliff, e.g. team or investor allocations

## wrapped-asset

> `templates/wrapped-asset/` has no `template.toml` manifest — variables and post-generate hints for it, if any, are not declared.

Wrapped asset — mints a wrapper balance on deposit, burns it on withdraw,
always 1:1 with an underlying SEP-41 token locked in the contract.

* **Wrap** — deposit `amount` of the underlying token; the contract locks
  it and credits the caller's wrapper balance by `amount`.
* **Unwrap** — burn `amount` of wrapper balance; the contract releases
  `amount` of the underlying token back to the caller.
* **Conservation** — total wrapper supply always equals the underlying
  token balance held by this contract.

**Entrypoints:** `initialize`, `wrap`, `unwrap`, `balance_of`, `total_supply`

**When to pick it:** a 1:1 wrapper around an existing SEP-41 token, e.g. to give it a different contract identity

## yield-vault

> `templates/yield-vault/` has no `template.toml` manifest — variables and post-generate hints for it, if any, are not declared.

ERC-4626-style yield vault with proportional shares and vault-favoured rounding

**Entrypoints:** `initialize`, `asset`, `total_assets`, `total_shares`, `balance_of`, `convert_to_shares`, `convert_to_assets`, `deposit`, `withdraw`, `add_yield`

**When to pick it:** ERC4626-style pooled yield with proportional shares, when share-inflation attacks must be rounded against depositors
