#!/usr/bin/env bash
# Verify all deployed contracts match the compiled WASM.
# Usage: ./scripts/verify-all.sh testnet

set -euo pipefail

NETWORK=${1:-testnet}
ENV_FILE=".contracts.$NETWORK.env"

if [ ! -f "$ENV_FILE" ]; then
  echo "Error: $ENV_FILE not found. Run deploy-all.sh first."
  exit 1
fi

source "$ENV_FILE"

for CONTRACT in token staking distribution; do
  CONTRACT_ID=${!CONTRACT:-}
  if [ -z "$CONTRACT_ID" ]; then
    echo "Skipping $CONTRACT (not deployed)"
    continue
  fi
  echo "Verifying $CONTRACT ($CONTRACT_ID)..."
  forge verify --contract "$CONTRACT" --contract-id "$CONTRACT_ID" --network "$NETWORK"
  echo "  ✅ $CONTRACT verified"
done
