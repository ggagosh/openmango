#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "$ROOT_DIR/scripts/lib.sh"
image="$(realpath "${1:?Pass the AppImage to check}")"
temporary="$(mktemp -d)"
trap 'rm -rf "$temporary"' EXIT
expected="$(cat "$image.sha256")"
[[ "$(sha256_file "$image")" == "$expected" ]]
if [[ -n "${OPENMANGO_LINUX_UPDATE_PUBLIC_KEY:-}" ]]; then
    minisign -Vm "$image.json" -P "$OPENMANGO_LINUX_UPDATE_PUBLIC_KEY"
fi
cd "$temporary"
APPIMAGE_EXTRACT_AND_RUN=1 "$image" --version
"$image" --appimage-extract >/dev/null
payload="$temporary/squashfs-root"
desktop-file-validate "$payload/com.openmango.app.desktop"
for tool in mongodump mongorestore; do
    "$payload/usr/lib/openmango/bin/$tool" --version >/dev/null
done
python3 "$ROOT_DIR/scripts/check_linux_sidecar.py" "$payload/usr/lib/openmango/bin/mongosh-sidecar"
if [[ "${2:-}" == --database-tests ]]; then
    python3 "$ROOT_DIR/scripts/check_linux_data.py" "$payload/usr/lib/openmango/bin"
fi
echo "AppImage integrity, extraction, application entry point, and bundled tools passed"
