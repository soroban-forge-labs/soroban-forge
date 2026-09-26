# soroban-forge-bindings-py

`soroban-forge bindings-py` — generates `client.py`: a typed Python client
from a built contract's wasm, one method per entrypoint. Mirrors
`soroban-forge-bindings-ts`'s public surface
(`read_crate_name`/`locate_wasm`/`generate_bindings`,
`--path`/`--wasm`/`--output`/`--force`), local and offline.

## Why this isn't a `stellar-cli` wrapper

Every other soroban-forge module that reads a contract's interface shells
out to the official `stellar` CLI and never reimplements XDR/spec decoding.
That isn't possible here: `stellar contract bindings python` is
unimplemented — it prints a message pointing at a third-party tool,
[`stellar-contract-bindings`](https://github.com/lightsail-network/stellar-contract-bindings)
on PyPI, whose `python` command requires `--contract-id` and `--rpc-url`
and only works against an already-*deployed* contract. That's a different
model from every other soroban-forge subcommand (and from `bindings ts`),
which read a locally *built* wasm and need no network.

So this crate reads the interface the same official way `spec`/`verify`/
`bindings ts --react` do — `stellar contract info interface --output json`
— and renders the Python source itself. The generated client's *runtime*
(building, simulating, signing and submitting transactions; encoding and
decoding `SCVal`s) is never reimplemented: every method delegates to the
official `stellar-sdk` PyPI package's
`stellar_sdk.contract.ContractClient`/`AssembledTransaction` and
`stellar_sdk.scval`. What's generated is only the thin per-contract,
per-method surface.

## Generate

```sh
stellar contract build
soroban-forge bindings-py
pip install stellar-sdk
python -c 'from client import Client'
```

```python
from client import Client

client = client.Client(
    contract_id="C...",
    rpc_url="https://soroban-testnet.stellar.org",
    network_passphrase="Test SDF Network ; September 2015",
)
tx = client.balance(id="G...")
print(tx.result)
```

## What gets generated

- One dataclass per `#[contracttype] struct`, with `_decode`/`_encode`
  staticmethod/method pairs.
- One dataclass per tagged-enum case (`#[contracttype] enum`), named
  `<EnumName>_<Variant>`, plus a `Union[...]` alias and module-level
  `_decode_<Name>`/`_encode_<Name>` dispatchers. A variant's associated data
  is always a `values: tuple[...]` field, whatever its arity, mirroring the
  TypeScript generator's `{tag, values}` shape.
- One `IntEnum` per plain or error enum (`#[contracterror]`) — both spec
  kinds share the same shape and both render this way.
- One method per entrypoint on `Client`, typed by argument and return
  type, delegating to `self._client.invoke(...)`.
- `__constructor` is excluded — a deployed contract cannot be
  re-constructed, same reasoning as `bindings ts --react`'s hooks.
- A `py.typed` marker (PEP 561) alongside `client.py`.

### Known simplifications

- **`Result<T, E>` renders as plain `T`.** On the Soroban host, a contract
  function returning `Result<T, E>` traps on `Err` rather than returning a
  decodable error value, so `E` never appears on the wire for a successful
  call — the error path is already an exception `stellar-sdk` raises, not a
  value this code would decode.
- **`muxed_address` is treated as `address`** (`stellar_sdk.Address`, via
  `scval.from_address`/`to_address`): `stellar-sdk` 16.x has no dedicated
  muxed-address `scval` helper.
- **A spec name that collides with a Python keyword is escaped** with a
  trailing underscore (`from` -> `from_`) — this actually happens: a
  token's `transfer(from, to, amount)` has `from` as an argument name.

## Public surface

- `read_crate_name(dir)` / `locate_wasm(dir, crate_name)` — where the
  release build lands
- `generate_bindings(contract_dir, wasm_override, output, force)` — the
  programmatic API behind `bindings-py`
- `BindingsPyPlugin` — the `ForgePlugin` impl

## Why `bindings-py`, not `bindings py`

`soroban-forge-core`'s plugin dispatch maps one top-level subcommand name to
exactly one plugin (`crates/core/src/cli.rs`), and `soroban-forge-bindings-ts`
already owns `bindings` (with `ts` as its only sub-subcommand). Adding `py`
there would mean this crate depending on `bindings-ts`, which the "modules
never depend on each other" rule in `CONTRIBUTING.md` rules out. So this is
its own top-level command instead.

## Testing

```sh
cargo test -p soroban-forge-bindings-py
```

Tests never shell out to the real `stellar` binary — the pre-flight checks
(missing wasm, existing output dir), spec parsing, and every type/expression
renderer are covered directly on constructed spec JSON.

The renderer was additionally validated by hand against real contracts
covering every supported construct — primitives (`token`, `nft` templates),
structs and tagged enums with void/single/multi-arity cases (`governance`
template), and error enums (`amm` template) — checked with `mypy --strict`
(zero errors) and with real `stellar-sdk` encode/decode round-trips of every
generated struct and enum shape.
