# Execution decisions

## OBS-001: Homebrew executable

The user requires the installed Homebrew Herdr, not .local/bin/herdr. Earlier PATH-selected calls were bounded read-only inventory. Retain their provenance as superseded; do not use that executable for builds or runtime checks. Pin the resolved Homebrew path and SHA-256 before BOOT-01. The protected server remains untouched.

## OBS-002: Archive shadowing executable

At the user's explicit request, moved /home/nnex/.local/bin/herdr to /home/nnex/.local/state/cockpit-execution/run-20260904T214621Z/archived-executables/herdr-local-protocol22. Same-filesystem rename preserved inode, mode, and SHA-256 32c977c413bbf005216cfa9585a0c1cfb403753dc70933a81b1e3d8bc5abb70f. No server/session command ran. PATH now resolves to /home/linuxbrew/.linuxbrew/bin/herdr. Retain this archive; restoration needs separate authorization.

## OBS-003: Wayland tooling

User reports wayland-utils installed. Use read-only compositor discovery; all automated GUI input remains isolated from the user desktop. Run-owned Xvfb packages are available as a non-system-install fallback.

## OBS-004: WebUI-first verification

The user selects the WebUI for tight-loop behavioral checks. Native AppImage startup and simple compatibility smoke tests suffice; repeated full native behavioral/temporal scenarios in plan 16 are superseded. Required browser behavior, negative controls, ownership safety, and source identity evidence remain unchanged. This is an explicit user scope change, not a response to a failing gate.

## DEC-005: Stable direct attachment

Protocol 20 uses one `TerminalAnsi` / `TerminalAttach` stream per visible pane. First-full and consecutive-frame rules apply across all adapters. Full repaint bytes do not reset xterm. A sole bounded socket reader survives concurrent input/resize/scroll; known auxiliary and graphics traffic is consumed without disconnecting text. Protocol-22/client-shell code and the image addon leave the active path; history stays reachable.

## DEC-006: Historical direct-attach application-mouse limitation

At the 2026-09-04 run checkpoint, the pinned stable direct-attach API did not expose safe pane-targeted application-mouse state/coordinates. Its raw Input path and globally routed InputEvents did not justify unconditional SGR or guessed global coordinates. The checkpoint therefore recorded `terminal_mouse_input=false`, preserved click/focus and direct scrolling, and kept G01 incomplete. This remains the historical decision for that path, not a current claim that Herdr 0.8.2 cannot carry SGR through other APIs.

## POST-006: 2026-09-06 mouse-input correction

The later live check verified a press/release SGR pair delivered by `herdr pane send-keys` to the fixture; the user also reports wheel/scroll delivery through ordinary xterm.js panes attached to Herdr. The active decision now separates physical direct-attach filtering, explicit CLI emulation, normal xterm.js scrolling, and structured app-mode pointer routing. Native pointer forwarding remains to trace; if unavailable, an explicit ownership-gated emulator is permitted. No unconditional shell-prompt injection or guessed global coordinates are permitted.

## DEC-007: Native packaging tool compatibility

AppImage release compilation passed. linuxdeploy 659c9db's bundled strip failed on Fedora ELF `.relr.dyn` sections. Its source offers NO_STRIP but no STRIP-path override. Building with NO_STRIP=1 preserves those ELF files; the temporary 108550648-byte AppImage then opened a real WebKit window and delivered NATIVE-STABLE-123 plus CR exactly to w5:p1. SHA-256 ca1ce374f6293accc64efbe92f47430ae5c0de4692f8494c64f8829cd935e064. No tool cache, user installation, or Herdr executable was replaced. Xvfb lacks DRI3, so this proves native startup/input compatibility, not accelerated native presentation.

## DEC-008: Evidence collection and the first scroll control

Stable CLI snapshots omit a session tag; guarded server status supplies the authoritative session and socket. The gateway has a different status DTO, validated against its protocol, project version, capabilities, and nested Herdr identity. Native/browser evidence is paired explicitly. A real negative control exposed a swallowed protocol disagreement: it returned PASS before repair and now returns FAIL.

Startup screenshots use bounded non-interlaced RGB/RGBA PNG, the format produced by both verified hosts. Header-only JPEG/WebP acceptance was removed. Empty ownership roots cannot authorize submitted paths; report replacement cannot truncate a linked file. Missing source identities and malformed evidence remain non-passing.

The first browser compositor control captured 3601 frames over 60 seconds and 100 trusted wheel inputs. Its p95 was 43 ms; all offsets matched, and every full-frame hash matched one of two independently captured reference viewports. Away-tail frames stayed unchanged while output continued. This is one control run, not aggregate G01 acceptance.
