# WS-10 `TerminalPane` split

Wave 1 · Size M · Depends on: – · Blocks: WS-24

## Goal
`TerminalPane` (894-line component) separates frame delivery, resize negotiation, mouse translation and theming.

## Owns
- `src/app/TerminalPane.tsx`, `src/app/TerminalPane.test.tsx`
- a new `src/app/terminal/` for the extracted modules (keep `TerminalPane.tsx` where it is to avoid import churn)

## Evidence
- `TerminalPane.tsx:~244` has ~35 refs.
- The frame queue limits are `MAX_QUEUED_FRAME_COUNT` 64 and 8 MiB.
- The resize/settle logic starts at ~285.
- Theme colours are hard-coded at ~66-90, and the font size at ~91.

## Change
1. `useTerminalFrameQueue`: queue limits, backpressure, drain.
2. `useTerminalResize`: measure, settle, report.
3. A `terminalMouse.ts` pure module for mouse/wheel translation.
4. `terminalTheme.ts`: theme and font constants.

## Keep
- Frame ordering and limits.
- Resize timing (no extra SIGWINCH).
- Wheel routing.
- Focus and DOM.
- No CSS edits.

## Acceptance
- `TerminalPane` is under 300 lines.
- `TerminalPane.test.tsx` passes, with only import-path edits.

## Verify
- `bun run test -- src/app/TerminalPane`.
- Browser smoke on a disposable fixture: type, run a full-screen TUI, resize the window, switch tabs, use wheel scrollback and TUI mouse.
- Probe tab-switch resize with `skill://cockpit-pty-resize-tab-switch-probe`.
- One native run (WebKitGTK rendering).
