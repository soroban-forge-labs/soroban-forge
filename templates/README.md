# Contract templates

Each subdirectory is a template consumable by `soroban-forge new --template <name>`.
Owned by Module 2 (scaffolding) — see [`crates/scaffold`](../crates/scaffold)
for the template format and how to add a new one.

- `hello-world` — minimal greeter contract
- `token` — SEP-41 fungible token (`soroban_sdk::token::TokenInterface`)
- `crowdfund` — escrow/deadline crowdfunding example

Manifests are shipped as `Cargo.toml.hbs` so cargo doesn't treat these
directories as packages; the `.hbs` suffix is stripped when a project is
generated.
