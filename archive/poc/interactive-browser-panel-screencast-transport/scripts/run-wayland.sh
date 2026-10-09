#!/usr/bin/env sh
set -eu

if [ "${XDG_SESSION_TYPE:-}" != "wayland" ] && [ -z "${WAYLAND_DISPLAY:-}" ]; then
  printf '%s\n' 'This POC requires a Wayland session (XDG_SESSION_TYPE=wayland or WAYLAND_DISPLAY).' >&2
  exit 1
fi

export GDK_BACKEND=wayland
export WINIT_UNIX_BACKEND=wayland
exec bun run dev
