#!/usr/bin/env bash
# Deploy all contracts in the workspace to the specified network.
# Usage: ./scripts/deploy-all.sh testnet

set -euo pipefail

NETWORK=${1:-testnet}
echo "Deploying all contracts to $NETWORK..."

forge build --all

CONTRACTS=("token" "staking" "distribution")

for CONTRACT in "${CONTRACTS[@]}"; do
  echo "Deploying $CONTRACT..."
  CONTRACT_ID=$(forge deploy --contract "$CONTRACT" --network "$NETWORK" --json | jq -r '.contractId')
  echo "  $CONTRACT deployed at $CONTRACT_ID"
  echo "$CONTRACT=$CONTRACT_ID" >> ".contracts.$NETWORK.env"
done

echo "Done. Contract IDs written to .contracts.$NETWORK.env"
