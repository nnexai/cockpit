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

## Provider increment

Status: **PASS** for GitHub issue import through the real browser app and disposable companion. The run-owned config enabled `github` with the installed authenticated `gh` executable; installed file-viewer and Reviewr plugins were enabled in the run-owned Herdr registry. The Sources panel imported `https://github.com/nnexai/cockpit/issues/4` into the companion at `/tmp/cockpit-focus-20260912/companions/bbb18ee9-30cc-45fe-bbfa-25aa512c24a4`. The generated asset records provider `github`, canonical ID `nnexai/cockpit#4`, provider instance `https://github.com`, revision `2026-09-10T13:01:16Z`, 3 comments, and 8,993 bytes. See `provider-receipt.json` and [provider-import-1440.png](provider-import-1440.png).

Validation:

- `cargo test -p cockpit-providers` — 18 tests passed.
- `cargo test -p cockpit-core repositories::tests::github` — 2 passed, 105 filtered.
- `cargo build -p cockpit-host` — passed.
- One initial browser probe exposed and fixed the `url`/`html_url` serde collision; the final import completed with status `materialized`.

Direct task setup with an issue URL, ready-state transitions, and opening the prepared Context directly remain deferred to Packet 12. Native coverage is not applicable to this provider-only increment.
