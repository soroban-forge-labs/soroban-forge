# soroban-forge-spec

`soroban-forge spec` — prints the interface of a built contract: every
entrypoint with its argument and return types, plus the structs, enums and
error enums those signatures refer to.

```sh
stellar contract build          # the spec is read out of the built wasm
soroban-forge spec              # human-readable listing
soroban-forge spec --json       # the same spec as JSON
soroban-forge spec --wasm path/to/contract.wasm
```

The interface lives in the wasm's `contractspecv0` custom section as XDR.
Per soroban-forge's "wrap, don't reimplement" rule this module does not decode
that XDR itself — it shells out to the official
`stellar contract info interface` and owns only the parts around it: finding
the build (`target/wasm32v1-none/release/<crate>.wasm`, the same layout
`bindings ts`, `verify` and `doctor` expect), choosing the representation, and
reporting a missing `stellar` CLI as `ToolMissing` (exit `2`) with a pointer
to `soroban-forge doctor`.

Nothing here touches the network, so `spec` works under `--offline`, unless
`spec diff` is asked to compare a deployed contract by ID.

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

- `read_crate_name(dir)` / `locate_wasm(dir, crate_name)` — where the release
  build lands
- `resolve_wasm(dir, wasm_override)` — the wasm the spec is read from; errors
  point at `stellar contract build`
- `SpecFormat` (`Rust` / `Json`) — `SpecFormat::from_json_flag(ctx.json)`
- `spec_cli_args(wasm, format)` — the `stellar` arguments we invoke
- `dump_interface(dir, wasm_override, format) -> (PathBuf, String)` — the
  programmatic API behind the subcommand
- `format_header(wasm)` — the human-mode header line
- `classify_spec_arg(arg) -> SpecArg` — auto-detect a `spec diff` argument
  (file / wasm / contract ID)
- `read_spec_json(arg, network, offline)` / `diff_specs(old_json, new_json)`
  / `diff(old, new, network, offline)` — the programmatic API behind
  `spec diff`
- `SpecDiff` (`is_empty` / `is_breaking`), `ChangedEntrypoint`
- `format_diff_report` / `json_diff_report` / `breaking_change_error` —
  `spec diff` output and the error a breaking change becomes
- `NetworkArgs` — `--network` / `--rpc-url` / `--network-passphrase` for a
  contract-ID source
- `SpecPlugin` — the `ForgePlugin` impl

## Tests

```sh
cargo test -p soroban-forge-spec
```

The unit tests cover wasm resolution, the error messages and the exact
`stellar` command line that gets built, so they pass without `stellar-cli`
installed. Interface output itself is the CLI's, not ours.
