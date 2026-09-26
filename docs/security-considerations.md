# Security considerations

**The bundled templates are starting points, not audited code.** Nothing in
`templates/` has gone through an external audit, and none of them are
production contracts you can deploy as-is with real value at stake. This page
states what each trust category actually looks like across the bundled
templates, so you know what to check — and what to add — before you do.

If you find a gap in a template that isn't listed here, please open an issue;
this page describes the templates as they exist today, not a guarantee about
what they will always do.

## Authorization

Every template that moves funds or changes privileged state gates the
relevant entrypoint on `require_auth()` — but *whose* address is required
differs by template, and getting that wrong when you copy the pattern is the
most common way to reintroduce a bug the template avoided:

- **Caller-authenticated**: most templates (`token`, `nft`, `escrow`,
  `vesting`, `crowdfund`, …) require the acting party's own address —
  the sender, the depositor, the admin — via that address's
  `require_auth()`.
- **Callback-authenticated**: `flash-loan`'s `receiver.exec` callback
  requires the *pool's* address, satisfied automatically because the pool is
  the direct caller — this is what stops an unrelated account from invoking
  the callback with numbers it made up, and it depends on Soroban's host
  rejecting reentrant calls into a contract already on the stack.
- **Address-compared, not authenticated**: `prediction-market`'s `resolve`
  takes the caller explicitly and compares it to a stored oracle address,
  rather than calling `require_auth()` on that address directly. This
  correctly rejects a wrong signer, but it means the check lives in an
  `if caller != stored_oracle` branch rather than the framework's own
  authorization machinery — read it carefully before copying the shape
  elsewhere.
- **Deliberately unauthenticated**: `prediction-market`'s `claim` and
  `flash-loan`'s repayment path require no authorization at all, by design —
  funds can only go to their rightful owner, so nothing is gained by
  triggering someone else's payout or by anyone but the borrower repaying.
  This is safe *because* of how each contract routes funds, not a general
  rule; don't drop `require_auth()` elsewhere without the same argument.

`cross-contract` and `access-control` exist specifically to demonstrate these
primitives in isolation — start there if you're adding authorization to a
contract of your own.

## Integer overflow

All bundled templates use `i128` for balances and amounts, and every
template's `Cargo.toml` pulls in the shared
[`_partials/release-profile.toml`](../templates/_partials/release-profile.toml),
which sets:

```toml
[profile.release]
overflow-checks = true
```

That means arithmetic overflow **panics and aborts the transaction** in
release builds too, not just in `cargo test` — Rust normally only checks
overflow in debug builds. This is a build-profile setting, not something
individual contract logic enforces, so it protects a template only as long as
nothing removes or overrides that profile. If you fork a template into a
project with its own `Cargo.toml`, carry `overflow-checks = true` forward
explicitly.

Panic-on-overflow converts a silent wraparound into a reverted transaction,
which is the safe failure mode — but it is still a denial of that
transaction, not a caught error the contract can recover from or report a
typed `Error` for. Amounts derived from user input that could plausibly
overflow `i128` (extremely large `token_decimals`, compounding reward math
over a long-lived `staking` deployment) are worth bounding explicitly rather
than relying on the panic as the only guard.

## Storage TTL expiry

Soroban archives persistent storage entries that outlive their TTL; an
archived entry is inaccessible until restored. How each template handles this
depends on which storage durability it uses:

- **Instance storage only** (`allowlist-token`, `amm`, `crowdfund`, `escrow`,
  `faucet`, `governance`, `oracle-consumer`, `pausable`,
  `payment-splitter`, `prediction-market`, `streaming`, `subscription`,
  `upgradeable`, `vesting`, `wrapped-asset`, and others): state lives in the
  contract's instance entry, which shares the instance's own TTL and needs no
  manual bumping. The tradeoff is that every entry you add here grows one
  entry's footprint for the life of the contract, with no per-entry
  expiration — fine for the bounded state these templates keep, worth
  watching if you add unbounded per-user data to instance storage yourself.
- **Persistent storage, TTL managed** (`access-control`, `merkle-airdrop`,
  `multi-token`, `nft`, `nft-marketplace`, `soulbound`, `token`,
  `yield-vault`): per-entry state (balances, ownership, claims) lives in
  persistent storage, and every read/write path calls `extend_ttl` on that
  entry, following the [TTL bump pattern](storage-ttl.md).
- **Persistent storage, TTL *not* managed — known gap** (`staking`,
  `timelock`): both templates store per-user or per-operation state in
  persistent storage (`Staked`/`RewardDebt` in `staking`, queued `Operation`s
  in `timelock`) but never call `extend_ttl` on any of it. As shipped, a
  staker's balance or a queued timelock operation can be archived once its
  TTL elapses, with no code path that ever renews it — restoring it would
  need an out-of-band restore, not a call the contract itself exposes. If you
  build on either template, add TTL bumping on every read/write of that state
  before relying on it past the default TTL window.

## Upgrade authority

Only `upgradeable` and `timelock` touch WASM upgrades, and they represent two
very different trust levels:

- `upgradeable`'s `upgrade(new_wasm_hash)` is gated on a single admin address
  fixed at deploy time. That admin can swap the contract's code for anything,
  immediately, with no delay and no second signer. This is the entire trust
  model — anyone who controls the admin key controls what code the contract
  runs next.
- `timelock` does not perform upgrades itself, but it is the intended way to
  soften that: queue the upgrade call through `timelock` instead of calling
  `upgrade` directly, so it is subject to a minimum delay and can be
  cancelled by a proposer or admin before it executes. Combining the two is a
  deliberate choice this repo leaves to you — `upgradeable` alone has no
  delay, and `timelock` alone upgrades nothing.

Neither template supports multi-party upgrade approval on its own; pair
`upgradeable` with `multisig` or `governance` (as the admin address) if a
single key should not be able to authorize an upgrade unilaterally.

## Oracle trust

`oracle-consumer` and `prediction-market` are the only templates that read an
externally supplied price or outcome, and both extend the same trust to a
single configured address:

- `oracle-consumer` reads `lastprice` from whatever contract address it was
  configured with, in the shape Reflector publishes. Nothing on-chain checks
  that address is actually a Reflector deployment, checks the returned
  price's staleness, or cross-checks it against a second source — the
  contract trusts the configured oracle completely. A misconfigured or
  compromised oracle address controls every value this contract derives from
  it.
- `prediction-market`'s `resolve` is gated on a single oracle address fixed
  at deploy time and can only be called once — there is no dispute window
  and no fallback if that address never calls `resolve` or resolves
  incorrectly. The market also never enforces a staking deadline (stakes are
  accepted until the moment of resolution) and pays out nothing if the
  resolved outcome has no backers; both are documented as deliberate,
  narrow tradeoffs in the template's own README, not bugs.

Neither template should be pointed at an oracle address you do not fully
trust, and neither implements the multi-oracle aggregation or staleness
checks a production price feed integration typically needs.
