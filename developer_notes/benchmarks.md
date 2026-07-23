# Benchmarks

Soroban enforces a per-transaction CPU instruction limit (~100M).

## Profiling

```rust
let (cpu, _mem) = env.budget().reset_default();
// run operation
let used = env.budget().cpu_instruction_count();
```

Keep hot paths under 10M instructions to leave headroom for composability.
