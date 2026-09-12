# Packet 02 runtime increment

Current increment: preserve terminal input across a Herdr focus handoff and show pending state on the target pane.

Status: **PASS** for the scoped browser gate. Native Tauri coverage is **INCONCLUSIVE** because this increment was exercised through the browser gateway only.

The pre-change browser baseline used the real gateway and Herdr session. A same-call Space selection followed by immediate terminal input left the target pane unchanged; input after the selection settled was delivered. After the change, a real click followed immediately by typing produced `FOCUS_TYPE_OK` in Herdr pane `w2:p1`. A six-step rapid target check delivered the first cockpit marker to `w1:p1` and the final FocusFixture marker to `w2:p1`; no marker appeared in the wrong pane or more than once.

Evidence is limited to [focus-after-1440.png](focus-after-1440.png), [focus-after-480.png](focus-after-480.png), and `focus-receipt.json`. The receipt records the exact commands and authoritative readbacks.

Validation:

- `bun run test -- src/app/TerminalPane.test.tsx` — 1 file, 9 tests passed.
- `bun run build` — `tsc --noEmit && vite build` passed; Vite emitted its existing large-chunk warning.
- Browser: exact gateway `http://127.0.0.1:4189/`, focused immediate-input proof and rapid-target proof; console errors 0 during the rapid check.

The Herdr 0.9.0 session, gateway, browser profile, fixture, and proof scripts are owned by `runtime_delivery` under `/tmp/cockpit-focus-20260912` and remain alive for the next workflow increment. Clean them only when the workflow is finished. Delayed acknowledgement, multiple panes, full packet matrices, and native runtime behavior remain deferred.
