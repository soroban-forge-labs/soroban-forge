# forge.config.ts

Full schema reference for the Soroban Forge configuration file.

```typescript
import { defineConfig } from 'soroban-forge';

export default defineConfig({
  // Network to target when no --network flag is passed
  defaultNetwork: 'testnet',

  // Named network presets
  networks: {
    testnet: {
      rpcUrl: 'https://soroban-testnet.stellar.org',
      passphrase: 'Test SDF Network ; September 2015',
    },
    mainnet: {
      rpcUrl: 'https://horizon.stellar.org',
      passphrase: 'Public Global Stellar Network ; September 2015',
    },
  },

  // Contracts to build / deploy
  contracts: [
    { name: 'distribution', path: 'contracts/distribution' },
    { name: 'token',        path: 'contracts/token' },
  ],

  // Compiler settings
  compiler: {
    optimise: true,       // -C opt-level=z
    lto: true,            // link-time optimisation
    panicAbort: true,     // abort on panic (no unwinding)
  },
});
```
