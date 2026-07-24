# CLI Reference

## Global options

- `--quiet`, `-q` — suppress informational command output; errors and exit
  codes are unchanged.
- `--verbose`, `-v` — enable debug logging.

Global options may appear before or after a subcommand and can be combined.

## Commands

- `soroban-forge new <name> --template <t>` — create a contract project.
- `soroban-forge templates` — list all bundled contract templates with descriptions.
- `soroban-forge test-init` — generate a test harness.
- `soroban-forge ci-init --provider github` — generate CI workflows.
- `soroban-forge doctor [--json]` — check the local Soroban toolchain (optionally emitting machine-readable JSON).
