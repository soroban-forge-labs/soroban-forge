# Subscription Payments Example

An on-chain subscription contract that charges a fixed fee per billing period.

```sh
forge init my-subscription --template subscription
```

## Features

- Subscriber pre-authorises recurring payments
- Owner can call `charge(subscriber)` once per period
- Subscriber can cancel at any time

## Key Functions

| Function | Description |
|----------|-------------|
| `subscribe(period_secs, amount)` | Create a subscription |
| `charge(subscriber)` | Collect the periodic fee |
| `cancel()` | Terminate subscription |
