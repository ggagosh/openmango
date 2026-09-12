#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUTPUT_DIR="${1:-"$ROOT_DIR/target/app-icon"}"

mkdir -p "$OUTPUT_DIR"
xcrun actool "$ROOT_DIR/assets/app-icon/openmango.icon" \
    --compile "$OUTPUT_DIR" \
    --app-icon openmango \
    --platform macosx \
    --target-device mac \
    --minimum-deployment-target 11.0 \
    --output-partial-info-plist "$OUTPUT_DIR/icon-info.plist" \
    --output-format human-readable-text

# Use Apple's rendered fallback for the in-app icon, too.
/usr/bin/sips -s format png "$OUTPUT_DIR/openmango.icns" \
    --out "$OUTPUT_DIR/openmango.png" >/dev/null
