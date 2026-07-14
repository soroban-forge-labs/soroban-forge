#!/usr/bin/env bash
# Seed testnet with test accounts and initial contract state.
# Requires: stellar-cli, forge

set -euo pipefail

echo "Creating test accounts..."
stellar-cli keys generate alice --network testnet
stellar-cli keys generate bob --network testnet
stellar-cli keys generate admin --network testnet

echo "Funding accounts via Friendbot..."
for ACCOUNT in alice bob admin; do
  ADDRESS=$(stellar-cli keys address $ACCOUNT)
  curl -s "https://friendbot.stellar.org?addr=$ADDRESS" > /dev/null
  echo "  Funded $ACCOUNT ($ADDRESS)"
done

echo "Deploying contracts..."
source ".contracts.testnet.env" 2>/dev/null || ./scripts/deploy-all.sh testnet

echo "Seeding initial state..."
forge script scripts/seed.ts --network testnet

echo "Testnet seed complete."
