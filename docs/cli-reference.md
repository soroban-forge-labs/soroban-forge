# CLI Reference

## Global options

- `--quiet`, `-q` — suppress informational command output; errors and exit
  codes are unchanged.
- `--verbose`, `-v` — enable debug logging.
- `--log-file <path>` — also write JSON-lines structured logs to a file while trying to
  preserving normal terminal output.
- `--offline` — prohibit network access. Network-dependent operations fail with a
  a clear message, while `doctor` skips its connectivity prob.





  cd

Global options may appear before or after a subcommand and can be combined.

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
- `soroban-forge ci-init --provider <github|gitlab|circleci|bitbucket>` —
  generate CI workflows. `--matrix` adds a build/test workflow that runs across
  a Rust toolchain matrix (stable plus `--msrv`, default 1.84).
- `soroban-forge test-init` — generate a test harness.
- `soroban-forge ci-init --provider github [--dependabot]` — generate CI
  workflows (build+test and a rustfmp/clippy lint job); `--dependabot` also
  writes `.github/dependabot.yml` for weekly cargo and github-actions updates.
- `soroban-forge doctor [--json]` — check the local Soroban toolchain (optionally emitting machine-readable JSON).
- `soroban-forge bindings ts` — generate a TypeScript client package from the built contract wasm.
- `soroban-forge spec[--path <dir>] [--wasm <path>]` — print the built
  contract's interface: every entrypoint with its argument and return types,
  plus the types those signatures refer to. Reads the spec out of the wasm, so
  run `stellar contract build` first; `--json` emits the machine-readable spec.
  Works under `--offline`.
- `soroban-forge optimize` — optimize a built contract wasm. Use `--check` with
  `--max-size <bytes>` to fail (exit 1) if the optimized size exceeds the
  budget. The budget can also be set as `optimize.max_size` in `forge.toml`; the
  command-line `--max-size` overrides the config. On failure, the actual
  and budgeted sizes are printed.
- `soroban-forge verify <contract-id> [--network <n>]` — compare a deployed
  contract's wasm hash with the local release build; exits `1` on a mismatch.
  See [Contract Verification](contract-verification.md).
