# soroban-forge documentation

- **[Tutorial: zero to deployed testnet contract](tutorial-zero-to-testnet.md)** —
  the full walkthrough for newcomers, no prior Soroban knowledge required.
- **[Architecture](architecture.md)** — how the five modules fit together.
- **[SDK Upgrade & Migration Guide](upgrade-guide.md)** — how to migrate generated projects across SDK major versions.
- **[Troubleshooting / FAQ](troubleshooting.md)** — fixes for common setup
  and build failures.
- **[Exit codes](exit-codes.md)** — the stable exit-code contract for CI/scripts.

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
