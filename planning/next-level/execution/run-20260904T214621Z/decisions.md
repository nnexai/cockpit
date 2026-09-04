# Execution decisions

## OBS-001: Homebrew executable

The user requires the installed Homebrew Herdr, not .local/bin/herdr. Earlier PATH-selected calls were bounded read-only inventory. Retain their provenance as superseded; do not use that executable for builds or runtime checks. Pin the resolved Homebrew path and SHA-256 before BOOT-01. The protected server remains untouched.

## OBS-002: Archive shadowing executable

At the user's explicit request, moved /home/nnex/.local/bin/herdr to /home/nnex/.local/state/cockpit-execution/run-20260904T214621Z/archived-executables/herdr-local-protocol22. Same-filesystem rename preserved inode, mode, and SHA-256 32c977c413bbf005216cfa9585a0c1cfb403753dc70933a81b1e3d8bc5abb70f. No server/session command ran. PATH now resolves to /home/linuxbrew/.linuxbrew/bin/herdr. Retain this archive; restoration needs separate authorization.

## OBS-003: Wayland tooling

User reports wayland-utils installed. Use read-only compositor discovery; all automated GUI input remains isolated from the user desktop. Run-owned Xvfb packages are available as a non-system-install fallback.

## OBS-004: WebUI-first verification

The user selects the WebUI for tight-loop behavioral checks. Native AppImage startup and simple compatibility smoke tests suffice; repeated full native behavioral/temporal scenarios in plan 16 are superseded. Required browser behavior, negative controls, ownership safety, and source identity evidence remain unchanged. This is an explicit user scope change, not a response to a failing gate.
