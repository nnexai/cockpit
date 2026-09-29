# Illustrative state transitions (planning example, not product fixtures)

Notation: `row[a .5, b .5]` is a row split with child weights; `T:w1:p1` a terminal leaf (Herdr pane id), `F`/`R`/`B` the tab's Files/Review/Browser leaf (`${tabId}:files` …). `sel`, `real`, `zoom` are `selectedLeafId`, `lastRealLeafId`, `zoomLeafId`. `focus` is the observed Herdr focus triple `(space, tab, pane)`.

## E1 First load (deterministic pane-id order, geometry ignored)
Snapshot tab `w1:t1` members `w1:p10, w1:p2, w1:p1` (any array order), focus `(w1, w1:t1, w1:p2)`.
- Order by `compareStablePaneId` (digit runs by integer value, other text by UTF-16 code unit, ties by whole-string code-unit order): `p1, p2, p10` → first-load grid (01-design.md §3.1) → `row[T:p1 .5, T:p2 .5]` for 2; for 3: `col[row[T:p1 .5, T:p2 .5] .5, T:p10 .5]`.
- `sel = real = T:w1:p2` (focus class `initial`). No Herdr request.

## E2 Unchanged focus while a viewer is selected
State: `row[T:p1 .5, F .5]`, `sel = F`, `real = T:p1`, `observedFocus = (w1, t1, p1)`.
- Stream snapshot (agent status change, same focus triple) → class `unchanged` → no selection change, no DOM focus change, no attach churn.
- Terminal `T:p1` stays control-attached (attach mode never depended on selection); `controlAllowed(p1) = false` because `sel ≠ T:p1`.

## E3 Echo of Cockpit's own request after the user moved on
User clicks `T:p2` (sends `pane` focus p2, echo token 7), then clicks `F` before Herdr confirms.
- Local: `sel = F` immediately; Herdr focus request is not cancelled.
- Snapshot focus `(w1, t1, p2)` matches echo 7 → class `echo`: consume echo, `real = T:p2` only if `sel` is still `T:p2` (it is not), selection stays `F`.

## E4 External focus change (TUI or another client)
State as E2. Snapshot focus `(w2, w2:t3, w2:p5)` with no matching echo → class `external`:
- `activeSpaceId = w2`, `activeTabId = w2:t3`, that tab's `sel = real = T:w2:p5`; if `w2:t3.zoom` is another leaf → `zoom = null`.
- Effects: `cancel-transient(external-focus)` (header drag cancelled without drop; divider drag ends keeping live weights); Library stays open.
- A later snapshot with the same triple is `unchanged` and never re-selects `T:w2:p5` after the user picks a viewer.

## E5 New terminal while Files is selected (trustworthy creation identity)
State: `row[T:p1 .5, F .5]`, `sel = F`, `real = T:p1`. User: Split right.
- Request: `pane_split {pane_id: p1 (runtime source = real), direction: right, ratio: null}`; `pendingCreation = {tab t1, placeBeside F, dir row}`.
- If a stream snapshot shows new member `p3` first: `p3` goes to `heldMembers` (not rendered, not treated as external); a changed focus triple naming held `p3` is buffered with it.
- Response `created = {pane_id: p3, terminal_id: term_x, space_id: w1, tab_id: t1}` (validated by adapter against post-mutation snapshot, see 03-contract-evidence.md) → `splitLeaf(F, row)` → `row[T:p1 .5, row[F .5, T:p3 .5] .5]` normalised to `row[T:p1 .5, F .25, T:p3 .25]`; `sel = real = T:p3`. The buffered focus `(w1,t1,p3)` equals `created.pane_id` → echo.
- Any other held member not equal to `created.pane_id` → external rule (E6); a buffered focus naming it → external (selects it after insertion).
- A focus change to an already-known pane (e.g. TUI focuses `p1`, or a pane in another tab) while the creation is in flight is classified immediately → external (E4), never swallowed as a creation echo.
- Outcome unknown / error: `pendingCreation = null`, every held member → external rule, buffered focus → external; no automatic retry.

## E6 External terminal insertion (visible = unzoomed membership)
State: `row[T:p1 .6, col[T:p2 .5, F .5] .4]`, zoom `F`. Snapshot adds `p4` (not attributed).
- N = 3 leaves in the tree (zoom-hidden ones count). `insertRootEdge(row, after)`: `row[T:p1 .45, col[…] .3, T:p4 .25]`; internal col ratios unchanged.
- Focus unchanged (`unchanged`) → zoom stays `F`, selection unchanged. If the snapshot also focuses `p4` (external) → E4 rules: unzoom, select `T:p4`.

