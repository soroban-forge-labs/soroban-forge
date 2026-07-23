#!/usr/bin/env bash
set -euo pipefail
VERSION=$1
echo "Releasing v$VERSION..."
git tag -a "v$VERSION" -m "Release v$VERSION"
git push origin "v$VERSION"
echo "Tagged v$VERSION"
