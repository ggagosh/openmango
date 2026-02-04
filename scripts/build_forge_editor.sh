#!/usr/bin/env bash
set -euo pipefail

TMP_DIR="/tmp/openmango-cm6-build"

rm -rf "$TMP_DIR"
mkdir -p "$TMP_DIR"
cp assets/forge/editor.ts "$TMP_DIR/editor.ts"
cp assets/forge/mongosh-sidecar.ts "$TMP_DIR/mongosh-sidecar.ts"

cat > "$TMP_DIR/package.json" <<'JSON'
{
  "private": true,
  "type": "module",
  "dependencies": {
    "@codemirror/autocomplete": "^6.18.0",
    "@codemirror/commands": "^6.3.3",
    "@codemirror/lang-javascript": "^6.2.2",
    "@codemirror/language": "^6.10.1",
    "@codemirror/state": "^6.4.1",
    "@codemirror/view": "^6.26.3",
    "@mongosh/browser-runtime-electron": "^3.29.1",
    "@mongosh/service-provider-node-driver": "^3.18.1",
    "@lezer/highlight": "^1.2.1",
    "bson": "^6.7.0",
    "esbuild": "^0.20.2"
  }
}
JSON

echo "[forge-editor] installing npm deps (this can take a minute)..."
NPM_CONFIG_CACHE="/tmp/npm-cache" \
  npm --prefix "$TMP_DIR" install --no-fund --no-audit
echo "[forge-editor] bundling editor.js..."
"$TMP_DIR/node_modules/.bin/esbuild" "$TMP_DIR/editor.ts" \
  --bundle \
  --format=iife \
  --platform=browser \
  --target=es2019 \
  --outfile="assets/forge/editor.js" \
  --minify

echo "[forge-editor] bundling mongosh-sidecar.js..."
"$TMP_DIR/node_modules/.bin/esbuild" "$TMP_DIR/mongosh-sidecar.ts" \
  --bundle \
  --format=cjs \
  --platform=node \
  --target=node18 \
  --outfile="assets/forge/mongosh-sidecar.js" \
  --minify \
  --external:mongodb-client-encryption \
  --external:kerberos \
  --external:snappy \
  --external:@mongodb-js/zstd \
  --external:pac-proxy-agent \
  --external:electron \
  --external:cpu-features \
  --external:ssh2 \
  --external:@babel/preset-typescript/package.json \
  --external:aws4 \
  --external:socks \
  --external:gssapi
