# Changelog

All notable changes to Soroban Forge will be documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.0.0/).

## [Unreleased]

### Added

- `soroban-forge optimize --all` optimizes every contract crate in a
  workspace in one invocation, reporting one result per contract (an array
  under `--json`) (#412)
- `soroban-forge optimize --profile <name>`, `--target <triple>` and
  `--target-dir <dir>` optimize a build outside the default
  `target/wasm32v1-none/release`, keeping the crate-name detection that
  derives the wasm's file name; `--profile dev` resolves to cargo's `debug/`
  directory (#419)
- `soroban-forge optimize --compare-to <report.json>` reports the size delta
  (bytes and percent) against a report saved from an earlier
  `optimize --json` run (#418)
- `soroban-forge network list --check` probes each configured endpoint and
  annotates which networks are reachable; plain `network list` stays
  offline (#411)
- `soroban-forge network add` and `network use` probe the endpoint they
  select and warn when it does not respond, instead of leaving a typo'd or
  offline RPC URL to surface at the next `deploy`/`invoke`/`verify`. The
  command still succeeds and still saves the configuration; `--no-check`
  skips the probe, and `--offline` skips it silently
- `soroban-forge identity remove <name>` deletes a stored identity and reports
  a clear error when the name is not found (#420)
- `soroban-forge --list --json` now emits structured `builtin` and `external`
  subcommand arrays for scripts (#332)
- `soroban-forge bindings-py`: a new crate generating a typed Python client
  (`client.py`) from the built contract wasm — structs, tagged enums and
  error enums as dataclasses/`IntEnum`, one method per entrypoint,
  delegating all on-chain interaction to the official `stellar-sdk` package.
  Validated with `mypy --strict` and real encode/decode round-trips (#270)
- `.github/workflows/bindings-typecheck.yml`: generates `--react` bindings
  for the `token` and `nft` templates and runs a pinned `tsc --noEmit`
  against them on every push/PR (#269)
- `soroban-forge bindings ts --react`: emits `src/hooks.ts`, a typed React
  hook per entrypoint (a query-style hook for reads, a mutation hook for
  writes), exported at a `./hooks` subpath with `react` as an optional peer
  dependency. Strictly opt-in — nothing changes without the flag (#268)
- `.github/workflows/docs.yml`: builds `docs/` with mdBook and publishes it
  to GitHub Pages on every push to `main`; PRs build-check only, never
  deploy. Fixed `book.toml`, which no longer built under current mdBook
  (`multilingual` was removed) (#105)
- `soroban-forge bindings ts`: add `--out-dir` (alias `--output`) to customize where bindings are written and `--package-name` to set a custom npm package name, validated against npm naming rules (#267)
- `soroban-forge spec`: render contract interface as GitHub Flavored Markdown (`--format md` / `--format markdown`) with entrypoint and custom type tables (#276)
- `soroban-forge spec`: read interface from deployed contract ID (`spec <CONTRACT_ID>`), defaulting to local wasm when omitted and guarded under `--offline` (#278)
- `soroban-forge deploy`: detect unfunded testnet source identities with interactive Friendbot prompt and automatic funding via `--fund`, guarded under `--offline` and mainnet (#282)
- `soroban-forge verify` now prints an interface diff on a hash mismatch:
  entrypoints added, removed or changed between the deployed contract and
  the local build, also available as `spec_diff` in `--json`. If either
  interface cannot be read the diff is reported as unavailable and the
  mismatch verdict is unchanged (#275)
- `soroban-forge bindings ts` rewrites the generated `package.json` so the
  package is publishable as-is: conditional `exports` with `types`, `files`,
  a `prepack` build, name/version from `Cargo.toml`, and
  `@stellar/stellar-sdk` as a peer dependency (#266)
- `CODE_OF_CONDUCT.md` (Contributor Covenant 2.1), issue forms for bug
  reports and feature requests with a module picker that applies the
  `module:*` label, and a PR template covering tests, module READMEs and the
  changelog (#265)
- `docs/releasing.md`: the maintainer release process from version bump to
  published release, with each step marked manual or automated (#264)
- `soroban-forge identity import <name> --secret <S...>` (or `--secret-stdin`)
  registers an existing keypair, deriving and validating the public key (#421)
- `soroban-forge identity show <name> [--reveal-secret]` retrieves a stored
  identity; the secret key is printed only with the explicit flag (#422)

### Fixed

- Concurrent writes to `identities.json` and `networks.json` no longer lose
  each other: every read-modify-write of either store (`identity
  generate|import|use|fund|remove`, `network add|use|remove`) now runs under
  an advisory lock on a sibling lock file. Two `identity generate` runs started
  in parallel previously both read the same store and the second write
  discarded the first identity, even though its command reported success. See
  [docs/configuration.md](docs/configuration.md#identity-and-network-stores)
- `soroban-forge optimize` failures now include `stellar contract optimize`'s
  stderr in the error message instead of only the exit status; under `-v` the
  output still streams live (#417)
- `soroban-forge bindings ts` panicked ("Unknown argument or group id")
  on the plain, no-flags invocation — `--out-dir`'s fallback lookup queried
  its own `visible_alias` ("output") as if it were a second, separate arg id
- `main` failed to compile after conflicting merges in `core` (config
  report and unknown-key check), `doctor` (`gather_checks`) and `ci-presets`
  (`--diff` provider match, duplicate `Command` import, stray brace)
- New `pausable` template — a minimal circuit breaker: an admin fixed at
  deploy time can `pause`/`unpause`, and guarded entrypoints reject calls with
  `Error::Paused` while paused
- Templates `merkle-airdrop`, `faucet`, `subscription` and `streaming`:
  `initialize` can no longer be re-run. Anyone could previously call it again
  to take over the airdrop admin and `sweep`, drain the faucet, redirect
  subscription pulls to themselves, or redirect a funded stream; each now
  fails with `already initialized` (#505, #506, #507, #508)
- `soroban-forge new --license apache-2.0|mit|unlicense` writes a LICENSE
  file (author and year filled in) and sets the matching `license` field in
  the generated `Cargo.toml`. Omitting the flag keeps prior behaviour: no
  LICENSE file, no `license` field (#221)
- `soroban-forge new --devcontainer` adds an optional `.devcontainer/` with
  Rust, the `wasm32v1-none` target and `stellar-cli` preinstalled to the same
  minimum versions `doctor` checks for, so a scaffolded project is
  Codespaces-ready; documented in the generated project's README (#222)
- CI now scaffolds every bundled template and runs `cargo test` +
  `stellar contract build` against it, so a broken template fails the build.
  The template list comes from `new --list-templates --json`, not a
  hardcoded array (#223)
- Templates now share their `.gitignore`, `rust-toolchain.toml` and
  `Cargo.toml` release profile through `templates/_partials/`, composed at
  generation time; a template opts out simply by shipping its own copy of a
  file (#224)
- `soroban-forge new --var NAME=VALUE` (repeatable) plus support for a
  per-template `template.toml` manifest declaring custom variables. Missing
  values are prompted for when the session is interactive; non-interactive runs
  fall back to the declared default or fail with a message naming the flag
- `soroban-forge test-init --budget [ENTRYPOINT]` — generates
  `tests/forge_budget.rs`, a benchmark that measures one entrypoint with
  `env.cost_estimate().budget()` and asserts CPU-instruction and memory
  ceilings
- `soroban-forge test-init` now emits `tests/forge_init_once.rs` whenever the
  contract exposes an initialize-style entrypoint, asserting a second call is
  rejected
- `soroban-forge test-init --contract <name>` targets one member of a
  multi-contract workspace, matched by package name, crate name or directory.
  A workspace with more than one contract and no `--contract` now stops and
  lists the candidates instead of generating a harness for every member (#233)
- `soroban-forge test-init` now emits `tests/forge_upgrade.rs` for contracts
  with an upgrade entrypoint: it writes state, upgrades, and asserts the state
  survived. The test ships `#[ignore]`d and documents how to point it at a real
  v2 wasm (#234)
- `soroban-forge test-init --bench` emits criterion benchmarks under `benches/`
  and adds the `[[bench]]` target to `Cargo.toml`. Where `--budget` gates a
  ceiling, these track entrypoint cost over time (#235)
- `soroban-forge test-init` now emits `tests/forge_roundtrip.rs` for
  entrypoints taking `Option`, `Vec` or `Map` arguments, passing empty,
  single- and multi-element values through each — container arguments are
  frequently mis-encoded, the empty case most of all (#236)
- `flash-loan` template — an uncollateralized loan lent and repaid inside one
  transaction. The pool calls back into the borrower and then checks its own
  balance, so a borrower that does not repay principal + fee has the funding
  transfer unwound with the panic. Ships a `README.md` with the pattern's
  security caveats and 13 tests covering repaying, non-repaying, partially
  repaying and re-entering borrowers (#216)
- `soroban-forge doctor` reports a missing optional `wasm32-unknown-unknown`
  target as a warning instead of a failure, so it no longer makes doctor exit
  non-zero (#484)
- Repaired `doctor` and `identity` unit tests left unbalanced by a merge, which
  stopped those crates' test suites from compiling; the mainnet RPC test for
  `doctor`'s `config_network_url` now runs (#485)

### Changed

- `test-init --bench` is no longer an alias for `--budget`. It now emits
  criterion benchmarks; use `--budget` for the CPU/memory ceiling test (#235)
- `soroban-forge new --force` asks for confirmation before overwriting an
  existing directory. `--yes`, `--json` and non-interactive sessions skip the
  question, so scripts and CI are unaffected
- The `escrow`, `governance` and `vesting` templates now reject a second
  `initialize` call instead of silently resetting their state

- `test-init` now generates `tests/forge_ttl.rs` for contracts that use
  persistent storage: tests that bump an entry with `extend_ttl` and assert it
  outlives a ledger advance, as a starting point for rent regressions
- `test-init --layout <tests|inline>` — choose between the `tests/`
  integration-test directory (default) and a single `#[cfg(test)] mod
forge_tests` inside `src/`
- `ci-init --provider bitbucket` — generates `bitbucket-pipelines.yml`
  mirroring the GitHub build-test preset
- `ci-init --matrix [--msrv <version>]` — a build/test workflow that runs the
  job once per Rust toolchain (stable plus a pinned MSRV)
- `ci-init --provider github` now emits a `lint` job in `build-test.yml` running
  `cargo fmt --all --check` and `cargo clippy --all-targets -- -D warnings`, so
  generated CI fails on formatting or clippy violations
- `ci-init --dependabot` writes `.github/dependabot.yml` with weekly update
  schedules for the `cargo` and `github-actions` ecosystems
- `doctor` reports whether Docker is installed and its daemon is running
  (a warning when absent — it is only needed for reproducible wasm builds)
- `doctor` warns when git's `user.name`/`user.email` are unset, printing the
  `git config` commands to set them, since commits in a freshly created
  project otherwise fail confusingly
- `soroban-forge spec` — prints the built contract's interface (every
  entrypoint with its argument and return types, plus the types they refer to)
  by wrapping `stellar contract info interface`; `--json` emits the
  machine-readable spec
- Three new templates: `payment-splitter` (distributes received funds to
  payees by fixed shares), `subscription` (recurring payment charged once per
  elapsed interval) and `merkle-airdrop` (one claim per eligible address,
  verified against an on-chain merkle root)
- `soroban-forge verify <contract-id>` — compares the wasm hash of a deployed
  contract (fetched with `stellar contract fetch`) against the local release
  build, reporting match/mismatch as text or `--json`. Exits `1` on a
  mismatch so CI can gate on it
- Global `--quiet`/`-q` mode for suppressing successful command output while
  preserving errors and exit codes
- Comprehensive documentation suite
- Example contracts for common DeFi patterns
- CI pipeline with fmt, clippy, test, and WASM build steps

### Security

- The GitHub and GitLab deploy presets no longer pass the deployer's secret
  key as a `--source-account` argument, where any process that could list
  the runner's processes could read it and a `set -x` would echo it. Each
  preset now hands the key to `stellar keys add` through `STELLAR_SECRET_KEY`
  and deploys with `--source-account deployer` (#612)
- `governance` template: `cast_vote` no longer takes a caller-supplied
  `weight`, which let one voter pass any proposal (or subtract votes with a
  negative weight). Each admin-registered member now has exactly one vote,
  snapshotted at proposal creation; the README explains where voting power
  comes from and how to switch to snapshotted token weighting (#512)
- `storage-migration` template: `init` runs only once and refuses to
  overwrite existing v1 or v2 state (#511)
- `payment-splitter` template: `initialize` can no longer be re-run to
  rewrite the payee table and reset deposit accounting (#510)
- `lottery` template: a second `initialize` is rejected, so an in-progress
  or finished lottery can no longer be reset or taken over (#509)
