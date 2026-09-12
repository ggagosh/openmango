#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "$ROOT_DIR/scripts/lib.sh"
source "$ROOT_DIR/scripts/linux_packaging_tools.sh"
openmango_platform "${1:-}"
[[ "$(uname -s)" == Linux && "$OPENMANGO_ARCH_DIR" == linux-* ]] || {
    echo "Build Linux packages inside Linux." >&2; exit 1;
}
[[ "$(rustc -vV | sed -n 's/^host: //p')" == "$OPENMANGO_TARGET" ]] || {
    echo "Package on a Linux runner matching $OPENMANGO_TARGET." >&2; exit 1;
}
cd "$ROOT_DIR"
VERSION="$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$ROOT_DIR/Cargo.toml" | head -1)"
BUILD_DIR="$(realpath -m "${CARGO_TARGET_DIR:-"$ROOT_DIR/target"}")"
DIST_DIR="$(realpath -m "${OPENMANGO_DIST_DIR:-"$ROOT_DIR/dist"}")"
export CARGO_TARGET_DIR="$BUILD_DIR"
APP_DIR="$DIST_DIR/OpenMango-$OPENMANGO_ARCH_DIR.AppDir"
ARTIFACT="$DIST_DIR/OpenMango-$VERSION-$OPENMANGO_ARCH_DIR.AppImage"
mkdir -p "$DIST_DIR"

if [[ "${REQUIRE_LINUX_SIGNING:-0}" == 1 ]]; then
    : "${OPENMANGO_LINUX_UPDATE_PUBLIC_KEY:?Set the trusted Linux update public key}"
    : "${OPENMANGO_LINUX_SIGNING_KEY_FILE:?Set the Minisign secret-key file}"
    command -v minisign >/dev/null
fi

bash "$ROOT_DIR/scripts/build_mongosh_sidecar.sh" "$OPENMANGO_TARGET"
bash "$ROOT_DIR/scripts/download_tools.sh" "$OPENMANGO_TARGET"
cargo build --locked --release --bin openmango --features mimalloc

rm -rf "$APP_DIR"
install -d "$APP_DIR/usr/bin" "$APP_DIR/usr/lib/openmango/bin" \
    "$APP_DIR/usr/share/applications" "$APP_DIR/usr/share/icons/hicolor/256x256/apps" \
    "$APP_DIR/usr/share/doc/openmango"
install -m 755 "$BUILD_DIR/release/openmango" "$APP_DIR/usr/bin/openmango"
for tool in mongosh-sidecar mongodump mongorestore; do
    input="$ROOT_DIR/resources/bin/$OPENMANGO_ARCH_DIR/$tool"
    test -x "$input"
    case "$OPENMANGO_ARCH_DIR" in
        linux-arm64) file "$input" | grep -q 'ARM aarch64' ;;
        linux-x86_64) file "$input" | grep -q 'x86-64' ;;
    esac
    if [[ "$tool" != mongosh-sidecar ]]; then
        install -m 755 "$input" "$APP_DIR/usr/lib/openmango/bin/$tool"
    fi
done
install -m 644 "$ROOT_DIR/resources/linux/com.openmango.app.desktop" "$APP_DIR/com.openmango.app.desktop"
install -m 644 "$ROOT_DIR/assets/logo/openmango.png" "$APP_DIR/com.openmango.app.png"
install -m 644 "$ROOT_DIR/THIRD_PARTY_NOTICES" "$ROOT_DIR/LICENSE" \
    "$ROOT_DIR/assets/fonts/JetBrainsMonoNerdFont-LICENSE.txt" "$APP_DIR/usr/share/doc/openmango/"
cp -R "$ROOT_DIR/resources/linux/licenses" "$APP_DIR/usr/share/doc/openmango/licenses"
cp -R "$ROOT_DIR/resources/bin/$OPENMANGO_ARCH_DIR/licenses" \
    "$APP_DIR/usr/share/doc/openmango/licenses/mongodb-tools"
desktop-file-validate "$APP_DIR/com.openmango.app.desktop"

linux_packaging_tools "$BUILD_DIR/packaging-tools"
APPIMAGE_EXTRACT_AND_RUN=1 "$LINUXDEPLOY" --appdir "$APP_DIR" \
    --executable "$APP_DIR/usr/bin/openmango" \
    --executable "$APP_DIR/usr/lib/openmango/bin/mongodump" \
    --executable "$APP_DIR/usr/lib/openmango/bin/mongorestore" \
    --desktop-file "$APP_DIR/com.openmango.app.desktop" \
    --icon-file "$APP_DIR/com.openmango.app.png"
# Bun embeds its payload in the ELF file; patchelf rewriting breaks that payload.
# Copy it after linuxdeploy. The pinned runtime needs only the host's libc libraries.
install -m 755 "$ROOT_DIR/resources/bin/$OPENMANGO_ARCH_DIR/mongosh-sidecar" \
    "$APP_DIR/usr/lib/openmango/bin/mongosh-sidecar"
# linuxdeploy links AppRun to the main executable and patches ELF RPATHs.
APPIMAGE_EXTRACT_AND_RUN=1 "$APPIMAGETOOL" --runtime-file "$APPIMAGE_RUNTIME" \
    "$APP_DIR" "$ARTIFACT"
chmod +x "$ARTIFACT"

COMMIT="$(git -C "$ROOT_DIR" rev-parse HEAD)"
ARCH=x86_64
[[ "$OPENMANGO_ARCH_DIR" == linux-arm64 ]] && ARCH=aarch64
python3 "$ROOT_DIR/scripts/linux_update_metadata.py" "$ARTIFACT" \
    --version "$VERSION" --commit "$COMMIT" \
    --channel "${OPENMANGO_RELEASE_CHANNEL:-stable}" --arch "$ARCH"
if [[ -n "${OPENMANGO_LINUX_SIGNING_KEY_FILE:-}" ]]; then
    : "${OPENMANGO_LINUX_UPDATE_PUBLIC_KEY:?Set the public key to verify the release signature}"
    minisign -Sm "$ARTIFACT.json" -s "$OPENMANGO_LINUX_SIGNING_KEY_FILE"
    minisign -Vm "$ARTIFACT.json" -P "$OPENMANGO_LINUX_UPDATE_PUBLIC_KEY"
elif [[ "${REQUIRE_LINUX_SIGNING:-0}" == 1 ]]; then
    echo "Refusing to publish an unsigned Linux update." >&2; exit 1
else
    echo "Local preview only: metadata is unsigned and must not be published as an update."
fi
echo "Packaged: $ARTIFACT"
