#!/usr/bin/env bash
set -euo pipefail
echo "Installing Soroban Forge dependencies..."
rustup target add wasm32-unknown-unknown
npm install -g soroban-forge
echo "Done."
