# soroban-forge documentation

## Getting Started

- **[Getting Started](getting-started.md)** — install Forge and scaffold your first project.
- **[Quickstart](quickstart.md)** — five-minute walkthrough from zero to a deployed testnet contract.
- **[Tutorial: zero to deployed testnet contract](tutorial-zero-to-testnet.md)** — the full walkthrough for newcomers, no prior Soroban knowledge required.
- **[Environment](environment.md)** — configure `.forge.env` for RPC URLs, secrets, and target network.

## Configuration & CLI

- **[Configuration](configuration.md)** — full `forge.config.ts` schema reference.
- **[CLI Reference](cli-reference.md)** — every command, flag, and global option.
- **[Quiet mode](quiet-mode.md)** — suppressing informational output with `--quiet`.
- **[Network Configuration](network-config.md)** — RPC endpoints and passphrases for testnet and mainnet.
- **[Plugins](plugins.md)** — installing and using the Forge plugin ecosystem.

## Building & Deploying

- **[Deployment](deployment.md)** — `forge deploy` to testnet and mainnet.
- **[Multi-Contract Workspaces](multi-contract.md)** — Cargo workspace layouts, building all contracts, cross-contract calls.
- **[Contract Verification](contract-verification.md)** — verify on-chain WASM against source for public auditability.
- **[Upgrades](upgrades.md)** — WASM upgrades and storage migration patterns.
- **[Fee Model](fee-model.md)** — base, resource, and inclusion fees; estimating costs with `forge estimate`.
- **[Templates](templates.md)** — built-in contract templates (token, NFT, DAO, staking, multisig).
- **[Generated TypeScript Bindings](generated-bindings.md)** — auto-generated type-safe clients from your contract ABI.

## Testing & Debugging

- **[Testing](testing.md)** — `forge test`, watch mode, and coverage.
- **[Testing Guide](testing-guide.md)** — unit tests in Rust and integration tests in TypeScript with examples.
- **[Debugging](debugging.md)** — local simulation, reading events, and common VM errors.
- **[Troubleshooting / FAQ](troubleshooting.md)** — fixes for common setup and build failures.
- **[FAQ](faq.md)** — frequently asked questions about templates, Rust support, and more.

## Contract Development

- **[Smart Contract Patterns](smart-contract-patterns.md)** — access control, typed storage keys, and error handling patterns.
- **[Authentication](auth.md)** — Soroban's invocation-tree authorisation model and `require_auth()`.
- **[Access Control Patterns](access-control-patterns.md)** — single admin, role-based, and two-step ownership transfer.
- **[Error Handling](error-handling.md)** — typed error enums with `#[contracterror]` and `Result`-returning entrypoints.
- **[Storage](storage.md)** — persistent, temporary, and instance durability types.
- **[Storage TTL Management](storage-ttl.md)** — rent mechanics, TTL bump pattern, and durability comparison table.
- **[Events](events.md)** — publishing events with `env.events().publish()`.
- **[Events Reference](events-reference.md)** — event structure, topic design, RPC querying, and best practices.
- **[Interoperability](interoperability.md)** — calling other contracts and passing auth entries for sub-calls.
- **[Contract Upgrades](upgrades.md)** — WASM upgrade flow and migration pattern.

## Best Practices & Tips

- **[Best Practices](best-practices.md)** — input validation, integer arithmetic, event emission, and test coverage guidelines.
- **[Soroban SDK Tips](soroban-sdk-tips.md)** — practical tips for types, TTLs, and `#[contracttype]`.
- **[Tooling](tooling.md)** — recommended Rust, Soroban, JS, and CI tools.

## Architecture & Reference

- **[Architecture](architecture.md)** — how the five modules fit together.
- **[Exit Codes](exit-codes.md)** — the stable exit-code contract for CI/scripts.
- **[Contributing](contributing.md)** — setup and contribution guidelines.

---

Per-module reference docs live next to the code, one README per module:

| module | doc |
|--------|-----|
| 1 — CLI core | [crates/core/README.md](../crates/core/README.md) |
| 2 — Scaffolding & templates | [crates/scaffold/README.md](../crates/scaffold/README.md) · [templates/README.md](../templates/README.md) |
| 3 — Test harness generator | [crates/testgen/README.md](../crates/testgen/README.md) |
| 4 — CI/CD presets | [crates/ci-presets/README.md](../crates/ci-presets/README.md) · [presets/README.md](../presets/README.md) |
| 5 — Doctor & DX | [crates/doctor/README.md](../crates/doctor/README.md) |

## Reference

- **[Glossary](../developer_notes/glossary.md)** — definitions for Soroban and soroban-forge terms (WASM, Ledger, TTL, SAC, XDR, and more).
