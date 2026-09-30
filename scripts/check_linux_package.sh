#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "$ROOT_DIR/scripts/lib.sh"
image="$(realpath "${1:?Pass the AppImage to check}")"
temporary="$(mktemp -d)"
trap 'rm -rf "$temporary"' EXIT
expected="$(cat "$image.sha256")"
[[ "$(sha256_file "$image")" == "$expected" ]]
if [[ -n "${OPENMANGO_UPDATE_PUBLIC_KEY:-}" ]]; then
    minisign -Vm "$image.json" -P "$OPENMANGO_UPDATE_PUBLIC_KEY"
fi
cd "$temporary"
APPIMAGE_EXTRACT_AND_RUN=1 "$image" --version
"$image" --appimage-extract >/dev/null
payload="$temporary/squashfs-root"
desktop-file-validate "$payload/com.openmango.app.desktop"
# Nothing inside may need a newer glibc than the oldest supported Ubuntu LTS has, or the AppImage
# won't start there: "version GLIBC_2.39 not found".
max_glibc="${OPENMANGO_MAX_GLIBC:-2.35}"
newest="$(
    find "$payload" -type f -print0 | while IFS= read -r -d '' elf; do
        [[ "$(head -c 4 "$elf" | tr -d '\0')" == $'\x7fELF' ]] || continue
        version="$(objdump -T "$elf" 2>/dev/null | grep -o 'GLIBC_[0-9.]*' | sort -uV | tail -1 || true)"
        [[ -n "$version" ]] && printf '%s %s\n' "${version#GLIBC_}" "${elf#"$payload"/}"
    done | sort -V | tail -1
)"
echo "Newest glibc needed: ${newest:-none} (limit $max_glibc)"
needed="${newest%% *}"
if [[ -n "$needed" && "$(printf '%s\n%s\n' "$max_glibc" "$needed" | sort -V | tail -1)" != "$max_glibc" ]]; then
    echo "The AppImage needs glibc $needed; build it on a system with glibc $max_glibc or older." >&2
    exit 1
fi
for tool in mongodump mongorestore; do
    "$payload/usr/lib/openmango/bin/$tool" --version >/dev/null
done
python3 "$ROOT_DIR/scripts/check_sidecar.py" "$payload/usr/lib/openmango/bin/mongosh-sidecar"
if [[ "${2:-}" == --database-tests ]]; then
    python3 "$ROOT_DIR/scripts/check_linux_data.py" "$payload/usr/lib/openmango/bin"
fi
echo "AppImage integrity, extraction, application entry point, and bundled tools passed"
