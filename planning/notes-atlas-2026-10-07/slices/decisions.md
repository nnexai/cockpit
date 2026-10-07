## Notes Decisions implementation atlas (code-derived; no runtime proof)

Scope: Current/History/search/order/list/detail/new/create/edit/replace/back/cancel/dates/metadata and replacement conflicts. Evidence below is source inspection only. No live user data, fixture, UI, build, or tests were exercised. No files were changed.

### N-DEC-01 — Collection: Current/History, search, ordering, empty/loading/error

**Owner and evidence:** `src/app/notes/Decisions.tsx:8-42,98-106` (`Decisions`, `ordered`, collection controls/list); core `crates/cockpit-core/src/notes/decisions.rs:523-550` (`execute`, `DecisionList`); protocol `crates/cockpit-protocol/src/notes.rs:54-59,129-132` (`NotesDecisionFilter`, `DecisionList`). Styling: `src/app/notes/notes.css:186-204,261-267`.

- Initial filter is Current; query is empty; ordering is newest; list records/results initialize empty. List requests debounce 160 ms and issue `decision_list` with selected status and query. Changing model decision summaries also re-triggers the request. Requests cancelled by component effect cleanup are ignored (`Decisions.tsx:28-42`).
- Search input is explicitly labelled “Search decision title and body” (`Decisions.tsx:105`). Core lowercases query and matches the parsed title or the post-title body text, not arbitrary frontmatter/preamble. Search is case-insensitive substring matching (`decisions.rs:524-547`).
- Current and History correspond to derived status: a record is History/Replaced if any valid parsed successor record links to it with `replaces`; otherwise Current. `all` exists in protocol/core but the UI selector exposes Current and History only (`decisions.rs:324-377,529-540`; UI `Decisions.tsx:105`).
- UI order sorts `recorded` instants newest-first by default, oldest-first on toggle, with ID tie-break. Invalid/missing UI dates sort after valid ones; if both invalid, ID order is used (`Decisions.tsx:98-103`). Core also sorts parsed recorded instants descending then ID before filtering (`decisions.rs:368-376`); UI re-sorts selected-filter results. File names are not chronological authority.
- While loading, `#decisionList` sets `aria-busy`; it does not show a dedicated loading label/spinner. Empty results are suppressed during loading (`Decisions.tsx:106`). After completion, empty list displays “No [filter] decisions yet” or “No [filter] decisions match …”. Empty-state action is New decision for unfiltered results or Clear search for a query. If query has no match in selected status, UI makes a second request for opposite status and offers a count/link to switch (`Decisions.tsx:31-39,106`).
- Read failure sets the local error string shown as `role=alert` in detail area (`Decisions.tsx:108`). A successful list request clears that local error. Wrong result-kind responses return without installing records or a dedicated new error, but the effect's `finally` still clears loading (`Decisions.tsx:32-39`).
- Sorting toggle says “Newest” or “Oldest”; native button/select/input semantics and labels provide keyboard-accessible controls. Rows are buttons; selected styling is conditional on selected ID and not creating (`Decisions.tsx:105-106`; CSS `notes.css:197-204`).

### N-DEC-02 — List selection, detail, back, and navigation

**Owner and evidence:** `src/app/notes/Decisions.tsx:43-57,106-114` (`decision_get`, row selection and detail); `src/app/notes/NotesView.tsx:17-33,48-51,150` (`ViewState`, persistence and Decisions panel); CSS `notes.css:205-221,261-265`.

- Selecting a row calls the outer `onSelect(id)`, hides New and switches from list presentation to detail (`Decisions.tsx:106`). `selectedId` is held at Notes surface level and persisted per Notes UUID in memory/localStorage (`NotesView.tsx:17-33,150`; persistence effect `NotesView.tsx:48-51`). Decision selection is not reset by tab changes in this component.
- On selected ID, the component requests `decision_get`, updates full record, clears local error, and reconciles any matching edit draft against returned title/body and revision. Failure sets local alert and clears record (`Decisions.tsx:43-57`). A changed summary revision also re-runs fetch (`Decisions.tsx:21,57`).
- Detail shows title and rendered Markdown body, recorded timestamp/status, and an expandable Details disclosure. Details includes immutable recorded value, decided date, relative reference, absolute file path, predecessor/successor navigation links, source problems, and Replace action (`Decisions.tsx:111-113`). Missing predecessor display is labelled “(missing record)”; successor link falls back to ID if no summary title is cached. Parsed source `problems` are displayed (`decisions.rs:193-247`; UI `Decisions.tsx:113`).
- “Back to decisions” only sets `showList=true`; it does not clear selected ID/record/draft. CSS hides detail/list responsively: desktop keeps both panes and hides Back; narrow layout shows either list or detail and exposes Back (`notes.css:205-221,261-265`).
- No decision-specific focus transfer or focus restoration is implemented around row selection, opening detail, Back, New, or Cancel in `Decisions.tsx`; interactions use ordinary buttons/inputs/editor. This is code-derived; actual focus behavior is not runtime-tested.

