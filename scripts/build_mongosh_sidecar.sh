#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SIDE_CAR_DIR="$ROOT_DIR/tools/forge-sidecar"

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

npm run build
