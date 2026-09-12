#!/usr/bin/env bash
# Pinned MongoDB Database Tools; hashes come from the publisher's full.json.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "$ROOT_DIR/scripts/lib.sh"
openmango_platform "${1:-}"
TOOLS_VERSION=100.14.1

case "$OPENMANGO_ARCH_DIR" in
    macos-arm64)
        archive="mongodb-database-tools-macos-arm64-${TOOLS_VERSION}.zip"
        checksum=c75e80b7c92d8884d7d47796111dda0461b64b915449731e043a186e2a62d6f8 ;;
    macos-x86_64)
        archive="mongodb-database-tools-macos-x86_64-${TOOLS_VERSION}.zip"
        checksum=dcd9ddab8f21da21191ee2169dfd352d8280ec9a90ef13c145d201e32256fe1c ;;
    linux-x86_64)
        archive="mongodb-database-tools-ubuntu2204-x86_64-${TOOLS_VERSION}.tgz"
        checksum=96567f4a8239ac460a21a4c8ab7e54cda84092036044927bbd5a4eeee5d08117 ;;
    linux-arm64)
        archive="mongodb-database-tools-ubuntu2204-arm64-${TOOLS_VERSION}.tgz"
        checksum=670727e163df0ce86978f50ebd5dcd75e345e0027889654c1c0cccf6cd4183d9 ;;
esac

cache="${CARGO_TARGET_DIR:-"$ROOT_DIR/target"}/downloads/$archive"
download_verified "https://fastdl.mongodb.org/tools/db/$archive" "$checksum" "$cache"
temporary="$(mktemp -d)"
trap 'rm -rf "$temporary"' EXIT
destination="$ROOT_DIR/resources/bin/$OPENMANGO_ARCH_DIR"
mkdir -p "$destination"

if [[ "$archive" == *.zip ]]; then
    unzip -q -j "$cache" "*/bin/mongodump" "*/bin/mongorestore" \
        "*/LICENSE.md" "*/THIRD-PARTY-NOTICES" -d "$temporary"
else
    tar -xzf "$cache" -C "$temporary" --strip-components=2 \
        "${archive%.tgz}/bin/mongodump" "${archive%.tgz}/bin/mongorestore"
    tar -xzf "$cache" -C "$temporary" --strip-components=1 \
        "${archive%.tgz}/LICENSE.md" "${archive%.tgz}/THIRD-PARTY-NOTICES"
fi
for tool in mongodump mongorestore; do
    test -s "$temporary/$tool"
    install -m 755 "$temporary/$tool" "$destination/$tool"
done
mkdir -p "$destination/licenses"
install -m 644 "$temporary/LICENSE.md" "$temporary/THIRD-PARTY-NOTICES" "$destination/licenses/"
echo "Verified MongoDB tools $TOOLS_VERSION installed to $destination"