### N-DEC-03 — New decision draft/create/cancel

**Owner and evidence:** `src/app/notes/Decisions.tsx:58-76,105,110` (`beginNew`, `saveNew`, form); `src/app/notes/drafts.ts:1-13,43-74` (`DecisionDraft`, draft storage); core `decisions.rs:414-430,455-482,607-614,679-720` (`validate_payload`, `new_content`, create); protocol `notes.rs:136-140`.

- New initializes/retains a `newDecision` draft `{title:"", body:"", decided:"", replaces:null, revision:null}`; New button becomes “Resume draft” when one already exists. It switches into write view. Draft values persist through `updateDrafts` and per-Notes draft storage (`Decisions.tsx:58-76,110`; `drafts.ts:1-13,43-74`).
- Inputs: title maxLength 512; optional historical “Decided date” placeholder `YYYY-MM-DD or RFC3339`; Markdown editor. UI says Recorded date is set only on save. Core creates `recorded` at publication time, and stores optional `decided` (`Decisions.tsx:110`; `decisions.rs:455-482`).
- Record button disabled while shared mutation busy, blank/whitespace title, unknown outcome, or replacement blocked. `saveNew` additionally prevents duplicate invocation through a ref. Successful typed decision response selects the new ID, shows detail, and removes the new draft only if title/body/date still match the submitted values; otherwise edits made in flight remain (`Decisions.tsx:62-76,110`).
- Cancel (discard draft) is disabled only while `model.busy`; it deletes `newDecision` and hides the editor, without changing selected ID or explicitly focusing another control (`Decisions.tsx:110`). The button text makes data loss explicit. No separate cancel confirmation is coded.
- Core title must be nonempty/single-line (no control characters), title <=512 bytes and body <=256 KiB; optional decided must be exact valid calendar date or RFC3339 instant; failures return `notes_invalid_input`/`notes_too_large`. Collection caps are 4,096 directory entries and 64 MiB aggregate, each file <=256 KiB (`decisions.rs:13-18,161-182,414-430,679-720`). UI `maxLength` counts browser string units, while core bound is byte length; core remains final authority.
- Save errors are handled by shared `model.mutate`, rendered in the outer Notes problem area (`useNotes.ts:98-146`; `NotesView.tsx:152-160`). The create UI does not itself render a field-level validation message or mark decided-date input invalid; observed errors are global.

### N-DEC-04 — Edit existing decision and metadata preservation

**Owner and evidence:** `src/app/notes/Decisions.tsx:43-57,77-97,111-112` (`decision_get`, `saveEdit`, edit UI); `drafts.ts:15-23` (`reconcileDraft`, `changedDraft`, `acknowledgeDraft`); `useNotes.ts:76-96,98-137`; core `decisions.rs:445-452,560-606,653-677`.

- Edit creates per-ID title/body drafts from the fetched record and revision if absent, then enters editing. Markdown can switch Source/Preview; CodeMirror `Mod-s` invokes supplied save callback and editor textbox has `role=textbox`, `aria-multiline=true` (`Decisions.tsx:111-112`; `MarkdownEditor.tsx:36-55,90-93`).
- Save requires dirty draft, no unknown outcome, and no unresolved conflict unless Keep mine is explicitly selected. It sends both title and body, with expected revision derived from dirty body when body changed, otherwise title revision (`Decisions.tsx:77-80`). Core compares expected revision against source revision before update; stale result is `notes_conflict` (`decisions.rs:445-452,560-568`).
- Core updates splice into original Markdown rather than regenerate frontmatter. Recorded/decided/replaces and unknown metadata remain byte-preserved; title edits replace the first recognized H1 text or insert a heading when none exists; body changes replace body after the first title boundary (`decisions.rs:653-677`). Unterminated frontmatter refuses edit (`decisions.rs:569-576`).
- A confirmed save advances base/revision but preserves text typed during the in-flight request; clean saved draft is removed and editing closes; newly typed residual draft remains in edit mode (`Decisions.tsx:81-96`; `drafts.ts:15-23`).
- “Back / keep edit” exits editing without deleting draft; Edit button then offers “Resume edit · draft kept” when dirty. “Discard edits” removes only this record's edit draft and exits edit. These actions are not disabled during busy state in the JSX (`Decisions.tsx:111-112`). Save button is disabled when clean, busy, or outcome unknown.
- External revision detection occurs during `useNotes.refresh`: matching decision draft revision mismatch marks both title/body conflict; `decision_get` reconciliation retains dirty content and sets conflict when draft revision differs from freshly fetched revision (`useNotes.ts:76-96`; `Decisions.tsx:43-57`; `drafts.ts:15-17`). Conflict panel renders saved title/body preview plus Keep mine and Reload. Reload explicitly replaces both draft values/base/revision with saved content. Keep mine sends the current record revision as expected revision, an explicit user-authorized overwrite after saved content is shown (`Decisions.tsx:77-80,112`).

