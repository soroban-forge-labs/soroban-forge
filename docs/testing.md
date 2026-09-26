# Testing

Forge ships with a first-class test runner:

```sh
forge test
forge test --watch
forge test --coverage
```

## Localnet integration test

`soroban-forge test-init --localnet` adds `tests/forge_localnet.rs`, an ignored
integration test that builds the contract, deploys it, then invokes the first
detected entrypoint. It uses the Stellar CLI and expects a local Soroban
quickstart network to be running with RPC at `http://localhost:8000/soroban/rpc`.
The default source identity is `alice`; configure and fund that identity for
your local network before running the test.

```sh
soroban-forge test-init --localnet
cargo test --test forge_localnet -- --ignored
```

The generated test is ignored so normal `cargo test` remains offline. Override
the source, RPC endpoint, passphrase, entrypoint, or CLI arguments with
`FORGE_LOCALNET_SOURCE`, `FORGE_LOCALNET_RPC_URL`,
`FORGE_LOCALNET_PASSPHRASE`, `FORGE_LOCALNET_FUNCTION`,
`FORGE_LOCALNET_DEPLOY_ARGS`, and `FORGE_LOCALNET_ARGS`. Argument overrides
are whitespace-separated Stellar CLI arguments passed after `--`.
