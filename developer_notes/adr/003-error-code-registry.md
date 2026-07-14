# ADR-003: Global Error Code Registry

**Status:** Accepted  
**Date:** 2026-07-10

## Context

When multiple contracts are composed, overlapping error codes from different `#[contracterror]` enums make debugging difficult.

## Decision

Reserve error code ranges per contract type:

| Range   | Owner |
|---------|-------|
| 1–99    | Core / shared errors |
| 100–199 | Token contracts |
| 200–299 | Staking / distribution contracts |
| 300–399 | Governance contracts |
| 400–499 | Escrow / timelock contracts |

Forge's code generator assigns the correct range based on the chosen template.

## Consequences

- Errors are globally unique across a composed application
- Off-chain tooling can decode errors without knowing which contract emitted them
