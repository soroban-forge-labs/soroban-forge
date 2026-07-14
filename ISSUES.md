# Follow-up issues for v0.2+

Well-scoped starter work for contributors, grouped by module. Difficulty tags:
**trivial** (good first issue, < 1 day) · **medium** (a few days) ·
**high** (design work involved). Claim an issue by commenting on it in the
GitHub tracker.

## Module 1 — CLI core (`crates/core`)

1. **[trivial] Add `--quiet` global flag** — suppress non-error output;
   thread it through `ForgeContext` next to `verbose`.
2. **[medium] Add `soroban-forge config` subcommand** — print the resolved
   `forge.toml` (with defaults filled in) and warn on unknown keys.
3. **[medium] Structured output mode** — a global `--json` flag plugins can
   consult via `ForgeContext`, emitting machine-readable results (needed for
   editor integrations).
4. **[high] Dynamic plugin discovery** — investigate loading external
   subcommands `soroban-forge-<name>` from PATH (cargo-style), so third-party
   plugins don't need to be compiled in.

## Module 2 — Scaffolding & templates (`crates/scaffold`, `templates/`)

5. **[trivial] Add `--edition` option** — let `new` generate edition 2024
   projects once the ecosystem settles on it.
6. **[medium] Add a `multisig` template** — an M-of-N account/authorization
   example based on the official custom-account example.
7. **[medium] `soroban-forge new --from <git-url>`** — scaffold from a remote
   template repository instead of a bundled one.
8. **[high] Template manifest (`template.toml`)** — per-template metadata
   (description, extra variables with prompts, post-generate hints) instead of
   the current convention-only format.

## Module 3 — Test harness generator (`crates/testgen`)

9. **[trivial] Detect multiple `#[contract]` structs** — currently only the
   first is used; generate one smoke test per contract.
10. **[medium] Fuzz-test generator** — `test-init --fuzz` emitting a
    `cargo-fuzz` target that feeds arbitrary values into contract methods.
11. **[medium] Parse constructor signatures** — read `__constructor` argument
    types and generate a smoke test with sensible default values instead of an
    `#[ignore]`d TODO.
12. **[high] Property-based invariant harness** — proptest-based generator
    asserting user-declared invariants (e.g. token supply conservation) across
    random call sequences.

## Module 4 — CI/CD presets (`crates/ci-presets`, `presets/`)

13. **[trivial] Pin action versions by SHA** — replace tag references in the
    GitHub presets with pinned commit SHAs plus a comment noting the tag.
14. **[medium] Add a GitLab CI preset** — `ci-init --provider gitlab` writing
    `.gitlab-ci.yml`; `output_dir()` already anticipates per-provider paths.
15. **[medium] Cache stellar-cli in the deploy workflow** — installing via
    the shell script on every run is slow; use a cached binary or a published
    action.
16. **[high] Release workflow preset** — tag-triggered workflow that builds
    the wasm, attaches it to a GitHub Release with checksums, and (optionally)
    verifies reproducibility.

## Module 5 — Docs, examples & DX (`crates/doctor`, `docs/`, `examples/`)

17. **[trivial] `doctor --json`** — machine-readable check output for use in
    scripts and editors.
18. **[trivial] Check soroban-sdk version in doctor** — when run inside a
    contract project, warn if the project's soroban-sdk is behind the version
    forge templates pin.
19. **[medium] Video/asciinema quickstart** — record the zero-to-testnet
    tutorial and embed it in the README.
20. **[high] `soroban-forge upgrade` guide + docs site** — document migrating
    generated projects across sdk majors; publish docs/ via GitHub Pages.
