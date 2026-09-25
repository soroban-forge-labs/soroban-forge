# soroban-forge-bindings-ts

`soroban-forge bindings ts` — generates a TypeScript client package from a
built contract's wasm.

Wraps the official `stellar contract bindings typescript` — this module
never reimplements XDR-spec-to-TypeScript generation itself.

## A publishable package

After the CLI generates the package, `package.json` is rewritten so the
output can be `npm pack`ed or published without edits:

| field              | value                                                                   |
|--------------------|-------------------------------------------------------------------------|
| `name`, `version`  | from `[package]` in `Cargo.toml` (kept from the CLI when unknown)        |
| `exports`          | `{ ".": { "types", "import", "default" }, "./package.json" }`            |
| `main`, `types`    | `./dist/index.js`, `./dist/index.d.ts` for tools that predate `exports`  |
| `files`            | `dist`, `src`, `README.md`                                              |
| `scripts.prepack`  | `npm run build`, so a pack never ships a stale or missing `dist/`        |
| `peerDependencies` | `@stellar/stellar-sdk` (also kept in `devDependencies` for the build)   |
| `engines.node`     | `>=18`                                                                  |

The declarations resolve under both `node16`/`nodenext` and `bundler`
module resolution.

**Supported Stellar SDK range.** The generated client is written against
the SDK version the installed `stellar-cli` targets, so the peer range is
the one the CLI emits: `^16` for stellar-cli 28. If a future CLI stops
pinning the SDK, the fallback is `DEFAULT_STELLAR_SDK_RANGE` (`^16.0.0`).
Apps install the SDK themselves, which keeps a single SDK instance in the
bundle:

```sh
cd bindings/typescript
npm install
npm pack          # runs the build, then packs dist/, src/ and README.md
```

## CLI Options

- `--path <dir>` — contract project directory [default: current directory]
- `--wasm <path>` — path to built wasm file [default: target/wasm32v1-none/release/<crate>.wasm]
- `--out-dir <dir>`, `-o` (alias: `--output`) — directory to write generated bindings to [default: `bindings/typescript`]
- `--package-name <name>` — custom legal npm package name overriding the default derived from `Cargo.toml`
- `--force` — overwrite the output directory if it exists
- `--watch` — re-generate bindings on contract source changes
- `--react` — see below

## `--react`: generated hooks

```sh
soroban-forge bindings ts --react
```

Strictly opt-in — without the flag nothing changes and `react` is never
mentioned in the generated package. With it, `src/hooks.ts` gets one hook per
entrypoint (`__constructor` excluded — a deployed contract cannot be
re-constructed), typed against the generated `Client` via TypeScript's
`Parameters`/`ReturnType` rather than duplicating its types, so the hooks
always match whatever `stellar-cli` generated:

- a **read** entrypoint gets `useXxx(client, ...args)`, a query-style hook
  that fetches on mount/args-change and returns `{ data, loading, error,
  refetch }`
- a **write** entrypoint gets `useXxxMutation(client)`, which never runs on
  its own and returns `{ mutate, data, loading, error }` — `mutate(...)`
  builds, simulates, signs and sends the transaction

Soroban's interface carries no read/write (view/mutating) annotation the way
an ABI's `stateMutability` would, so the split is a naming heuristic:
`get_`/`is_`/`has_`/`list_`/`view_`/`query_`/`read_` prefixes and a fixed list
of common getters (`balance`, `owner`, `admin`, `name`, `symbol`, …) are
reads; everything else is a write (the safer default — a write hook never
auto-runs). See `looks_like_read` if a contract's naming needs a different
call.

`package.json` gains `react` as an **optional** peer dependency (plus
`@types/react` in `devDependencies` for the build) and a `./hooks` export
subpath — consumers who only use the plain client are unaffected. A "React
hooks" section with a usage example is appended to the generated README.

## Public surface

- `validate_npm_package_name(name)` — validates npm package names according to npm rules
- `read_package_info(dir)` — reads `[package].name` and `version` from `Cargo.toml`
- `locate_wasm(dir, crate_name)` — the default build output path under `target/wasm32v1-none/release/`
- `generate_bindings(contract_dir, wasm_override, output, force)` — standard bindings generator
- `generate_bindings_with_options(contract_dir, wasm_override, output, package_name, react, force)` — the full bindings generator behind `bindings ts`
- `make_publishable(package_json, info)` — package.json rewrite
- `make_publishable_with_name(package_json, info, package_name)` — package.json rewrite with custom name
- `render_hooks_ts(entrypoints)` — renders `src/hooks.ts` for `--react`
- `BindingsTsPlugin` — the `ForgePlugin` impl

## Testing

```sh
cargo test -p soroban-forge-bindings-ts
```

Tests never shell out to the real `stellar` binary — the pre-flight checks
(missing wasm, existing output dir) and the `package.json` rewrite are
covered directly.

CI additionally generates real `--react` bindings for the `token` and `nft`
templates and type-checks them with a pinned `tsc` — see
`.github/workflows/bindings-typecheck.yml`. That job is what actually
compiles the generated output; it catches regressions this crate's own
unit tests (which never invoke `stellar` or `tsc`) cannot.