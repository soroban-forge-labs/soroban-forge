# Contract Upgrades

Soroban supports WASM upgrades via `env.deployer().update_current_contract_wasm()`.

## Migration Pattern

1. Deploy new WASM
2. Call `migrate()` to transform storage layout
3. Verify invariants
