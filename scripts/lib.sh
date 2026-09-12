#!/usr/bin/env bash
# Shared output variables are consumed by the scripts sourcing this file.
# shellcheck disable=SC2034
# Shared target names and verified downloads for release tooling.

openmango_platform() {
    local target="${1:-}"
    if [[ -z "$target" ]]; then
        case "$(uname -s)-$(uname -m)" in
            Darwin-arm64) target=aarch64-apple-darwin ;;
            Darwin-x86_64) target=x86_64-apple-darwin ;;
            Linux-aarch64|Linux-arm64) target=aarch64-unknown-linux-gnu ;;
            Linux-x86_64) target=x86_64-unknown-linux-gnu ;;
            *) echo "Unsupported host: $(uname -s) $(uname -m)" >&2; return 1 ;;
        esac
    fi
    OPENMANGO_TARGET="$target"
    case "$target" in
        aarch64-apple-darwin) OPENMANGO_ARCH_DIR=macos-arm64; OPENMANGO_BUN_TARGET=bun-darwin-arm64 ;;
        x86_64-apple-darwin) OPENMANGO_ARCH_DIR=macos-x86_64; OPENMANGO_BUN_TARGET=bun-darwin-x64 ;;
        aarch64-unknown-linux-gnu) OPENMANGO_ARCH_DIR=linux-arm64; OPENMANGO_BUN_TARGET=bun-linux-arm64 ;;
        x86_64-unknown-linux-gnu) OPENMANGO_ARCH_DIR=linux-x86_64; OPENMANGO_BUN_TARGET=bun-linux-x64 ;;
        *) echo "Unsupported target: $target" >&2; return 1 ;;
    esac
}

sha256_file() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    else
        shasum -a 256 "$1" | awk '{print $1}'
    fi
}

download_verified() (
    set -euo pipefail
    local url="$1" expected="$2" destination="$3" temporary
    if [[ -f "$destination" && "$(sha256_file "$destination")" == "$expected" ]]; then
        return
    fi
    mkdir -p "$(dirname "$destination")"
    temporary="$(mktemp "${destination}.XXXXXX")"
    trap 'rm -f "$temporary"' EXIT
    curl --fail --location --retry 3 --connect-timeout 15 --max-time 600 \
        --silent --show-error "$url" --output "$temporary"
    if [[ "$(sha256_file "$temporary")" != "$expected" ]]; then
        echo "Checksum mismatch: $url" >&2
        exit 1
    fi
    mv "$temporary" "$destination"
)
