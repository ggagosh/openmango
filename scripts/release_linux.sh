#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP_NAME="OpenMango"
VERSION="$(grep '^version = ' "$ROOT_DIR/Cargo.toml" | head -1 | cut -d '"' -f2)"

DIST_DIR="$ROOT_DIR/dist"
BIN_PATH="$ROOT_DIR/target/release/openmango"
RELEASE_DIR="$DIST_DIR/${APP_NAME}-${VERSION}-linux"
TAR_PATH="$DIST_DIR/${APP_NAME}-${VERSION}-linux.tar.gz"

mkdir -p "$DIST_DIR"

cargo build --release --features mimalloc

rm -rf "$RELEASE_DIR"
mkdir -p "$RELEASE_DIR"
cp "$BIN_PATH" "$RELEASE_DIR/$APP_NAME"
chmod +x "$RELEASE_DIR/$APP_NAME"

rm -f "$TAR_PATH"
tar -czf "$TAR_PATH" -C "$DIST_DIR" "$(basename "$RELEASE_DIR")"

echo "Built: $RELEASE_DIR"
echo "Packaged: $TAR_PATH"
