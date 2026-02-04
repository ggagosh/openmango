#!/usr/bin/env bash
# Download Node.js runtime for local development and bundling
set -euo pipefail

NODE_VERSION="24.13.0"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RESOURCES_DIR="$SCRIPT_DIR/../resources/bin"

# Detect architecture and OS
ARCH=$(uname -m)
OS=$(uname -s)

if [ "$OS" = "Darwin" ]; then
    if [ "$ARCH" = "arm64" ]; then
        NODE_ARCH="macos-arm64"
        NODE_FILE="node-v${NODE_VERSION}-darwin-arm64.tar.gz"
    else
        NODE_ARCH="macos-x86_64"
        NODE_FILE="node-v${NODE_VERSION}-darwin-x64.tar.gz"
    fi
elif [ "$OS" = "Linux" ]; then
    NODE_ARCH="linux-x86_64"
    NODE_FILE="node-v${NODE_VERSION}-linux-x64.tar.xz"
else
    echo "Unsupported OS: $OS"
    exit 1
fi

NODE_URL="https://nodejs.org/dist/v${NODE_VERSION}/${NODE_FILE}"
DEST_DIR="$RESOURCES_DIR/$NODE_ARCH"

echo "Downloading Node.js ${NODE_VERSION} for ${NODE_ARCH}..."
mkdir -p "$DEST_DIR"

# Create temp directory for download
TMP_DIR=$(mktemp -d)
trap 'rm -rf "$TMP_DIR"' EXIT

curl -fLs "$NODE_URL" -o "$TMP_DIR/node.tar"

if [[ "$NODE_FILE" == *.tar.gz ]]; then
    tar -xzf "$TMP_DIR/node.tar" -C "$TMP_DIR"
else
    tar -xJf "$TMP_DIR/node.tar" -C "$TMP_DIR"
fi

cp "$TMP_DIR"/node-v${NODE_VERSION}-*/bin/node "$DEST_DIR/node"
chmod +x "$DEST_DIR/node"

echo "Node.js installed to: $DEST_DIR"
echo "  - node: $("$DEST_DIR/node" --version 2>/dev/null | head -1 || echo 'installed')"
