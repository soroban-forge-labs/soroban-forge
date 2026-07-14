# Crowdfunding Example

A time-bounded crowdfunding contract that refunds backers if the goal is not met.

```sh
forge init my-crowdfund --template crowdfund
cd my-crowdfund
forge build && forge test
```

## Features

- Campaign creator sets a funding goal and deadline
- Backers pledge XLM or any SAC token
- If goal is met before deadline → funds released to creator
- If goal is missed → all backers can withdraw their pledges

## Key Functions

| Function | Description |
|----------|-------------|
| `pledge(backer, amount)` | Add to the campaign balance |
| `finalize()` | Release funds if goal met |
| `refund(backer)` | Return pledge if goal missed |