## E7 Drag header to centre (swap) and edge (restructure)
`row[T:p1 .5, col[T:p2 .5, R .5] .5]`.
- Drop `R` on centre of `T:p1`: `swapLeaves` → `row[R .5, col[T:p2 .5, T:p1 .5] .5]`; weights travel with positions; no Herdr mutation.
- Drop `T:p1` on bottom half of `T:p2`: `removeLeaf(p1)` then `splitLeaf(p2, col, after)` → `col[T:p2 .25, T:p1 .25, R .5]` (flattened); selection = dragged leaf.

## E8 Viewer reuse and source switch
`F` exists with companion root of `p1`. User runs Open Files from `T:p2` (folder root).
- No split. Core `viewer_open(tab t1, kind files, source_pane p2, selector files_folder)` replaces the tab's Files source and returns a new `binding_id`; unzoom if `F` hidden; `sel = F`, header flash.
- The slot keeps `viewsBySource[old source]` (including any unsaved comment editor text) for the run; switching back restores it. Durable comment batches keep their original `source_kind/source_id`; the new source shows its own batch or an empty one.

## E9 Last real terminal closed / moved away
`row[T:p1 .5, col[F .5, B .5] .5]`. User closes `T:p1` (confirmation names viewers).
- `pane_close p1` succeeds; snapshot no longer lists tab `t1` (Herdr closes an emptied tab [INFERENCE]) or lists it without members.
- Confirmed loss → effects `browser-retire(t1)` (draft guard → stop session → remove the tab's proven profile/workspace/config/receipt), `viewer-release(t1, files)`; tab layout dropped; comment batches, browser drafts/feedback, Library and vault untouched. Removal failure → in-memory cleanup notice (strip with Retry cleanup / Dismiss).
- `sync = stale/disconnected` instead → no effect (not membership loss).

## E10 Server restart
Snapshot `server_instance` differs from the stored one → drop every tab layout for the session, first-load again (E1). Pane ids like `w1:p1` may be reused by the new server; a stored `terminal_id` mismatch for a surviving pane id likewise removes and re-inserts that leaf.

## E11 Browser lifecycle per tab (core receipt states in `tab-associations/<key>.json`)
- Open Browser in tab `t1` with no leaf → leaf `B` (status `opening`) → `BrowserAction::OpenFresh{url:null}`: receipt absent/closed → launch at `default_url`; receipt `open` (survivor from a page reload or another window) → stop → remove proven artifacts → launch fresh at `default_url` (never adopted, C17) → `open`.
- Close `B` → guard (drafts durable) → `BrowserAction::Close` → receipt `closing` → CLI `close` confirmed → `cleanup_pending` → profile/workspace/config removed by no-follow walk under the browser root → receipt file deleted → response `connection: absent, cleanup: done`.
- Stop fails → receipt `outcome_unknown` or error, leaf stays `close_failed` with Retry close / Dismiss (Retry issues `Status` first).
- Stop ok, removal fails (locked file) → receipt `cleanup_failed{reason}`, leaf removed, cleanup notice for `t1`; Open for `t1` disabled until `BrowserAction::Cleanup` succeeds or notice dismissed; the next `OpenFresh` retries cleanup before launching.
- Tab `t2`'s receipt, profile and page are never touched by any of the above.

## E12 Legacy Space browser cutover (operator-scoped, no silent deletion)
- Host start finds `associations/K.json` (stem = key `K`). Core stops `cockpit-K`, confirms stopped, writes `legacy-archive/K.json` {key, endpoint, session, Space `w1` "api", archived_at}, removes the receipt from `associations/`.
- It opens `profiles/K`, `workspaces/K` and `configs/K.json` without following links and records the manifest: `profiles/K` dir dev 64768 ino 1201, `workspaces/K` dir ino 1202, `configs/K.json` file ino 1203. A symlink at any of these paths is recorded under `not_candidates` and never opened. Nothing is deleted.
- The strip shows "Saved before tabs: Space api": 2 saved annotations, 1 draft, and "Review 3 items". Saved work is reachable with `scope: LegacyArchive{K}`. Sending asks for a recipient from the agents of the currently focused tab; the send revalidates that recipient and pastes without submitting.
- The user clicks `Remove these items`. The request echoes the 3 manifest rows. Core re-opens each path; `profiles/K` now has ino 1300 (replaced since review), so it is preserved and marked `changed`. The other two match and are removed.
- `Keep` would mark all three `kept` and delete nothing. `legacy-archive/K.json` is deleted only when no saved work and no unresolved candidates remain.
