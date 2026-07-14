# Examples

Checked-in output of `soroban-forge`, kept for browsing without installing the
tool. Owned by Module 5 (docs & DX).

- [`hello-forge/`](hello-forge) — the result of:

  ```sh
  soroban-forge new hello-forge --template hello-world
  soroban-forge test-init --force
  soroban-forge ci-init --deploy
  ```

These directories are excluded from the cargo workspace (they are standalone
projects). Regenerate them with the commands above after changing templates or
presets, so the examples never drift from the real output.
