# Troubleshooting / FAQ

Common failures when setting up or using `soroban-forge`, and how to fix them.
Run `soroban-forge doctor` first — it catches most of these automatically.

## "error[E0463]: can't find crate" / build fails targeting wasm

**Symptom:** `cargo build --target wasm32v1-none` (or `stellar contract build`)
fails because the target isn't installed.

**Fix:**

```sh
rustup target add wasm32v1-none
```

If you're on an older toolchain that only knows the pre-2024 target name, add
that one instead and then upgrade:

```sh
rustup target add wasm32-unknown-unknown
rustup update
```

Confirm the fix:

```sh
soroban-forge doctor
```

## `ed25519-dalek` fails to build

**Symptom:** compiling a generated contract (or its dev-dependencies) errors
out inside `ed25519-dalek` or one of its transitive deps (`curve25519-dalek`,
`zeroize`), often with a message about incompatible editions or an `asm`
feature not being available.

**Fix:** this is almost always a Rust version mismatch — `ed25519-dalek`
(pulled in by `soroban-sdk`'s testing utils) requires a recent stable
toolchain.

```sh
rustup update stable
rustc --version   # should be >= 1.84
```

If you're pinned to an older toolchain for another project, use `rustup
override set stable` inside the generated project directory rather than
changing your global default.

If the error persists after updating, delete the lockfile and let cargo
re-resolve versions:

```sh
rm Cargo.lock
cargo build
```

## `stellar-cli` not found / `stellar: command not found`

**Symptom:** `soroban-forge new`, `doctor`, or the quickstart steps that
shell out to `stellar contract build` / `stellar contract deploy` fail
because the `stellar` binary isn't on your `PATH`.

**Fix:** install the CLI directly from crates.io:

```sh
cargo install --locked stellar-cli
```

Then verify it's discoverable:

```sh
stellar --version
```

If it's installed but still not found, check that cargo's bin directory
(usually `~/.cargo/bin`) is on your `PATH`.

## `doctor` warns about a known-bad `stellar-cli` version

**Symptom:** `soroban-forge doctor` prints a warning that the active
`stellar-cli` version is a known-bad release and recommends upgrading.

**Fix:** install a safe version from the latest stable release:

```sh
cargo install --locked stellar-cli
```

`doctor` keeps the denylist in one place so patching it for a new bad release
is straightforward; the warning includes the recommended replacement version.

## Still stuck?

Run `soroban-forge doctor` and include its full output when opening an issue —
it reports your rustc/cargo version, installed targets, and whether
`stellar-cli` is on `PATH`, which covers most of what maintainers will ask for.