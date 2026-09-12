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

## Packet 12 integrated increment

Status: **PASS** for the scoped browser workflow. The real setup dialog created linked checkout `/tmp/cockpit-focus-20260912/worktrees/fixture-repo-f3136c9f-c08d-4c0a-83fb-e7ee8c363528`, companion `f3136c9f-c08d-4c0a-83fb-e7ee8c363528`, and context terminal from `https://github.com/nnexai/cockpit/issues/4`. The operation reached `completed`; the companion asset is 8,993 bytes with three GitHub comments. Opening Context from that terminal rendered the asset, and graphical Review showed two tracked edits plus one untracked file.

Review line and whole-file drafts saved, preview refresh retained both anchors, rapid file navigation returned to the selected file, and Tab 1 → Tab 2 retained the selected file and drafts. Computed source/diff font checks ran at 1440×900, 1280×800, 1024×768, and 480×900. Evidence is limited to [packet12-context-1440.png](packet12-context-1440.png), [packet12-review-1440-final.png](packet12-review-1440-final.png), and `packet12-style-receipt.json`.

Validation:

- `cargo test -p cockpit-core projects::tests` — 4 passed, 103 filtered.
- `cargo build -p cockpit-host` — passed after the direct Context and Review root repairs.
- `bun run test -- src/app/review/ReviewPane.test.tsx src/app/review/ReviewViewer.test.tsx src/app/projects/SetupDialog.stale.test.ts src/client/projectProtocol.test.ts src/app/paneRenderers.test.tsx` — 5 files, 25 tests passed.
- `bun run build` — `tsc --noEmit` and Vite 316 modules passed; existing large-chunk warning emitted.

Native constructor coverage, the full packet matrix, and automatic zero-click draft rebinding remain **INCONCLUSIVE/deferred**. Run-owned Herdr, gateway, browser, fixture, and services remain under `/tmp/cockpit-focus-20260912` for the next increment.
