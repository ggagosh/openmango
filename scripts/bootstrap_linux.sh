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
    unzip xz-utils file patchelf python3 desktop-file-utils minisign fonts-dejavu-core \
    libfontconfig-dev libxcb1-dev libxkbcommon-dev libxkbcommon-x11-dev \
    libwayland-dev libssl-dev

if [[ "${1:-}" == --desktop-tests ]]; then
    apt-get install -y --no-install-recommends \
        xvfb xauth xdotool x11-utils openbox xfwm4 dbus-x11 gnome-keyring \
        libsecret-tools python3-gi gir1.2-gtk-3.0 \
        xdg-desktop-portal xdg-desktop-portal-gtk \
        mesa-vulkan-drivers libgl1-mesa-dri libegl1 mesa-utils vulkan-tools
fi
