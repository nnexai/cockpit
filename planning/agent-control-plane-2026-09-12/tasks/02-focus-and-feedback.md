# Acquire terminal control through normal selection

The user wants Herdr-style focus: selecting a pane should take control without an extra Take focus action. Routine focus should not produce a long toast.

## Inspect and reproduce

The [issue-4 runtime trial](../workflow/issue-4/README.md) observed loss of the beginning of a command typed immediately after selecting the visible terminal. Herdr readback confirmed the partial command. Retrying after focus settled worked. Reproduce this with explicit timing and harmless input; the trial does not establish its root cause.

Read `src/app/TerminalPane.tsx:166-197`, `293-310`, `src/app/session/focusCoordinator.ts`, `session/sessionStore.ts`, and `App.tsx:962`, `1211`. The code already has control requests, ownership checks, and pending input. Diagnose which normal gesture still requires extra action. The global delayed-focus toast is present in source.

Compare clicks, agent-row selection, tab switches, keyboard pane navigation, and re-entry after a menu with Herdr. Exercise a delayed confirmation and a second disposable client taking ownership. Capture event order and the first typed character, not just the final border color.

## Proposed behavior

Normal selection requests semantic focus and writable control through the existing coordinator. Focus xterm at the correct point. A normal success requires no further button and no toast. Use a small pane-local pending indicator with an accessible label. Show a compact retry control and inspectable error if acquisition fails. Keep longer text for an error whose consequence needs explanation.

Bind pending input to the existing session epoch, pane, and request token. If selection changes before confirmation, never flush the old queue into a new target. Test ownership loss during queued paste, reconnect, and failure followed by retry.

Do not add a parallel focus store. Passive output, hover, and background snapshots must not cause takeover loops. External ownership loss remains observable; the next deliberate user selection can request control naturally. Preserve input exactly once after ownership confirmation, or retain the failed action for explicit retry. Coordinate with packet 01.

## Acceptance

- Click/select and type requires one normal gesture, with input only in the intended pane.
- Ten rapid A/B selections and a delayed acknowledgement produce the confirmed final target without a stolen or duplicated keystroke.
- Opening and dismissing Commands, a context menu, or a viewer overlay restores the correct input target and scroll position.
- Background agent updates do not steal focus from search, a comment, or a setup input.
- Two clients do not repeatedly seize control from one another without local intent.
- Remove `.focus-feedback` and its global rendering if the local replacement covers the same state.

## Delivery rules

This packet describes future implementation. The current planning pass does not execute it.

When assigned, inspect the current checkout and reproduce before editing. Preserve unrelated work. Use a uniquely named disposable Herdr session and browser profile, never `default` or inherited user resources. Keep Herdr terminal content, status meanings, hierarchy, ordering, tabs, and pane layout authoritative. Compare changed Herdr-backed behavior with its TUI. Cockpit-owned graphical content may be redesigned.

Verify the complete real path at 1440×900, 800×1000, 600×900, and 480×900 where relevant. Use native Cockpit as well as the browser for clipboard, platform input, and window-dependent behavior. Use real data; mockup clicks and fixture pages are not acceptance evidence. Record PASS, FAIL, or INCONCLUSIVE, created resources, and cleanup. Remove replaced UI in the same increment, run appropriate checks, and commit only the completed scope. Report observed behavior, remaining limits, evidence paths, and commit.