### N-DEC-05 — Replace and source conflicts

**Owner and evidence:** `src/app/notes/Decisions.tsx:25-27,62-76,109-113`; core `decisions.rs:615-640,679-720`; filesystem `crates/cockpit-core/src/notes/fs.rs:229-271`; tests `decisions.rs:848-897`.

- Replacement creates a new decision linked by `replaces: <oldId>`; old Markdown remains untouched. Replace action only enabled for Current record when not busy and no new draft; already-Replaced shows “Already replaced. Follow the successor instead.” Existing new draft shows a warning to finish/cancel it. New replacement draft starts with source title, empty body/date, source ID and source revision (`Decisions.tsx:113`).
- UI blocks replacement draft when source missing, source now Replaced, or source revision differs from draft revision, once `model.scratchpad` has loaded (the truthy scratchpad condition gates this UI calculation). Messages preserve draft and provide Review source; if source still current, “Use current source revision” explicitly advances captured revision (`Decisions.tsx:25-27,109`). Code-derived potential: while Notes bootstrap has not populated scratchpad, this guard is false, so the UI may transiently permit submission; core checks revision/status and publication source again, so this is not a core bypass.
- Replace request includes captured source revision. Core rejects stale revision (`notes_conflict`) and already-replaced source (`notes_decision_replaced`); generated successor gets fresh recorded timestamp and `replaces` link. The publisher re-reads source revision immediately before rename and refuses if it changed (`decisions.rs:615-640,679-720`; `fs.rs:229-271`).
- For a replacement conflict returned by mutation, shared model error appears in outer alert; refresh updates summaries, where replacement draft blocker can display. Draft is not automatically retargeted/retried. Replacing source still leaves predecessor unchanged and status/successor list are derived from successor links on collection scan (`decisions.rs:324-377`).
- UI has no operation to delete a decision; replacement is additive/history preserving (protocol operations `notes.rs:129-153`; UI `Decisions.tsx:113`).

### N-DEC-06 — Dates, provenance and source metadata

**Owner and evidence:** `src/app/notes/Decisions.tsx:106,110,113`; protocol `crates/cockpit-protocol/src/notes.rs:312-329`; core `crates/cockpit-core/src/notes/decisions.rs:161-182,193-247,414-482`.

- `recorded` is generated by core on create/replace, not user editable. List formats valid recorded dates using local `toLocaleDateString`; missing/invalid displays “Recorded: unknown”. Detail footer formats valid timestamp with `toLocaleString`; detail disclosure shows raw recorded string (or unknown), decided date (or unknown), relative path and absolute path.
- `decided` is optional user-provided historical date, accepted only exact `YYYY-MM-DD` valid calendar date or RFC3339 timestamp. Imported invalid dates become `None` and add `decided_invalid` problem; malformed frontmatter clears recognized metadata and adds `frontmatter_malformed`. Invalid recorded value adds `recorded_invalid`; recorded must parse RFC3339 for sorting/display as trusted date (`decisions.rs:161-182,193-247`). No missing date is invented on read.
- Full summary carries stable ID, title, recorded, decided, replaces, replaced_by, status, revision and problems; full record adds Markdown body, relative path and absolute path (`notes.rs:312-329`). Replacement status and successor IDs are derived from links, not persisted state (`decisions.rs:324-377`). Details exposes source issues rather than concealing them (`Decisions.tsx:113`).

### N-DEC-07 — Pending, unknown outcome and storage recovery

**Owner and evidence:** `src/app/notes/useNotes.ts:43-64,98-137,138-168`; `NotesView.tsx:152-160`; `drafts.ts:43-74`; `Decisions.tsx:25-27,62-97,110-113`.

