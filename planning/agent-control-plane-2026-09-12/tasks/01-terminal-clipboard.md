# Repair ordinary terminal copy and paste

The user reports that copy/paste works in Herdr but fails in Cockpit. Fix the Cockpit input integration without rewriting terminal content or replacing the renderer.

## Inspect and reproduce

Start at `src/app/TerminalPane.tsx:166-286`, `src/app/input/keymap.ts`, browser/native adapters, and Tauri capability configuration. No explicit app clipboard handling was found in the source pass, but xterm and the host may provide defaults. Do not assume missing handlers are the cause.

In matching disposable Herdr and Cockpit sessions, copy selected terminal text to a plain text editor and back. Try the platform's expected keyboard shortcuts, the context menu, and the normal paste gesture. Cover browser and native clients, Unicode, tabs, multiline text, bracketed-paste mode, and a nonempty terminal selection. Keep Ctrl+C interrupt behavior when no selection exists. Test an observer pane and control acquisition before paste.

## Implement the smallest complete repair

Trace the first failing boundary: key routing, text selection, clipboard permission, host integration, paste framing, or owned input delivery. Use one correct clipboard path per host. Preserve magic-escape handling, IME input, terminal application mouse mode, and the existing Shift+Enter behavior. Coordinate with packet 02 before changing input queueing.

Use explicit user gestures for clipboard reads and writes. Do not add a background clipboard poll or a broad permission fallback. Test focus changes while a clipboard read is pending.

Expose Copy/Paste at the relevant terminal context menu where supported. Do not introduce a second permanent instruction editor to work around the failure. Do not automatically send Enter or treat paste as a submitted agent task.

## Acceptance

- Copy preserves literal selected text and does not copy surrounding Cockpit chrome.
- Paste reaches exactly the intended writable pane, once, with expected newlines and framing.
- A delayed ownership response neither drops the paste nor sends it to the previously selected pane.
- Permission denial or unavailable clipboard access has a local, actionable failure, with text retained for retry.
- Native and browser evidence includes the actual clipboard round trip. Browser API mocks alone do not pass.

## Delivery rules

This packet describes future implementation. The current planning pass does not execute it.

When assigned, inspect the current checkout and reproduce before editing. Preserve unrelated work. Use a uniquely named disposable Herdr session and browser profile, never `default` or inherited user resources. Keep Herdr terminal content, status meanings, hierarchy, ordering, tabs, and pane layout authoritative. Compare changed Herdr-backed behavior with its TUI. Cockpit-owned graphical content may be redesigned.

Verify the complete real path at 1440×900, 800×1000, 600×900, and 480×900 where relevant. Use native Cockpit as well as the browser for clipboard, platform input, and window-dependent behavior. Use real data; mockup clicks and fixture pages are not acceptance evidence. Record PASS, FAIL, or INCONCLUSIVE, created resources, and cleanup. Remove replaced UI in the same increment, run appropriate checks, and commit only the completed scope. Report observed behavior, remaining limits, evidence paths, and commit.
