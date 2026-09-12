#!/usr/bin/env bash
set -euo pipefail

if [[ -z "${WAYLAND_DISPLAY:-}" && "${XDG_SESSION_TYPE:-}" != "wayland" ]]; then
  printf 'run-wayland.sh: Wayland is unavailable (set WAYLAND_DISPLAY or use a Wayland session)\n' >&2
  exit 1
fi
if [[ -z "${WAYLAND_DISPLAY:-}" ]]; then
  printf 'run-wayland.sh: XDG_SESSION_TYPE=wayland but WAYLAND_DISPLAY is unset\n' >&2
  exit 1
fi

export GDK_BACKEND=wayland
export WINIT_UNIX_BACKEND=wayland
exec cargo run --manifest-path src-tauri/Cargo.toml "$@"
