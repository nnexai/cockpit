# Make browser feedback compact and unambiguous

Redesign the Cockpit-owned browser feedback workflow around the selected Space, actual capture, annotations, and explicit delivery target. Preserve existing acknowledgement and duplicate-risk semantics.

## Inspect and reproduce

Read `src/app/App.tsx:593-647`, `714-870`, `921-925`, the browser protocols, and extension feedback records. Reproduce with the production extension paired to a disposable Space and an actual pending capture. The atlas empty panel is useful layout evidence; mixed PASS/FAIL cells do not prove delivery.

## Work

Replace the duplicate headings and “Browser: open · browser is open” copy with one scoped heading and one meaningful connection state. Show page identity and pending annotation content. Keep refresh, dismiss, and delivery actions compact. Use a list/detail or drawer layout that remains usable in a narrow pane/window.

Bind every draft and action to the capture and Space that created it. Target selection uses current eligible pane data. Show pending, rejected, accepted, and outcome-unknown results distinctly. Do not equate successful transport with agent completion, or clear annotations because a button was clicked. Preserve edits during polling and reconnect.

## Acceptance

- A real capture appears in the correct Space and remains editable through polling updates.
- Switching Spaces during a pending lookup/send cannot show or deliver the previous Space's data as the new Space's data.
- Empty, disconnected, stale, rejected, and unknown outcomes retain enough context for recovery.
- Accepted delivery and acknowledgement follow their existing distinct contracts; selected acknowledgement removes only the acknowledged IDs.
- Unknown delivery preserves the item and target. Retrying uses the existing duplicate-risk confirmation and cannot silently send twice.
- Normal refresh needs no toast; consequential delivery failures remain persistent and inspectable.
- Coordinate `App.tsx` and shared styles with packets 03–05. Remove the replaced floating panel/chrome in the completed increment.

## Delivery rules

This packet describes future implementation. The current planning pass does not execute it.

When assigned, inspect the current checkout and reproduce before editing. Preserve unrelated work. Use a uniquely named disposable Herdr session and browser profile, never `default` or inherited user resources. Keep Herdr terminal content, status meanings, hierarchy, ordering, tabs, and pane layout authoritative. Compare changed Herdr-backed behavior with its TUI. Cockpit-owned graphical content may be redesigned.

Verify the complete real path at 1440×900, 800×1000, 600×900, and 480×900 where relevant. Use native Cockpit as well as the browser for clipboard, platform input, and window-dependent behavior. Use real data; mockup clicks and fixture pages are not acceptance evidence. Record PASS, FAIL, or INCONCLUSIVE, created resources, and cleanup. Remove replaced UI in the same increment, run appropriate checks, and commit only the completed scope. Report observed behavior, remaining limits, evidence paths, and commit.
