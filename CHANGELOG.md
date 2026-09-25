# Changelog

All notable changes to Soroban Forge will be documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.0.0/).

## [Unreleased]

### Added
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

### Fixed
- `main` failed to compile after conflicting merges in `core` (config
  report and unknown-key check), `doctor` (`gather_checks`) and `ci-presets`
  (`--diff` provider match, duplicate `Command` import, stray brace)
- New `pausable` template — a minimal circuit breaker: an admin fixed at
  deploy time can `pause`/`unpause`, and guarded entrypoints reject calls with
  `Error::Paused` while paused
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
