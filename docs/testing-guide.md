# Testing Guide

## Unit Tests (Rust)

```rust
#[cfg(test)]
mod test {
    use super::*;
    use soroban_sdk::{testutils::Address as _, Env};

    #[test]
    fn test_initialize() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let contract_id = env.register(MyContract, ());
        let client = MyContractClient::new(&env, &contract_id);
        client.initialize(&admin);
        assert_eq!(client.get_admin(), admin);
    }
}
```

## Integration Tests (TypeScript)

```typescript
import { forge } from 'soroban-forge/testing';

describe('Distribution contract', () => {
  it('distributes rewards proportionally', async () => {
    const { client, mint } = await forge.deploy('distribution');
    await mint({ to: userA, amount: 1000n });
    await client.deposit({ user: userA, amount: 1000n });
    await client.distribute({ from: admin, amount: 4000n });
    expect(await client.getPending({ user: userA })).toBe(4000n);
  });
});
```
