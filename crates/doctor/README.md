# soroban-forge-doctor (Module 5, CLI part)

Environment checks. **Owner: Person E** (together with [`/docs`](../../docs)
and [`/examples`](../../examples)).

Implements `soroban-forge doctor`, which verifies:

| check                  | severity | fix printed                          |
|------------------------|----------|--------------------------------------|
| `rustc` ≥ 1.84         | fail     | `rustup update stable` / rustup.rs   |
| `cargo`                | fail     | rustup.rs                            |
| `wasm32v1-none` target | fail     | `rustup target add wasm32v1-none`    |
| `stellar` CLI          | fail     | `brew install stellar-cli` / cargo   |
| `git`                  | warn     | git-scm.com                          |

Exits non-zero when any required check fails, so it can gate CI or setup
scripts.

The check logic (`version_at_least`, `format_report`) is pure and unit-tested;
only the thin `capture()` helper touches the system.

## Tests

`cargo test -p soroban-forge-doctor`
