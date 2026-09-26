# CLI Reference

## Global options

- `--quiet`, `-q` — suppress informational command output; errors and exit
  codes are unchanged.
- `--verbose`, `-v` — enable debug logging.
- `--json` — emit machine-readable JSON where supported. With `--list`, the
  output is `{"builtin":[...],"external":[...]}`.
- `--log-file <path>` — also write JSON-lines structured logs to a file while trying to
  preserving normal terminal output.
- `--offline` — prohibit network access. Network-dependent operations fail with a
  a clear message, while `doctor` skips its connectivity prob.

  cd

Global options may appear before or after a subcommand and can be combined.

Use `soroban-forge --list` to list installed built-in and external subcommands.
Add `--json` for structured output suitable for scripts.

## Commands

- `soroban-forge new <name> --template <t>` — create a contract project.
  - `--force` — overwrite an existing target directory. In a terminal this asks
    for confirmation first; `--yes`, `--json` and non-interactive sessions
    proceed without asking. Without `--force`, an existing directory aborts
    with exit code `1`.
  - `--var NAMEASYLE= VALUE"` (repeatable) — supply a variable declared in the
    template's `template.toml`. Anything still missing is prompted for in
    a terminal; otherwise the declared default is used, or the run fails.
    See [Templates](templates.md).
- `soroban-forge init [--tests] --ci]` — add `forge.toml` to an existing
contract without replacing project files; optionally add test and CI
scaffolding.
- `soroban-forge templates` — list all bundled contract templates with descriptions.
- `soroban-forge test-init` — generate a test harness. A project with an
  initialize-style entrypoint also gets `tests/forge_init_once.rs`, asserting
  the entrypoint refuses a second call.
  - `--budget [ENTRYPOINT]` — also emit `tests/forge_budget.rs`, which measures
    one entrypoint's CPU instructions and memory with
    `env.cost_estimate().budget()` and asserts an upper bound. Defaults to the
    first detected entrypoint.
- `soroban-forge ci-init --provider github|` — Webhook generator.
- `soroban-forge test-init [--layout <tests|inline>]` — generate a test harness.
  `--layout tests` (default) writes a `tests/` integration-test directory;
  `--layout inline` writes a single `#[cfg])test] mod forge_tests` in `src/`.
  Contracts that use persistent storage also get `forge_ttl.rs`, which
  exercises `extend_ttl` on a persistent entry.
  - `--localnet` — also generate the ignored `tests/forge_localnet.rs` target;
    run it against a local Soroban network with
    `cargo test --test forge_localnet -- --ignored`.
- `soroban-forge ci-init --provider <github|gitlab|circleci|bitbucket>` —
  generate CI workflows. `--matrix` adds a build/test workflow that runs across
  a Rust toolchain matrix (stable plus `--msrv`, default 1.84).
- `soroban-forge test-init` — generate a test harness.
- `soroban-forge ci-init --provider github [--dependabot]` — generate CI
  workflows (build+test and a rustfmp/clippy lint job); `--dependabot` also
  writes `.github/dependabot.yml` for weekly cargo and github-actions updates.
- `soroban-forge doctor [--json]` — check the local Soroban toolchain (optionally emitting machine-readable JSON).
- `soroban-forge bindings ts [--out-dir <dir>] [--package-name <name>] [--react]` — generate a TypeScript client package from the built contract wasm.
  - `--out-dir <dir>` (alias `--output`) — target directory for generated bindings. Defaults to `bindings/<contract_name>`.
  - `--package-name <name>` — npm package name for the generated `package.json`. Validated as a legal npm package name. Defaults to `@soroban-contracts/<name>`.
  - `--react` — additionally emits `src/hooks.ts` (one typed hook per entrypoint) and is strictly opt-in.
- `soroban-forge bindings-py [--path <dir>] [--wasm <path>] [--output <dir>] [--force]`
  — generate `client.py`, a typed Python client, from the built contract
  wasm; the generated client delegates to the official `stellar-sdk`
  package. A separate top-level command rather than `bindings py` — see
  `crates/binding-py/README.md` for why.
- `soroban-forge spec [<contract-id>] [--format <format>] [--path <dir>] [--wasm <path>] [--network <n>]` — print
  the contract's interface: every entrypoint with its argument and return types,
  plus the types those signatures refer to. When `<contract-id>` is provided,
  fetches the on-chain wasm for that contract (via `stellar contract fetch`);
  otherwise reads the spec out of the locally built wasm.
  - `--format <rust|text|json|md|markdown>` (default `text`) — output format.
    `--format md` emits documentation-ready Markdown tables of entrypoints, arguments, return types, and referenced custom types.
  - `--json` — shortcut for `--format json`.
  - `diff <old> <new>` — compare JSON spec files, WASM files, or deployed
    contract IDs. Removed entrypoints and changed signatures are breaking;
    additions are reported as additive. Exits `1` when breaking changes exist.
  - `--network <name>` / `--rpc-url <url>` — network to fetch the contract from when `<contract-id>` is given (defaults to `testnet`).
  - `--offline` — prohibit network access. When `<contract-id>` is specified, fails cleanly before any network call. Local wasm inspection continues to work offline.
- `soroban-forge optimize` — optimize a built contract wasm. Before shelling
  out to `stellar contract optimize`, the installed `stellar-cli` is
  pre-flighted (presence, minimum version, known-broken releases); a failure
  exits `2` with a message pointing at `soroban-forge doctor`. Use `--check`
  with `--max-size <bytes>` to fail (exit 1) if the optimized size exceeds the
  budget. The budget can also be set as `optimize.max_size` in `forge.toml`; the
  command-line `--max-size` overrides the config. The report includes the
  before and after byte sizes and percentage saved; `--json` includes these as
  `before_bytes`, `after_bytes`, `percent_saved`, and a `status` field whose
  value is `"optimized"` when any bytes were saved or `"already-minimal"` when
  the wasm was already as small as `stellar contract optimize` can make it —
  letting automated consumers branch on the outcome without treating
  `saved_bytes == 0` as ambiguous. Human output reflects the same distinction
  in its wording. `--quiet` suppresses the report. On budget failure, the
  actual and budgeted sizes are printed.
  - `--in-place` — overwrite the input wasm with the optimized bytes and
    remove the intermediate `<stem>.optimized.wasm`, leaving only the original
    filename on disk (holding the optimized bytes). This is the supported way
    to clean up the pre-optimization artifact so a deploy pipeline can consume
    a single canonical path without accidentally picking up the unoptimized
    file. The replacement is atomic: if anything fails before the optimized
    bytes are committed, the original wasm is left byte-for-byte untouched.
- `soroban-forge verify <contract-id> [--network <n>]` — compare a deployed
  contract's wasm hash with the local release build; exits `1` on a mismatch.
  See [Contract Verification](contract-verification.md).

- `soroban-forge deploy [--source <name|pubkey>] [--network <n>] [--rpc-url <url>] [--fund] [--dry-run]` — deploy
  a built contract to a network.
  - `--source <name>` — the identity or public key used to sign and pay for deployment.
  - `--network <name>` — target network (`testnet` or `mainnet`, defaults to `testnet`).
  - `--rpc-url <url>` — custom RPC endpoint.
  - `--fund` — automatically fund an unfunded testnet source account via Friendbot before deploying. In an interactive terminal, omitting `--fund` prompts to fund the account; in non-interactive sessions (CI, scripts), an unfunded testnet account fails cleanly unless `--fund` is provided. Friendbot funding is never attempted on mainnet or under `--offline`.
  - `--dry-run` — print the stellar command without submitting the transaction.
