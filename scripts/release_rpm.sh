#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "$ROOT_DIR/scripts/lib.sh"
openmango_platform "${1:-}"
[[ "$(uname -s)" == Linux && "$OPENMANGO_ARCH_DIR" == linux-* ]] || {
    echo "Build Linux packages inside Linux." >&2; exit 1;
}
[[ "$(rustc -vV | sed -n 's/^host: //p')" == "$OPENMANGO_TARGET" ]] || {
    echo "Package on a Linux runner matching $OPENMANGO_TARGET." >&2; exit 1;
}
command -v rpmbuild >/dev/null || { echo "Install rpm-build first." >&2; exit 1; }
cd "$ROOT_DIR"
VERSION="$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$ROOT_DIR/Cargo.toml" | head -1)"
BUILD_DIR="$(realpath -m "${CARGO_TARGET_DIR:-"$ROOT_DIR/target"}")"
DIST_DIR="$(realpath -m "${OPENMANGO_DIST_DIR:-"$ROOT_DIR/dist"}")"
export CARGO_TARGET_DIR="$BUILD_DIR"
STAGE_DIR="$BUILD_DIR/rpm/root"
TOP_DIR="$BUILD_DIR/rpm/build"
mkdir -p "$DIST_DIR"

bash "$ROOT_DIR/scripts/build_mongosh_sidecar.sh" "$OPENMANGO_TARGET"
bash "$ROOT_DIR/scripts/download_tools.sh" "$OPENMANGO_TARGET"
cargo build --locked --release --bin openmango --features mimalloc

# The layout src/connection/tools.rs expects: helpers in ../lib/openmango/bin next to the binary.
rm -rf "$STAGE_DIR" "$TOP_DIR"
install -d "$STAGE_DIR/usr/bin" "$STAGE_DIR/usr/lib/openmango/bin" \
    "$STAGE_DIR/usr/share/applications" "$STAGE_DIR/usr/share/icons/hicolor/256x256/apps" \
    "$STAGE_DIR/usr/share/doc/openmango"
install -m 755 "$BUILD_DIR/release/openmango" "$STAGE_DIR/usr/bin/openmango"
for tool in mongosh-sidecar mongodump mongorestore; do
    input="$ROOT_DIR/resources/bin/$OPENMANGO_ARCH_DIR/$tool"
    test -x "$input"
    case "$OPENMANGO_ARCH_DIR" in
        linux-arm64) file "$input" | grep -q 'ARM aarch64' ;;
        linux-x86_64) file "$input" | grep -q 'x86-64' ;;
    esac
    install -m 755 "$input" "$STAGE_DIR/usr/lib/openmango/bin/$tool"
done
install -m 644 "$ROOT_DIR/resources/linux/com.openmango.app.desktop" \
    "$STAGE_DIR/usr/share/applications/com.openmango.app.desktop"
install -m 644 "$ROOT_DIR/assets/logo/openmango.png" \
    "$STAGE_DIR/usr/share/icons/hicolor/256x256/apps/com.openmango.app.png"
install -m 644 "$ROOT_DIR/THIRD_PARTY_NOTICES" "$ROOT_DIR/LICENSE" \
    "$ROOT_DIR/assets/fonts/JetBrainsMono-OFL.txt" "$STAGE_DIR/usr/share/doc/openmango/"
cp -R "$ROOT_DIR/resources/linux/licenses" "$STAGE_DIR/usr/share/doc/openmango/licenses"
cp -R "$ROOT_DIR/resources/bin/$OPENMANGO_ARCH_DIR/licenses" \
    "$STAGE_DIR/usr/share/doc/openmango/licenses/mongodb-tools"
desktop-file-validate "$STAGE_DIR/usr/share/applications/com.openmango.app.desktop"

# RPM versions can't hold '-': 0.5.0-beta.1 becomes 0.5.0~beta.1, which sorts before 0.5.0.
rpmbuild -bb "$ROOT_DIR/resources/linux/openmango.spec" \
    --define "_topdir $TOP_DIR" \
    --define "openmango_version ${VERSION//-/\~}" \
    --define "openmango_root $STAGE_DIR"
ARTIFACT="$DIST_DIR/OpenMango-$VERSION-$OPENMANGO_ARCH_DIR.rpm"
mv "$TOP_DIR"/RPMS/*/openmango-*.rpm "$ARTIFACT"
(cd "$DIST_DIR" && sha256sum "$(basename "$ARTIFACT")" > "$(basename "$ARTIFACT").sha256")
rpm -qlp "$ARTIFACT"
echo "Packaged: $ARTIFACT"
