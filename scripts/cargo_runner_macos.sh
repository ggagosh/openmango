#!/usr/bin/env bash
set -euo pipefail

binary="$1"
shift

if [[ "$(basename "$binary")" == "openmango" ]]; then
    identity="${OPENMANGO_DEV_SIGNING_IDENTITY:-}"
    if [[ -z "$identity" ]]; then
        identity="$(security find-identity -v -p codesigning \
            | sed -n 's/.*"\(Apple Development:[^"]*\)".*/\1/p' \
            | head -1)"
    fi

    if [[ -z "$identity" ]]; then
        echo "warning: no Apple Development identity; running unsigned (Keychain may prompt after rebuilds)" >&2
    else
        codesign \
            --force \
            --sign "$identity" \
            --identifier "com.openmango.app.dev" \
            --timestamp=none \
            "$binary"
    fi
fi

exec "$binary" "$@"
