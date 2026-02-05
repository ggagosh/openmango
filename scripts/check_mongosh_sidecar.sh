#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SIDE_CAR_DIR="$ROOT_DIR/tools/forge-sidecar"
SRC="$ROOT_DIR/tools/forge-sidecar/src/sidecar.ts"
OUT="$ROOT_DIR/assets/forge/mongosh-sidecar.js"
TMP_OUT="$ROOT_DIR/target/mongosh-sidecar.check.js"

if ! command -v node >/dev/null 2>&1; then
  echo "Node.js is required. Run 'just download-node' or install Node.js." >&2
  exit 1
fi

if ! command -v npm >/dev/null 2>&1; then
  echo "npm is required to build the Forge sidecar bundle." >&2
  exit 1
fi

cd "$SIDE_CAR_DIR"

if [ ! -d node_modules ]; then
  npm install
fi

./node_modules/.bin/esbuild "$SRC" --bundle --platform=node --format=cjs --target=node18 --external:*.node --external:electron --external:kerberos --external:mongodb-client-encryption --external:ssh2 --external:cpu-features --external:pac-proxy-agent --external:@babel/preset-typescript/package.json --outfile="$TMP_OUT" >/dev/null

if ! cmp -s "$TMP_OUT" "$OUT"; then
  echo "Forge sidecar bundle is out of date. Run: ./scripts/build_mongosh_sidecar.sh" >&2
  exit 1
fi
