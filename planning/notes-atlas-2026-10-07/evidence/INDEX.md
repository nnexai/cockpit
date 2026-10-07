# Notes visual evidence and runtime verification

Date: 2026-10-07. Owner run: 8297690c-7e8a-4974-a2c8-bb94886195b9. Evidence is local disposable data, never user Notes.

## Provenance and limits

**Actual disposable app (A):** shared browser build served by owned gateway http://127.0.0.1:45131, isolated Herdr session polish-6y_a886d, owned root /tmp/cpol-6y_a886d. Fixture created using scripts/verify/ui_polish_runtime.py start. Notes root /tmp/cpol-6y_a886d/data/cockpit/notes, pinned Notes UUID 694e8c52-6f21-473b-a966-ff161da52da1, resolved from actual decision File field. Actual loaded script /assets/index-CssAnWz1.js; existing bundle timestamp 2026-10-07 04:13:01 +0200, later than inspected Notes CSS/MarkdownEditor source timestamps 2026-10-06 21:49:56 +0200. Existing target/debug/cockpit gateway binary timestamp 2026-10-07 00:12:35 +0200. No shared build or installation performed; source atlas is authoritative for current implementation, runtime is this identified existing bundle, not independently rebuilt proof. Browser Chromium headless; deviceScaleFactor 1.25. Wide 1440×1000 CSS pixels, medium 900×900, narrow 420×900. Actual screenshots contain synthetic *fixture content* in the real app, not a synthetic UI/harness.

**Code-derived (C):** five research slices and atlas source/state contracts, including fault/rare states not induced. Existing test references are not test execution evidence.

**Illustrative (I):** design presentation before/proposed concepts; authored diagrams/mock UI, not app screenshots or deployed changes.

## Actual evidence index

| Evidence | Screenshot | CSS viewport | Surface coverage | What it proves |
|---|---|---|---|---|
| A01 | [Unbound](01-unbound-wide.png) | 1440×1000 | N-SHELL-01/02 | Explicit create/attach, no implicit setup |
| A02 | [Catalog error](02-catalog-empty-wide.png) | 1440×1000 | N-SHELL-03 | Initial nonexistent fixture Notes path reports notes_not_found and offers Retry reading catalog; this is an error, not an empty catalog success |
| A03 | [Scratchpad preview](03-scratchpad-preview-wide.png) | 1440×1000 | N-SCR-02/03 | Fixture Markdown saved, preview and saved status visible |
| A04 | [Todos and comments](04-todos-comments-wide.png) | 1440×1000 | N-TODO-01/04/06; N-COM-01/02 | Task detail, added durable comment, promoted board membership |
| A05 | [Kanban wide](05-kanban-wide.png) | 1440×1000 | N-KAN-01/03/08 | Settled Backlog→Doing keyboard move and lane counts/live status |
| A06 | [Kanban medium](06-kanban-medium.png) | 900×900 | N-KAN-01 | Responsive board density |
| A07 | [Kanban narrow](07-kanban-narrow.png) | 420×900 | N-KAN-01/08 | Lane jumps and horizontal board scroll; document scrollWidth=420, board scrollWidth=1057 |
| A08 | [Decision detail](08-decision-detail-wide.png) | 1440×1000 | N-DEC-02/04/05/06 | Edited record, immutable recorded date, historical decided date, file/reference and replacement disclosure |
| A09 | [Decision history](09-decisions-history-wide.png) | 1440×1000 | N-DEC-01/05 | History shows predecessor as Replaced; old selected successor detail may remain Current while list shows History |
| A10 | [Decision narrow](10-decisions-narrow.png) | 420×900 | N-DEC-01/02 | Narrow decision list/detail navigation layout |
| A11 | [Scratchpad conflict](11-scratchpad-conflict-wide.png) | 1440×1000 | N-SCR-03; N-REC-01/03 | Local draft retained, saved external fixture version disclosed, explicit Keep mine/Reload |
| A12 | [Transfer confirmation](12-attach-transfer-confirm-wide.png) | 1440×1000 | N-SHELL-03 | Selected exact UUID Attached elsewhere; explicit Keep current association/Attach here |
| A13 | [Unadopted todo](13-todo-unadopted-wide.png) | 1440×1000 | N-TODO-05 | Hand-written task shows explicit Adopt task & comments |
| A14 | [Adopted detail](14-todo-adopted-detail-wide.png) | 1440×1000 | N-TODO-05/06 | Adopted canonical task detail settles and comments empty composer visible |
| A15 | [Comments medium](15-todos-comments-medium.png) | 900×900 | N-TODO-06; N-COM-01/02 | Responsive detail layout |
| A16 | [Comments narrow](16-todos-comments-narrow.png) | 420×900 | N-TODO-06; N-COM-01/02 | Detail takes narrow surface; completion/move/comment controls available |
| A17 | [Unboarded thread](17-unboarded-retained-comment-wide.png) | 1440×1000 | N-TODO-04; N-COM-03 | Unboarding retained edited durable comment and displayed original author/created timestamp |
| A18 | [Delete confirmation](18-comment-delete-confirm-wide.png) | 1440×1000 | N-COM-04 | Explicit inline Keep/Delete comment confirmation; Keep was exercised, deletion not committed |
| A19 | [Scratchpad source](19-scratchpad-source-wide.png) | 1440×1000 | N-SCR-02; N-SHELL-04 | Home from Todos tab selected/focused Scratchpad and current source editor visible |

