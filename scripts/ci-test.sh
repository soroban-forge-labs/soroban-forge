#!/usr/bin/env bash
set -euo pipefail
echo "Running CI test suite..."
cargo fmt --all -- --check
cargo clippy -- -D warnings
cargo test
echo "All checks passed."
