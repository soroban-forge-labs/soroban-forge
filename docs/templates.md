# Templates

For the full list of every bundled template — what each demonstrates, its
entrypoints, and when to pick it — see the
[template catalogue](template-catalogue.md). This page covers the smaller
set below plus how template variables work.

List them at any time with `soroban-forge templates`:

- `amm` – constant-product AMM / liquidity pool (x*y=k, 0.3% fee)
- `atomic-swap` – atomic two-party token swap with dual authorization
- `crowdfund` – escrow/deadline crowdfunding contract
- `dutch-auction` – descending-price auction with linear price decay and immediate settlement
- `escrow` – token escrow with approval or timeout-based refund path
- `flash-loan` – uncollateralized single-transaction loan repaid via a borrower callback
- `governance` – DAO governance with weighted voting, quorum, and proposal execution
- `hello-world` – minimal greeter contract (recommended starting point)
- `multisig` – M-of-N multisig account contract (`CustomAccountInterface`)
- `nft` – NFT with per-token metadata and minting
- `prediction-market` – binary outcome market with oracle resolution and parimutuel payouts
- `nft-marketplace` – NFT marketplace for listing, buying, and cancelling sales with configurable fees
- `token` – SEP-41 fungible token (`soroban_sdk::token::TokenInterface`)
- `timelock` – timelock controller for delayed execution and cancellation of queued calls
- `vesting` – token vesting with cliff + linear release schedule
- `yield-vault` – ERC4626-style yield vault with proportional shares and vault-favoured rounding

## Variables

Template files (and file names) may contain `{{variable}}` placeholders.
soroban-forge always provides `project_name`, `crate_name` (snake_case),
`author`, `sdk_version` and `edition`.

A template can declare its own variables in a `template.toml` at its root:

```toml
description = "a token with a configurable symbol"

[[variables]]
name = "token_symbol"
prompt = "Token symbol"
default = "TKN"

[[variables]]
name = "admin_address"
prompt = "Admin account (G...)"
required = true          # the default; set false to allow an empty value
```

Supply values on the command line:

```console
$ soroban-forge new my-token --template my-token --var token_symbol=USDC
```

Anything you leave out is prompted for when stdin and stdout are both a
terminal — press enter to accept the default shown in brackets:

```console
$ soroban-forge new my-token --template my-token
this template needs a few values (press enter to accept a default):
Token symbol [TKN]:
Admin account (G...): GB7X...
```

Runs that cannot prompt — piped input, `--json`, or `--yes` — use the declared
default instead. A required variable with no default and no `--var` is an
error naming the flag that would have supplied it, so CI fails fast rather than
hanging on a prompt.

`template.toml` configures generation and is never written into the generated
project. The five built-in variables are derived by soroban-forge and cannot be
redeclared or overridden with `--var`.
`soroban-forge templates` lists every bundled template with a one-line
description; `soroban-forge new <name> --template <t>` scaffolds one.

| template           | what you get                                                     |
|--------------------|------------------------------------------------------------------|
| `hello-world`      | minimal greeter contract (recommended starting point)            |
| `token`            | SEP-41 fungible token (`soroban_sdk::token::TokenInterface`)     |
| `nft`              | non-fungible token with per-token metadata and minting           |
| `nft-marketplace`  | NFT marketplace for listing, buying, and cancelling sales        |
| `crowdfund`        | escrow/deadline crowdfunding contract                            |
| `dutch-auction`    | descending-price auction with linear price decay                 |
| `escrow`           | token escrow with approval or timeout-based refund path          |
| `vesting`          | token vesting with cliff + linear release schedule               |
| `payment-splitter` | splits received funds between payees by fixed shares             |
| `subscription`     | recurring payment charged once per elapsed interval              |
| `merkle-airdrop`   | one-claim-per-address airdrop verified against a merkle root      |
| `amm`              | constant-product AMM / liquidity pool (`x*y=k`, 0.3% fee)         |
| `atomic-swap`      | atomic two-party token swap with dual authorization              |
| `flash-loan`       | uncollateralized loan repaid inside one transaction              |
| `governance`       | DAO governance with weighted voting, quorum and execution        |
| `multisig`         | M-of-N multisig account contract (`CustomAccountInterface`)      |
| `yield-vault`      | ERC4626-style yield vault with vault-favoured rounding           |
| `prediction-market`| binary outcome market with oracle resolution and parimutuel payouts |
| `timelock`         | timelock controller for delayed execution and cancellation       |

Every template ships a `README.md` with build/deploy instructions and unit
tests that pass out of the box:

```sh
soroban-forge new my-contract --template payment-splitter
cd my-contract
cargo test                 # the template's own tests
stellar contract build     # the deployable wasm
```

## Payment splitter

Payees and shares are fixed at initialization and payments are *pulled*: a
deposit only increases `total_received`, and each payee calls `release` to
withdraw `total_received * share / total_shares` minus what they already took.
Entitlements are floor-divided, so up to `total_shares - 1` units can sit in
the contract as rounding dust — `undistributed` reports it, and later deposits
make it claimable.

