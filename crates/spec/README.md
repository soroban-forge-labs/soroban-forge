# soroban-forge-spec

`soroban-forge spec` — prints the interface of a built contract or deployed
contract: every entrypoint with its argument and return types, plus the structs,
enums and error enums those signatures refer to.

```sh
stellar contract build          # the spec is read out of the built wasm
soroban-forge spec              # human-readable listing
soroban-forge spec --format md  # documentation-ready Markdown table for READMEs
soroban-forge spec --json       # the same spec as JSON
soroban-forge spec --wasm path/to/contract.wasm
soroban-forge spec <CONTRACT_ID> # fetch and dump deployed contract interface
```

## Options

- `<contract-id>` — deployed contract ID (C…) to fetch and read from
- `--format <rust|text|json|md|markdown>` — output format (default: rust, or json with `--json`)
- `--path <dir>` — contract project directory [default: current directory]
- `--wasm <path>` — path to local built wasm file
- `--network <name>` — configured network for fetching deployed contracts [default: testnet]
- `--rpc-url <url>` — Stellar RPC endpoint URL (overrides network default)
- `--network-passphrase <pass>` — Stellar network passphrase

When reading local wasm files, `spec` never touches the network and works under `--offline`.
When given a contract ID, `spec` fetches the deployed wasm via `stellar contract fetch`;
under `--offline` this fails cleanly before attempting any network calls. The same is
true of `spec diff`, below, when either side is a contract ID.

## Markdown Format

`spec --format md` emits documentation-ready Markdown tables for embedding directly
into READMEs:
- An **Entrypoints** table listing functions, argument types, and return types.
- A **Custom Types** section listing structs, enums, error enums and unions
  referenced by the contract's entrypoints.

## `spec diff` — interface stability check

```sh
soroban-forge spec diff old.wasm new.wasm
soroban-forge spec diff old-spec.json new.wasm           # a spec --json file works too
soroban-forge spec diff CDLZ… new.wasm --network testnet # or a deployed contract
```

Each of `OLD`/`NEW` is auto-detected: a `.json` file (as `spec --json`
writes), a strkey contract ID (`C…`), or otherwise a wasm file. Both sides
are read with `stellar contract info interface --output json` and compared
entrypoint by entrypoint:

- a **removed** entrypoint, or one whose **signature changed** — breaking
- a **new** entrypoint — additive

```
✗ BREAKING — 1 removed, 1 changed (1 added)

  + mint(to: address, amount: i128)
  - burn(from: address, amount: i128)
  ~ balance
      old  balance(id: address) -> i128
      new  balance(id: address) -> u128
```

Exits `1` on a breaking change, so CI can gate a release: `soroban-forge spec
diff "$(git show main:target/…/old.wasm)" new.wasm || exit 1` (or diff two
`spec --json` snapshots checked into the repo). `--json` reports
`{"old", "new", "added", "removed", "changed", "breaking"}`. `--network`,
`--rpc-url` and `--network-passphrase` only matter when a side is a contract
ID; a contract ID under `--offline` is a user error, not a network attempt.

Unlike `bindings ts --react`'s hooks, `spec diff` does not exclude
`__constructor` — a changed constructor signature is a real interface change
worth flagging here.

## Public surface

- `validate_contract_id(id)` — validate contract ID format
- `read_crate_name(dir)` / `locate_wasm(dir, crate_name)` — where the release build lands
- `resolve_wasm(dir, wasm_override)` — the wasm the spec is read from
- `SpecFormat` (`Rust` / `Json` / `Markdown`) — output format enum
- `render_markdown_spec(json)` — convert spec JSON to GitHub Markdown tables
- `dump_interface_from_wasm(wasm, format)` — extract interface from wasm
- `dump_interface(dir, wasm_override, format)` — programmatic API behind the plain subcommand
- `classify_spec_arg(arg) -> SpecArg` — auto-detect a `spec diff` argument
  (file / wasm / contract ID)
- `read_spec_json(arg, network, offline)` / `diff_specs(old_json, new_json)`
  / `diff(old, new, network, offline)` — the programmatic API behind
  `spec diff`
- `SpecDiff` (`is_empty` / `is_breaking`), `ChangedEntrypoint`
- `format_diff_report` / `json_diff_report` / `breaking_change_error` —
  `spec diff` output and the error a breaking change becomes
- `NetworkArgs` — `--network` / `--rpc-url` / `--network-passphrase`, shared
  by the top-level contract-ID lookup and `spec diff`
- `SpecPlugin` — the `ForgePlugin` impl

## Tests

```sh
cargo test -p soroban-forge-spec
```
