# soroban-forge vs stellar-cli — What Forge Adds

`soroban-forge` is a **scaffolding, test-harness and CI toolkit** that wraps the official [`stellar-cli`](https://github.com/stellar/stellar-cli).  It never reimplements what the CLI already does well; instead it automates the repetitive work that sits *around* the CLI.

---

## Side-by-side task map

| Task | stellar-cli alone | soroban-forge |
|---|---|---|
| **Build a contract to WASM** | `stellar contract build` | **Delegates** to `stellar contract build` |
| **Deploy a contract** | `stellar contract deploy` | **Delegates** to `stellar contract deploy` (with auto-build if needed) |
| **Invoke a contract function** | `stellar contract invoke` | **Delegates** to `stellar contract invoke` (with friendlier output) |
| **Generate TS bindings** | `stellar contract bindings typescript` | **Delegates** to `stellar contract bindings typescript` |
| **Scaffold a new project** | Manual — copy an example repo | `soroban-forge new` — tested, documented template |
| **Add tests to an existing contract** | Manual — write from scratch | `soroban-forge test-init` — generates fixtures, smoke tests, snapshot helpers |
| **Add CI workflows** | Manual — write YAML | `soroban-forge ci-init` — GitHub, GitLab, CircleCI, Bitbucket |
| **Verify local build matches deployed contract** | Manual — compare hashes | `soroban-forge verify` — automated hash comparison |
| **Check toolchain health** | Manual — run multiple commands | `soroban-forge doctor` — single command, auto-fix with `--fix` |
| **Print contract spec** | `stellar contract inspect` | `soroban-forge spec` — same data, formatted for readability |

---

## Key principle: wrapping, not reimplementing

- **Build and deploy** always shell out to `stellar-cli`.  Forge does not contain its own WASM compiler or deploy logic.
- **Bindings generation** calls `stellar contract bindings typescript` under the hood.
- **Contract invocation** delegates to `stellar contract invoke` and formats the result.

What forge *does* add is:

1. **Project scaffolding** — tested templates with working tests out of the box.
2. **Test automation** — generating fixtures, smoke tests, and snapshot helpers from an existing contract.
3. **CI automation** — writing provider-specific YAML workflows with best-practice defaults.
4. **Developer experience** — `doctor` for toolchain verification, `verify` for build/deploy parity checks, and structured JSON-logs for CI debugging.
