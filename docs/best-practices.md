# Best Practices

1. **Validate all inputs** before touching storage
2. **Never divide before checking for zero denominator**
3. **Use `i128` for all financial arithmetic** — never floating point
4. **Emit events** for every state transition
5. **Write tests first** — aim for 100% branch coverage
6. **Bump TTLs** on every persistent read in hot paths
7. **Separate storage helpers** from business logic