## Subscription

One plan (token, amount, interval) and one allowance per subscriber: the
subscriber `approve`s the contract on the token, `subscribe` charges the first
period, and the merchant calls `charge` once per elapsed interval. Each charge
advances the due date by exactly one interval, so the merchant can neither bill
twice for a period nor lose one by being late. `cancel` (or revoking the
allowance) stops future charges. The contract never holds subscriber funds.

## Merkle airdrop

Only the 32-byte merkle root of the `(address, amount)` allowlist is stored
on-chain, so the list can be arbitrarily large. `claim` recomputes the leaf,
walks the proof to the root and marks the claimant before transferring, so each
address claims exactly once. The hashing rules an off-chain tree builder must
reproduce are:

- leaf — `sha256(xdr(address) || be_bytes(amount))`
- pair — `sha256(min(a, b) || max(a, b))`, which makes proofs
  order-independent (sibling hashes only, no left/right flags)

The `leaf` entrypoint is public so a script can check its own hashing against
the contract's before publishing a root.

## Prediction market

A binary YES/NO market, staked in a SEP-41 token. Both sides escrow into one
pool, so the losing side's stakes are what pays the winning side's profit:

```text
payout = stake * total_pool / winning_pool
```

`resolve` is gated on a single oracle address fixed at deploy time and can only
be called once. It takes the caller explicitly and compares it to the stored
oracle rather than authorizing that oracle directly — the address that signed is
checked against the designated one, so a wrong signer is rejected instead of
silently accepted. `claim` needs no authorization: funds always go to the
staker, so triggering someone else's payout gains nothing.

Payouts are floor-divided, leaving up to `winning_pool - 1` units in the
contract as rounding dust. Two edges are deliberately left open, and the
generated README says so: there is no staking deadline, so the market accepts
stakes until the moment it resolves; and if the oracle resolves to an outcome
nobody backed there are no winners, so the pool stays put and every `claim`
reports `NothingToClaim`.
## Dutch auction

The seller initializes the auction with `start_price`, `floor_price`, and
`duration_seconds`, then deposits the asset via `fund`. The price decays linearly
from `start_price` to `floor_price` over the duration. The first buyer to call
`buy` purchases the asset at the current price, which immediately transfers the
asset to the buyer and the payment to the seller.

## Yield vault

An ERC4626-style vault where depositors escrow a SEP-41 token and receive proportional
vault shares. As yield is added (via `add_yield`), each share becomes redeemable for a
greater amount of underlying assets.

The vault enforces a strict **vault-favoured rounding policy**:
- **Deposits**: `shares = (assets * total_shares) / total_assets` (rounds down to prevent share inflation / dilution of existing holders).
- **Withdrawals**: `assets = (shares * total_assets) / total_shares` (rounds down to prevent draining fractional assets from remaining holders).
## Timelock controller

Enforces a minimum time delay between queueing an operation (contract call, function,
arguments, and salt) and executing it. Proposers queue calls with `delay >= min_delay`,
and executors trigger the call once the delay has elapsed. Proposers or admins can
cancel queued operations before execution.
## NFT marketplace

Sellers list NFTs from any contract conforming to the `nft` template's interface,
specifying a payment token (SEP-41) and price. The NFT is escrowed in the
marketplace during the listing. Buyers purchase NFTs at the listed price, with a
configurable basis-point protocol fee automatically routed to a treasury address
and the remainder sent to the seller. Sellers can cancel open listings at any time
to reclaim their NFT.
## Flash loan

`flash_loan` snapshots the pool's balance, transfers the principal to the
borrower, calls back into `receiver.exec(pool, token, amount, fee)`, and then
requires the balance to have grown by at least `fee`. That final check is the
only thing enforcing repayment — a shortfall panics, and the panic unwinds the
transfer that funded the loan along with everything the borrower did with it.
The pool ends the call up exactly one fee, or the call never happened.

Two properties are worth knowing before building on it. Soroban's host rejects
calling a contract already on the call stack, so the pool cannot be re-entered
from the callback and needs no guard of its own. And a borrower must
authenticate its caller: `exec` is a public entrypoint, so `pool.require_auth()`
— satisfied by the pool being the direct caller — is what separates a real loan
from anyone invoking the callback with numbers they made up.

The generated README carries the full security caveats, including why nothing
should ever price off a spot balance or key privileges off a live one.

## Adding a template

Templates are plain directory trees under
[`templates/`](../templates), embedded into the binary at compile time. See
[`crates/scaffold`](../crates/scaffold) for the format: `{{variable}}`
placeholders in file contents and names, and a `Cargo.toml.hbs` whose `.hbs`
suffix is stripped on render so cargo doesn't treat the template as a package.
A new template needs a one-line entry in `template_description()`.
