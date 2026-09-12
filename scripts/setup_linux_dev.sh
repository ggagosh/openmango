#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "$ROOT_DIR/scripts/lib.sh"
openmango_platform
[[ "$OPENMANGO_ARCH_DIR" == linux-* ]] || { echo "Run this inside Linux." >&2; exit 1; }

bash "$ROOT_DIR/scripts/bootstrap_linux.sh" --desktop-tests
if ! command -v rustup >/dev/null 2>&1; then
    sudo apt-get install -y rustup
fi
rustup toolchain install 1.98.0 --profile minimal --component clippy --component rustfmt
rustup default 1.98.0

case "$OPENMANGO_ARCH_DIR" in
    linux-arm64)
        bun_archive=bun-linux-aarch64
        bun_sha=54328bbc2d9c8e0c9f892c544d66c57a83b84139e34909e5ee81758f1ac8fda7 ;;
    linux-x86_64)
        bun_archive=bun-linux-x64
        bun_sha=36368faef7527875d5ffa52e53cd48021741f2a83eb6208a8dd64068d422a913 ;;
esac
temporary="$(mktemp -d)"
trap 'rm -rf "$temporary"' EXIT
download_verified "https://github.com/oven-sh/bun/releases/download/bun-v1.4.2/$bun_archive.zip" \
    "$bun_sha" "$temporary/bun.zip"
unzip -q "$temporary/bun.zip" -d "$temporary"
install -d "$HOME/.local/bin"
install -m 755 "$temporary/$bun_archive/bun" "$HOME/.local/bin/bun"
echo 'Linux toolchain ready. Add ~/.local/bin and ~/.cargo/bin to PATH for builds.'
