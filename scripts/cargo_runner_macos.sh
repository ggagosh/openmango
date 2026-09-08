#!/usr/bin/env bash
set -euo pipefail

binary="$1"
shift

if [[ "$(basename "$binary")" != "openmango" ]]; then
    exec "$binary" "$@"
fi

identities="$(/usr/bin/security find-identity -v -p codesigning)"
identity_record="$(printf '%s\n' "$identities" | awk \
    -v requested="${OPENMANGO_DEV_SIGNING_IDENTITY:-}" '
    /^[[:space:]]*[0-9]+\)/ {
        fingerprint = $2
        name = $0
        sub(/^[^"]*"/, "", name)
        sub(/".*$/, "", name)
        if ((requested == "" && name ~ /^Apple Development:/) ||
            (requested != "" && (name == requested || toupper(fingerprint) == toupper(requested)))) {
            print fingerprint
            print name
            exit
        }
    }')"

if [[ -z "$identity_record" ]]; then
    echo "error: no matching development signing identity; install an Apple Development certificate or set OPENMANGO_DEV_SIGNING_IDENTITY to a full identity name or SHA-1." >&2
    exit 1
fi

identity="${identity_record%%$'\n'*}"
identity_name="${identity_record#*$'\n'}"
team_id="$(/usr/bin/security find-certificate -c "$identity_name" -p \
    | /usr/bin/openssl x509 -noout -subject -nameopt multiline \
    | sed -n 's/^[[:space:]]*organizationalUnitName[[:space:]]*=[[:space:]]*//p' \
    | tr -d '[:space:]')"
if [[ ! "$team_id" =~ ^[A-Z0-9]{10}$ ]]; then
    echo "error: could not determine the Apple team for the development signing identity." >&2
    exit 1
fi

# Keychain approvals should follow this app and team, not a renewed certificate's
# common name or the hash of a particular debug build.
code_requirement="identifier \"com.openmango.app.dev\" and anchor apple generic and certificate leaf[subject.OU] = \"$team_id\""
# Use the same canonical syntax that codesign displays (for example, it may
# omit quotes around an alphanumeric team ID) so the cache comparison is exact.
code_requirement="$(/usr/bin/csreq -r "=$code_requirement" -t)"
designated_requirement="designated => $code_requirement"
current_requirement="$(/usr/bin/codesign -d -r- "$binary" 2>/dev/null || true)"

# Cargo leaves an unchanged binary in place. Avoid accessing the signing key
# again unless a rebuild, signature failure, or identity change requires it.
if [[ "$current_requirement" != "$designated_requirement" ]] || \
    ! /usr/bin/codesign --verify --strict -R "=$code_requirement" "$binary" 2>/dev/null; then
    /usr/bin/codesign \
        --force \
        --sign "$identity" \
        --identifier "com.openmango.app.dev" \
        --requirements "=$designated_requirement" \
        --timestamp=none \
        "$binary"
    /usr/bin/codesign --verify --strict -R "=$code_requirement" "$binary"
fi

exec "$binary" "$@"
