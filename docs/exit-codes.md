# Exit codes

`soroban-forge` uses a small, stable set of process exit codes so CI
pipelines and scripts can branch on outcome without parsing error text.
This mapping is part of the public contract — changes to it are breaking
changes and will be called out in the changelog.

| code | meaning        | examples                                                                 |
|------|----------------|---------------------------------------------------------------------------|
| `0`  | success        | the subcommand completed without error                                    |
| `1`  | user error     | invalid arguments, a bad `forge.toml`, an unknown template, an output path that already exists without `--force` |
| `2`  | tool missing   | `stellar-cli` not found, `rustc`/`cargo` missing or below the minimum version, the `wasm32v1-none` target not installed — anything `soroban-forge doctor` would flag, or a plugin (e.g. `bindings ts`) failing because it couldn't find the external tool it wraps |
| `3`  | internal error | an I/O failure (can't read/write a file), or anything not classified above |

## Using it in scripts

```sh
soroban-forge bindings ts
case $? in
  0) echo "bindings generated" ;;
  1) echo "check your arguments/config" >&2; exit 1 ;;
  2) echo "missing a required tool — run: soroban-forge doctor" >&2; exit 1 ;;
  3) echo "internal error — please file an issue" >&2; exit 1 ;;
esac
```

## Consistency across subcommands

Every subcommand returns a [`ForgeError`](../crates/core/src/error.rs) on
failure, and the exit code is derived from the error variant in one place
(`ForgeError::exit_code`, called once by the `soroban-forge` binary) rather
than by each plugin calling `std::process::exit` itself. That's what keeps
the codes consistent as new subcommands are added — see
[CONTRIBUTING.md](../CONTRIBUTING.md) if you're implementing a new plugin.