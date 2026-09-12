#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SIDECAR_DIR="$ROOT_DIR/tools/forge-sidecar"
source "$ROOT_DIR/scripts/lib.sh"

if ! command -v bun >/dev/null 2>&1; then
  echo "Bun is required to build the Forge sidecar. Install from https://bun.sh" >&2
  exit 1
fi

openmango_platform "${1:-}"
OUT_DIR="$ROOT_DIR/resources/bin/$OPENMANGO_ARCH_DIR"
mkdir -p "$OUT_DIR"

cd "$SIDECAR_DIR"

bun install --frozen-lockfile

# Keep bytecode shallow: most of the startup gain without embedding every mongosh function.
bun build ./src/bun-entry.ts --compile --format=esm \
  --minify --keep-names --sourcemap --bytecode --bytecode-depth=1 \
  --target "$OPENMANGO_BUN_TARGET" \
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
