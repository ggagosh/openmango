#!/usr/bin/env bash
# Install a Windows setup into a temporary folder, check the app and bundled tools, uninstall.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "$ROOT_DIR/scripts/lib.sh"
installer="$(realpath "${1:?Pass the setup executable}")"
[[ "$(sha256_file "$installer")" == "$(cat "$installer.sha256")" ]]
# Installs share one AppId (resources/windows/openmango.iss): a check install would take over
# an existing installation's registration, and its uninstall would remove it.
if reg query 'HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\{E61F5184-E7BC-4EEB-9D8E-31082FC50C80}_is1' >/dev/null 2>&1; then
    echo "OpenMango is installed for this user. Uninstall it before running this check." >&2
    exit 1
fi
temporary="$(mktemp -d)"
app="$temporary/Open Mango ფაილები"
uninstall() {
    # Git Bash rewrites /SWITCHES as POSIX paths; these arguments are already Windows-style.
    MSYS_NO_PATHCONV=1 "$app/unins000.exe" /VERYSILENT /SUPPRESSMSGBOXES /NORESTART
    # The uninstaller relaunches itself from a temporary copy and returns immediately.
    for _ in $(seq 30); do [[ -f "$app/OpenMango.exe" || -f "$app/unins000.exe" ]] || break; sleep 1; done
}
# Never leave a registered installation behind, even when a check fails.
trap '[[ -f "$app/unins000.exe" ]] && uninstall; rm -rf "$temporary"' EXIT

MSYS_NO_PATHCONV=1 "$installer" /VERYSILENT /SUPPRESSMSGBOXES /NORESTART "/DIR=$(cygpath -w "$app")" \
    "/LOG=$(cygpath -w "$temporary/install.log")"
test -f "$app/unins000.exe"
version="$("$app/OpenMango.exe" --version)"
[[ "$version" == "OpenMango $(sed -n 's/^version = "\([^"]*\)"/\1/p' "$ROOT_DIR/Cargo.toml" | head -1)" ]]
for tool in mongodump mongorestore; do
    "$app/bin/$tool.exe" --version >/dev/null
done
python "$ROOT_DIR/scripts/check_sidecar.py" "$app/bin/mongosh-sidecar.exe"

if [[ "${2:-}" == --launch ]]; then
    # A GUI smoke test: the app must still be running after startup.
    "$app/OpenMango.exe" &
    pid=$!
    sleep 15
    kill -0 "$pid"
    MSYS_NO_PATHCONV=1 taskkill /PID "$(cat /proc/$pid/winpid)" /F >/dev/null
    echo "Installed app stayed open for 15 seconds"
fi

uninstall
[[ ! -f "$app/OpenMango.exe" && ! -f "$app/unins000.exe" ]]
echo "Windows installer, uninstaller, application entry point, and bundled tools passed"
