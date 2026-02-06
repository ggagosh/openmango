#!/usr/bin/env bash
# Download Node.js runtime for local development and release bundling.
set -euo pipefail

NODE_VERSION="${NODE_VERSION:-24.13.0}"
TARGET_ARCH=""

usage() {
    cat <<EOF
Usage: $0 [--arch <macos-arm64|macos-x86_64|linux-x86_64>] [--version <node-version>]

If --arch is omitted, host architecture is detected automatically.
EOF
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --arch)
            TARGET_ARCH="${2:-}"
            shift 2
            ;;
        --version)
            NODE_VERSION="${2:-}"
            shift 2
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            echo "Unknown argument: $1" >&2
            usage
            exit 1
            ;;
    esac
done

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RESOURCES_DIR="$SCRIPT_DIR/../resources/bin"

if [[ -z "$TARGET_ARCH" ]]; then
    ARCH="$(uname -m)"
    OS="$(uname -s)"
    if [[ "$OS" == "Darwin" ]]; then
        if [[ "$ARCH" == "arm64" ]]; then
            TARGET_ARCH="macos-arm64"
        else
            TARGET_ARCH="macos-x86_64"
        fi
    elif [[ "$OS" == "Linux" ]]; then
        TARGET_ARCH="linux-x86_64"
    else
        echo "Unsupported host OS: $OS" >&2
        exit 1
    fi
fi

case "$TARGET_ARCH" in
    macos-arm64)
        NODE_FILE="node-v${NODE_VERSION}-darwin-arm64.tar.gz"
        ;;
    macos-x86_64)
        NODE_FILE="node-v${NODE_VERSION}-darwin-x64.tar.gz"
        ;;
    linux-x86_64)
        NODE_FILE="node-v${NODE_VERSION}-linux-x64.tar.xz"
        ;;
    *)
        echo "Unsupported target arch: $TARGET_ARCH" >&2
        exit 1
        ;;
esac

NODE_URL="https://nodejs.org/dist/v${NODE_VERSION}/${NODE_FILE}"
DEST_DIR="$RESOURCES_DIR/$TARGET_ARCH"

echo "Downloading Node.js ${NODE_VERSION} for ${TARGET_ARCH}..."
mkdir -p "$DEST_DIR"

TMP_DIR="$(mktemp -d)"
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
echo "  - node: $("$DEST_DIR/node" --version 2>/dev/null | head -1 || echo "installed")"
