# CI presets

Workflow templates consumed by `soroban-forge ci-init --provider <p>`.
Owned by Module 4 — see [`crates/ci-presets`](../crates/ci-presets).

Each provider is a subdirectory; `github/` is the only provider in v0.1:

- `build-test.yml` — cargo test + wasm build on push/PR
- `contract-size.yml` — fails PRs when the built wasm exceeds a size limit
- `testnet-deploy.yml` — manual testnet deploy wrapping the official
  stellar-cli (only written with `--deploy`); references GitHub secrets, never
  stores keys

Templates may use `{{project_name}}` / `{{crate_name}}`; GitHub's own
`${{ ... }}` expressions pass through rendering untouched.
