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

## Packet 12 fixture increment

Status: **PASS** for the earlier disposable fixture only. It proved the imported issue asset, direct Context opening, graphical Review, and draft preview behavior. It used an empty fixture checkout and therefore does not establish the real Cockpit checkout workflow.

## Packet 12 real checkout correction

Status: **PASS** for the focused real browser workflow. The run cloned the committed Cockpit checkout to `/tmp/cockpit-real-20260912/cockpit-repo` with canonical `origin=https://github.com/nnexai/cockpit.git` and preserved the prior internal URL as `internal-origin`. Setup through the app created linked checkout `/tmp/cockpit-real-20260912/worktrees/cockpit-repo-6da9059f-4cf1-44b4-a1c8-71f2a47cbbd1`, companion `6da9059f-4cf1-44b4-a1c8-71f2a47cbbd1`, and direct Context from the focused terminal. The Context tree rendered the real `nnexai/cockpit#4` issue asset at 8,993 bytes with all three comments.

The UI created two harmless tracked edits (`docs/discovery-limits.md`, `src/app/styles.css`) and one untracked file. Review saved a line draft and whole-file draft, refresh retained both, rapid file navigation returned to the selected file, and Tab 1 → Tab 2 retained the selected file and drafts. Reopening the same checkout through `Open existing checkout` borrowed the retained companion; the saved batch was selected, the overview reopened, and explicit `Reattach` restored both actual anchors/text. The restored preview records the line 22 anchor and both comment bodies, with source-changed status requiring stale excerpt review.

A second real operation (`7d615383-5607-4526-8e8f-2dace14a766e`) held the run-owned source import lock after creating its workspace/companion. It stopped at `partial/context_preparing` with `source_import_busy`; after releasing the lock, `Retry source import` completed with the same workspace and resource paths and materialized the 8,993-byte issue asset. This is a source-lock recovery proof, not a provider outage claim.

The corrected viewport receipt covers 1440×900, 800×1000, 600×900, 480×900, and 360×900. Each has a nonzero Review diff and Context document area, with source and diff roles present. Representative evidence is [packet12-real-review-1440.png](packet12-real-review-1440.png), [packet12-real-review-480.png](packet12-real-review-480.png), and `packet12-style-receipt.json`.

Validation:

- `bunx vitest run src/app/context/ContextViewer.test.tsx src/app/review/ReviewPane.test.tsx src/app/review/ReviewViewer.test.tsx` — 3 files, 20 tests passed.
- `bun run build` — `tsc --noEmit` and Vite 316 modules passed; existing large-chunk warning emitted.
- `cargo test -p cockpit-core companion_checkout_match_is_exact_and_unambiguous` — 1 passed, 106 filtered; integration target 0 run, 20 filtered.

Native constructor coverage, the full packet matrix, and automatic zero-click draft rebinding remain **INCONCLUSIVE/deferred**. Run-owned Herdr, gateway, browser, clone, worktrees, companions, and proof receipts remain under `/tmp/cockpit-real-20260912`.
