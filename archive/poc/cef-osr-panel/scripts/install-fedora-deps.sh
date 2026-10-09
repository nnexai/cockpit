#!/usr/bin/env bash
set -euo pipefail

if [[ "${EUID}" -eq 0 ]]; then
  SUDO=()
elif command -v sudo >/dev/null 2>&1; then
  sudo -v
  SUDO=(sudo)
else
  printf 'install-fedora-deps.sh: run as root or install sudo first\n' >&2
  exit 1
fi

if ! command -v dnf >/dev/null 2>&1; then
  printf 'install-fedora-deps.sh: Fedora dnf was not found; install dependencies manually\n' >&2
  exit 1
fi

# CEF supplies Chromium itself; these are the native build/X11 and runtime development headers.
"${SUDO[@]}" dnf install -y \
  gcc-c++ cmake ninja-build pkgconf-pkg-config \
  libX11-devel libXcomposite-devel libXdamage-devel libXext-devel \
  libXfixes-devel libXi-devel libXrandr-devel libXrender-devel \
  libXtst-devel libxcb-devel libdrm-devel mesa-libgbm-devel \
  gtk3-devel pango-devel atk-devel at-spi2-atk-devel cups-devel alsa-lib-devel