## Exercised behavior and observed results

1. Header Open Notes focused local Notes region. Unbound app displayed create/attach. Initial attach catalog returned notes_not_found because isolated Notes root did not exist; Cancel returned to unbound setup. Explicit Create notes bound fixture w1 only and defaulted to Kanban.
2. Scratchpad typed through real CodeMirror control, saved and rendered preview with Saved · scratchpad.md. Browser fill uses keyboard-like editing: multiline Markdown list continuation was normalized by editor input; no source-parser bug is inferred from that driver effect.
3. Added todo through real composer, opened detail (hover reveals Comments control), posted comment, promoted task to Backlog. Added second card. Trusted keyboard Space→Right→Space moved first card Backlog→Doing, then Doing→Done after backend settled; Done checkbox checked and stored prior lane remained doing. Cancel probe Space→Left→Escape kept exact todos.md bytes and restored drag-handle focus; live status Move cancelled. Task unchanged. Explicit detail Move select from Done→Doing reopened it. Unboard retained task/comment, re-promote put it in Backlog; source task order stayed first/second/third throughout.
4. Added hand-written third fixture task through external source append, observed no ID on read and Adopt task & comments. Explicit detail opening adopted it (id=ecxkwqr74o); settled detail was canonical. Completed, hide/show completed and reopen were exercised. Brief transient removed-task/loading detail appeared while adoption response/refresh settled; not classified as persisted corruption.
5. Created decision Preserve canonical Markdown with decided=2026-10-01; edited body and preserved recorded=2026-10-07T02:54:22.602775196Z and decided date. Replaced it with successor. Exact Buffer comparison of predecessor before/after replacement was true (unchanged bytes). History/search canonical/oldest controls exercised; history predecessor shown Replaced. Search/filter dispatch is asynchronous; captures were taken after settled text where material.
6. Scratchpad local unsaved draft survived Close/Open. External fixture-only edit to scratchpad.md produced real conflict, retained draft and displayed saved version. Reload (discard my draft) restored external saved content. Keep mine was inspected, not dispatched. No synthetic conflict transport.
7. Created second owned fixture Space w2. Catalog chose exact UUID Attached elsewhere. Attach opened confirmation. Keep current association restored DOM focus to Attach. Second Attach→Attach here transferred same UUID; original w1 then showed No notes for this Space, destination kept all three todos, comments and both decisions. No user binding was touched.
8. Edited existing durable comment; UI retained Local user and original 2026-10-07 02:53:39 UTC created timestamp. Unboarding preserved edited comment. Delete confirmation shown; Keep canceled. No comment deletion proof claimed.
9. Actual layouts captured wide/medium/narrow. Tab Home from focused Todos moved focus to Scratchpad. No native renderer, screen-reader session or touch hardware proof.

## Not exercised / do not infer

Native Tauri/GPU path; pointer/touch drops; duplicate-ID repair; malformed/lazy continuation source; nested removal refusal; localStorage failure/quota; removed/orphan decision/comment draft recovery; oversized payload bounds; contention, symlink/no-follow safety and external-writer race; decision source stale/already-replaced refusal; confirmed comment deletion; unknown-outcome read/acknowledge/remount fault injection; every loading/error combination. These are atlas C evidence only, not proven runtime by screenshots. No test/build/lint runs performed because this is documentation/design investigation.

## Verification ownership

Only owned fixture Notes were mutated. No product source, installed config, user Notes, live user binding, provider, commit/push or widget publication. Screenshots persisted under requested atlas evidence path. Parent publishes presentation after review. Final presentation checks and cleanup recorded separately once executed.

## Owned runtime cleanup

The notes-atlas-owned Chromium tab was released. scripts/verify/ui_polish_runtime.py stop /tmp/cpol-6y_a886d returned “Stopped matching fixture processes”; runtime.json root/session were checked against the captured ownership identity, then this owned temporary root was removed. Requested screenshots and evidence remain under planning/notes-atlas-2026-10-07/evidence. No persistent user environment was selected for automation.
