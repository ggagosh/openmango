#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "$ROOT_DIR/scripts/lib.sh"
openmango_platform x86_64-unknown-linux-gnu
[[ "$OPENMANGO_ARCH_DIR" == linux-x86_64 && "$OPENMANGO_BUN_TARGET" == bun-linux-x64 ]]
openmango_platform aarch64-unknown-linux-gnu
[[ "$OPENMANGO_ARCH_DIR" == linux-arm64 && "$OPENMANGO_BUN_TARGET" == bun-linux-arm64 ]]
if openmango_platform invalid-target 2>/dev/null; then
    echo "An unsupported target was accepted" >&2; exit 1
fi
for script in lib.sh bootstrap_linux.sh download_tools.sh build_mongosh_sidecar.sh release_linux.sh linux_packaging_tools.sh; do
    bash -n "$ROOT_DIR/scripts/$script"
done
desktop-file-validate "$ROOT_DIR/resources/linux/com.openmango.app.desktop"
echo "Linux target mappings, packaging script syntax, and desktop entry passed"