- Writes are serialized per Notes UUID. Shared `busy` gates create, replacement, edit save; create says “Recording…” while busy. A mutation returning a decision invokes success handling before refresh; a subsequent refresh failure is presented as “Change saved, but refresh failed…” (`useNotes.ts:98-146`).
- A transport/ambiguous write error becomes `notes_outcome_unknown`, stores the warning in drafts, and blocks subsequent writes both in shared mutate and decision save controls. UI says saved state must be checked before another write because retry may duplicate. Outer Notes surface supports Check saved state; after successful read, user sees review prompt and must explicitly acknowledge potential duplicate before writes re-enable (`useNotes.ts:28-40,98-146`; `NotesView.tsx:152-160`). This recovery is shared rather than decision-specific.
- Drafts are keyed by Notes UUID in app memory/localStorage. If browser storage write fails, draft remains for app session and a visible warning says not durable after restart (`drafts.ts:69-74`; `useNotes.ts:55-62`; `NotesView.tsx:152-160`). Surface selection separately persists per UUID; that storage failure is silently caught (`NotesView.tsx:17-33,48-51`).
- A failed `decision_get` clears displayed record and reports error; it does not delete the retained edit draft. On reselect/reload, reconciliation can surface conflict. A generic failed list read is an error, not an empty-state claim. No runtime recovery/focus behavior was demonstrated.

### N-DEC-08 — Keyboard and DOM focus contract

**Evidence:** `NotesView.tsx:127-135,150`; `Decisions.tsx:104-117`; `MarkdownEditor.tsx:36-55,90-93`; `notes.css:261-265`.

- Parent Notes tabs use tab semantics and ArrowLeft/ArrowRight/Home/End navigation; Decisions is the final tab (`NotesView.tsx:127-135,150`). Decision pane is a `tabpanel` labelled by its tab.
- Decisions controls are native input/select/button/details/summary elements. Search, status, sort, list rows, editor source/preview, date/title inputs, metadata disclosure links, conflict options and save/back/cancel are therefore available to standard keyboard activation. Markdown textbox is explicitly labelled and multiline; Mod-S is save shortcut (`MarkdownEditor.tsx:52-55`).
- Mobile/narrow Back is displayed and desktop Back hidden. Row selection/detail changes do not explicitly move focus; no focus return target is recorded. This is an implementation observation, not a runtime accessibility verdict.

### Existing test evidence (references only; not executed)

- `crates/cockpit-core/src/notes/decisions.rs:848-897` — `replacement_preserves_old_bytes_and_refuses_second_replacement`: predecessor bytes/revision unchanged, successor link/status derived, second replacement refused.
- `decisions.rs:899-948` — `filtering_search_and_sort_use_frozen_instants_not_filenames`: sort by recorded instant (timezone equivalent), case-insensitive body search and status filtering.
- `decisions.rs:950-1001` — `updates_are_cas_and_never_rewrite_metadata`: stale revision conflict, ordinary update preserves recorded/decided/replaces/custom frontmatter, publisher detects stale bytes.
- `decisions.rs:775-847` — `frontmatter_scalars_are_bounded_and_top_level`, title splice and invalid dates/recorded tests.
- `decisions.rs:1003-1067` — collection/payload limits; `1072-1103` heading source-boundary regression; `1105-1134` imported title limits; `1135+` exact date validation.
- Frontend `src/app/notes/NotesView.behavior.test.tsx:82-83` only provides a mocked empty `decision_list` response in inspected hit; no decision interaction/regression test surfaced in the inspected frontend test files. `notesState.test.ts` yielded no decision-related search hit. This is a search observation, not a claim that no test exists elsewhere.

### Evidence-backed UX/behavior observations (not proposals)

- The list has `aria-busy` but no explicit loading announcement/spinner, and when selected record fetch is in-flight the prior detail can remain rendered until response (`Decisions.tsx:43-57,106`).
- Detail/read errors are inline within the detail pane, whereas write/unknown-outcome recovery appears in Notes-wide problem slot; this split is code-derived (`Decisions.tsx:108`; `NotesView.tsx:152-160`).
- “Back / keep edit” retains the edit draft but exits editing; this differs from explicit “Discard edits” and “Reload (discard my edits)” which do discard it (`Decisions.tsx:112`).
- Decision UI list date uses locale date without time while detail footer uses locale date/time; unknown/improper date shows different labels (“Recorded: unknown” list/footer and raw `unknown` in details) (`Decisions.tsx:106,113`).
- No decision UI test was identified in the searched frontend test owners; core behavior is directly covered by named Rust tests above. No finding here establishes runtime usability or accessibility.