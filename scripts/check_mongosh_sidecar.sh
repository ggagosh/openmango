#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SIDECAR_DIR="$ROOT_DIR/tools/forge-sidecar"

if ! command -v bun >/dev/null 2>&1; then
  echo "Bun is required. Install from https://bun.sh" >&2
  exit 1
fi

# Verify sidecar source compiles without errors (bundle-only, no binary output)
cd "$SIDECAR_DIR"

if [ ! -d node_modules ]; then
  bun install
fi

TMP_OUT="$(mktemp)"
trap 'rm -f "$TMP_OUT"' EXIT

bun build ./src/bun-entry.ts \
  --target bun \
  --outfile "$TMP_OUT" \
  --external electron \
  --external os-dns-native \
  --external kerberos \
  --external mongodb-client-encryption \
  --external ssh2 \
  --external cpu-features \
  --external pac-proxy-agent \
  --external @babel/preset-typescript/package.json \
  >/dev/null

echo "Forge sidecar bundle OK"
