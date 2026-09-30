#!/usr/bin/env bash
set -euo pipefail

if [[ "$(uname -s)" != Linux ]]; then
    echo "Run this inside an Ubuntu/Debian Linux builder, not on macOS." >&2
    exit 1
fi
if [[ "$EUID" -ne 0 ]]; then
    exec sudo bash "$0" "$@"
fi

export DEBIAN_FRONTEND=noninteractive
apt-get update
apt-get install -y --no-install-recommends \
    build-essential clang cmake pkg-config perl git curl ca-certificates \
    unzip xz-utils file patchelf python3 desktop-file-utils fonts-dejavu-core \
    libfontconfig-dev libxcb1-dev libxkbcommon-dev libxkbcommon-x11-dev \
    libwayland-dev libssl-dev

# Signs and verifies the update manifests. Ubuntu 22.04, which releases are built on, has no
# package for it: take upstream's static build, pinned.
if ! apt-get install -y --no-install-recommends minisign; then
    archive="$(mktemp)"
    curl -fsSL --max-time 120 -o "$archive" \
        https://github.com/jedisct1/minisign/releases/download/0.12/minisign-0.12-linux.tar.gz
    echo "9a599b48ba6eb7b1e80f12f36b94ceca7c00b7a5173c95c3efc88d9822957e73  $archive" | sha256sum -c -
    tar -xzf "$archive" -C /usr/local/bin --strip-components=2 "minisign-linux/$(uname -m)/minisign"
    rm -f "$archive"
    minisign -v
fi

if [[ "${1:-}" == --desktop-tests ]]; then
    apt-get install -y --no-install-recommends \
        xvfb xauth xdotool x11-utils openbox xfwm4 dbus-x11 gnome-keyring \
        libsecret-tools python3-gi gir1.2-gtk-3.0 \
        xdg-desktop-portal xdg-desktop-portal-gtk \
        mesa-vulkan-drivers libgl1-mesa-dri libegl1 mesa-utils vulkan-tools
fi
