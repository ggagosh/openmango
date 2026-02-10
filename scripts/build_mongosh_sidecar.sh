#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SIDECAR_DIR="$ROOT_DIR/tools/forge-sidecar"

if ! command -v bun >/dev/null 2>&1; then
  echo "Bun is required to build the Forge sidecar. Install from https://bun.sh" >&2
  exit 1
fi

# Determine architecture-specific output directory
ARCH="$(uname -m)"
OS="$(uname -s)"
if [[ "$OS" == "Darwin" ]]; then
  if [[ "$ARCH" == "arm64" ]]; then
    ARCH_DIR="macos-arm64"
  else
    ARCH_DIR="macos-x86_64"
  fi
elif [[ "$OS" == "Linux" ]]; then
  ARCH_DIR="linux-x86_64"
else
  echo "Unsupported OS: $OS" >&2
  exit 1
fi

OUT_DIR="$ROOT_DIR/resources/bin/$ARCH_DIR"
mkdir -p "$OUT_DIR"

cd "$SIDECAR_DIR"

if [ ! -d node_modules ]; then
  bun install
fi

bun build ./src/bun-entry.ts --compile \
  --outfile "$OUT_DIR/mongosh-sidecar" \
  --external electron \
  --external os-dns-native \
  --external kerberos \
  --external mongodb-client-encryption \
  --external ssh2 \
  --external cpu-features \
  --external pac-proxy-agent \
  --external @babel/preset-typescript/package.json

echo "Built mongosh-sidecar → $OUT_DIR/mongosh-sidecar"
