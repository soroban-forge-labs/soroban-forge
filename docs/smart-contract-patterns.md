# Smart Contract Patterns

## Access Control
Use `require_auth()` on any address that should authorise an action.

## Storage Keys
Use typed enums as storage keys to avoid collisions.

## Error Handling
Return `Result<T, ContractError>` from all public functions.
