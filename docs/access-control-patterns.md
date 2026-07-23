# Access Control Patterns

## 1. Single Admin

```rust
fn admin_only(env: &Env) {
    let admin: Address = env.storage().instance().get(&DataKey::Admin).unwrap();
    admin.require_auth();
}
```

## 2. Role-Based (Manager + Admin)

```rust
fn require_manager_or_admin(env: &Env, caller: &Address) {
    let admin: Address = storage::get_admin(env);
    let manager: Address = storage::get_manager(env);
    if *caller != admin && *caller != manager {
        panic!("Unauthorized");
    }
    caller.require_auth();
}
```

## 3. Two-Step Ownership Transfer

```rust
pub fn propose_admin(env: Env, new_admin: Address) {
    storage::get_admin(&env).require_auth();
    storage::set_pending_admin(&env, &new_admin);
}

pub fn accept_admin(env: Env) {
    let pending = storage::get_pending_admin(&env).expect("No pending admin");
    pending.require_auth();
    storage::set_admin(&env, &pending);
    storage::clear_pending_admin(&env);
}
```
