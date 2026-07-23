# soroban-forge-bindings-ts

`soroban-forge bindings ts` — generates a TypeScript client package from a
built contract's wasm.

Wraps the official `stellar contract bindings typescript` — this module
never reimplements XDR-spec-to-TypeScript generation itself.

## Public surface

- `read_package_info(dir)` — reads `[package].name` from `Cargo.toml`
- `locate_wasm(dir, crate_name)` — the default build output path under
  `target/wasm32v1-none/release/`
- `generate_bindings(contract_dir, wasm_override, output, force)` — the
  programmatic API behind `bindings ts`
- `BindingsTsPlugin` — the `ForgePlugin` impl

## Testing

```sh
cargo test -p soroban-forge-bindings-ts
```

Tests never shell out to the real `stellar` binary — only the pre-flight
checks (missing wasm, existing output dir) are covered directly.