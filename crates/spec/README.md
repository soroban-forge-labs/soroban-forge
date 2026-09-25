# soroban-forge-spec

`soroban-forge spec` ? prints the interface of a built contract or deployed
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

- `<contract-id>` ? deployed contract ID (C?) to fetch and read from
- `--format <rust|text|json|md|markdown>` ? output format (default: rust, or json with `--json`)
- `--path <dir>` ? contract project directory [default: current directory]
- `--wasm <path>` ? path to local built wasm file
- `--network <name>` ? configured network for fetching deployed contracts [default: testnet]
- `--rpc-url <url>` ? Stellar RPC endpoint URL (overrides network default)
- `--network-passphrase <pass>` ? Stellar network passphrase

When reading local wasm files, `spec` never touches the network and works under `--offline`.
When given a contract ID, `spec` fetches the deployed wasm via `stellar contract fetch`;
under `--offline` this fails cleanly before attempting any network calls.

## Markdown Format

`spec --format md` emits documentation-ready Markdown tables for embedding directly
into READMEs:
- An **Entrypoints** table listing functions, argument types, and return types.
- A **Custom Types** section listing structs, enums, error enums and unions
  referenced by the contract's entrypoints.

## Public surface

- `validate_contract_id(id)` ? validate contract ID format
- `read_crate_name(dir)` / `locate_wasm(dir, crate_name)` ? where the release build lands
- `resolve_wasm(dir, wasm_override)` ? the wasm the spec is read from
- `SpecFormat` (`Rust` / `Json` / `Markdown`) ? output format enum
- `render_markdown_spec(json)` ? convert spec JSON to GitHub Markdown tables
- `dump_interface_from_wasm(wasm, format)` ? extract interface from wasm
- `dump_interface(dir, wasm_override, format)` ? programmatic API
- `SpecPlugin` ? the `ForgePlugin` impl

## Tests

```sh
cargo test -p soroban-forge-spec
```
