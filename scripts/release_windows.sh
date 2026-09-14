#!/usr/bin/env bash
# Build the per-user Windows installer: OpenMango.exe, Forge sidecar, MongoDB tools, licenses.
# Run in Git Bash on a Windows machine matching the target architecture.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "$ROOT_DIR/scripts/lib.sh"
openmango_platform "${1:-}"
[[ "$OPENMANGO_ARCH_DIR" == windows-* ]] || {
    echo "Build Windows packages for a Windows target." >&2; exit 1;
}
[[ "$(rustc -vV | sed -n 's/^host: //p')" == "$OPENMANGO_TARGET" ]] || {
    echo "Package on a Windows machine matching $OPENMANGO_TARGET." >&2; exit 1;
}
cd "$ROOT_DIR"
VERSION="$(sed -n 's/^version = "\([^"]*\)"/\1/p' Cargo.toml | head -1)"
BUILD_DIR="${CARGO_TARGET_DIR:-"$ROOT_DIR/target"}"
DIST_DIR="${OPENMANGO_DIST_DIR:-"$ROOT_DIR/dist"}"
STAGE="$DIST_DIR/OpenMango-$OPENMANGO_ARCH_DIR"
ARTIFACT="$DIST_DIR/OpenMango-$VERSION-$OPENMANGO_ARCH_DIR-setup.exe"
INNO_DIR="$BUILD_DIR/packaging-tools/inno-setup-7.1.0"

bash scripts/build_mongosh_sidecar.sh "$OPENMANGO_TARGET"
bash scripts/download_tools.sh "$OPENMANGO_TARGET"
cargo build --locked --release --bin openmango

rm -rf "$STAGE"
mkdir -p "$STAGE/bin" "$STAGE/licenses/mongodb-tools"
cp "$BUILD_DIR/release/openmango.exe" "$STAGE/OpenMango.exe"
for tool in mongodump mongorestore mongosh-sidecar; do
    cp "resources/bin/$OPENMANGO_ARCH_DIR/$tool.exe" "$STAGE/bin/$tool.exe"
done
cp THIRD_PARTY_NOTICES LICENSE assets/fonts/JetBrainsMonoNerdFont-LICENSE.txt "$STAGE/licenses/"
# Bun and minisign-verify ship on Linux and Windows; their licenses live with the Linux ones.
cp resources/linux/licenses/bun-LICENSE.md resources/linux/licenses/minisign-verify-LICENSE "$STAGE/licenses/"
cp "resources/bin/$OPENMANGO_ARCH_DIR/licenses/"* "$STAGE/licenses/mongodb-tools/"

if [[ ! -f "$INNO_DIR/ISCC.exe" ]]; then
    installer="$BUILD_DIR/packaging-tools/innosetup-7.1.0-x64.exe"
    download_verified "https://github.com/jrsoftware/issrc/releases/download/is-7_1_0/innosetup-7.1.0-x64.exe" \
        0362a383ed217d4c4239b5933866dd96d3eb2102737da92f80f6057a4b40df2f "$installer"
    # Git Bash rewrites /SWITCHES as POSIX paths; these arguments are already Windows-style.
    MSYS_NO_PATHCONV=1 "$installer" /VERYSILENT /SUPPRESSMSGBOXES /NORESTART /SP- /CURRENTUSER /NOICONS \
        "/DIR=$(cygpath -w "$INNO_DIR")"
fi
MSYS_NO_PATHCONV=1 "$INNO_DIR/ISCC.exe" /Qp "/DAppVersion=$VERSION" "/DArch=${OPENMANGO_ARCH_DIR#windows-}" \
    "/DSourceDir=$(cygpath -w "$STAGE")" "/O$(cygpath -w "$DIST_DIR")" \
    "$(cygpath -w "$ROOT_DIR/resources/windows/openmango.iss")"
sha256_file "$ARTIFACT" > "$ARTIFACT.sha256"
echo "Packaged: $ARTIFACT"
