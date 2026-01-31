#!/usr/bin/env bash
# Download MongoDB Database Tools for local development
set -euo pipefail

TOOLS_VERSION="100.14.1"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RESOURCES_DIR="$SCRIPT_DIR/../resources/bin"

# Detect architecture and OS
ARCH=$(uname -m)
OS=$(uname -s)

if [ "$OS" = "Darwin" ]; then
    if [ "$ARCH" = "arm64" ]; then
        TOOLS_ARCH="macos-arm64"
        TOOLS_URL="https://fastdl.mongodb.org/tools/db/mongodb-database-tools-macos-arm64-${TOOLS_VERSION}.zip"
    else
        TOOLS_ARCH="macos-x86_64"
        TOOLS_URL="https://fastdl.mongodb.org/tools/db/mongodb-database-tools-macos-x86_64-${TOOLS_VERSION}.zip"
    fi
elif [ "$OS" = "Linux" ]; then
    TOOLS_ARCH="linux-x86_64"
    TOOLS_URL="https://fastdl.mongodb.org/tools/db/mongodb-database-tools-ubuntu2204-x86_64-${TOOLS_VERSION}.tgz"
else
    echo "Unsupported OS: $OS"
    exit 1
fi

DEST_DIR="$RESOURCES_DIR/$TOOLS_ARCH"

echo "Downloading MongoDB Database Tools ${TOOLS_VERSION} for ${TOOLS_ARCH}..."
mkdir -p "$DEST_DIR"

# Create temp directory for download
TMP_DIR=$(mktemp -d)
trap 'rm -rf "$TMP_DIR"' EXIT

if [[ "$TOOLS_URL" == *.zip ]]; then
    curl -sL "$TOOLS_URL" -o "$TMP_DIR/tools.zip"
    unzip -q -j "$TMP_DIR/tools.zip" "*/bin/mongodump" "*/bin/mongorestore" -d "$DEST_DIR/"
else
    curl -sL "$TOOLS_URL" -o "$TMP_DIR/tools.tgz"
    tar -xzf "$TMP_DIR/tools.tgz" -C "$TMP_DIR"
    cp "$TMP_DIR"/mongodb-database-tools-*/bin/mongodump "$DEST_DIR/"
    cp "$TMP_DIR"/mongodb-database-tools-*/bin/mongorestore "$DEST_DIR/"
fi

chmod +x "$DEST_DIR/mongodump" "$DEST_DIR/mongorestore"

echo "MongoDB tools installed to: $DEST_DIR"
echo "  - mongodump: $("$DEST_DIR/mongodump" --version 2>/dev/null | head -1 || echo 'installed')"
echo "  - mongorestore: $("$DEST_DIR/mongorestore" --version 2>/dev/null | head -1 || echo 'installed')"
