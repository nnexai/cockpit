# Supervisor redesign implementation plan (F1–F13, all 22 scenarios)

Planner: `SupervisorImplementationPlan`, 2026-10-07, for canonical task `7c3754f2-b280-4911-a98a-f668f53d4ecc` (run `21cd41d8-…`). This is a plan only. The plan agent did not edit product code, tests, docs or accepted artifacts, and ran no builds, tests, fixtures or browsers.

Companions: [`COVERAGE.md`](COVERAGE.md) (F1–F13 and scenario-by-scenario verification map), [`examples/contracts.md`](examples/contracts.md) (TypeScript interface sketches), [`examples/fixtures.md`](examples/fixtures.md) (typed DTO fixture snippets for tests and the harness). Examples are written as Markdown on purpose: `tsconfig.json:20` type-checks only `src`, and Vitest's default include picks up `*.test.*` files anywhere.

Accepted inputs (unchanged, read-only): `planning/supervisor-design-review-2026-10-06/DESIGN.md` (DESIGN), `PRESENTATION.html` (PRES), `verification/bounded-redesign-verification.md` (BOUNDED).

## Parent-reviewed execution corrections (2026-10-07)

Parent released the product-edit barrier through inbox sequence 12 under the existing exact Execute grant. These corrections override conflicting slice/check sketches below:

- Q-A option 3 is approved: count root Decide entries for “need you” and show an accurate Recover indication; the broader `RootSummary.needs_you` is not user-decision truth.
- Q-B omission is rejected. Archive-open performs bounded read-only closed-root snapshots through the existing API, fenced by current session/root/generation identity. Deliver every requested task count, with explicit loading/unavailable states; label `updated_at` as updated, never as an invented closure timestamp.
- Permanent tests cover consumer-visible boundaries, transitions, retained operations/drafts and authority invariants. Omit label-only/default/source/wiring tests proposed below; use browser or throwaway checks for those acceptance dimensions.
- Available LSP references are required before changing exports. Planner’s grep evidence is only a handoff, not permission to skip the available parent LSP device.
- Notes files and accepted Notes artifacts belong to the Notes worker. App/global styles/UiIcon changes require evidence and parent coordination. Fresh surgical Supervisor doc edits preserve widget and concurrent Notes sections.
- Use isolated Vite build output/runtime until the final shared build is coordinated. Preserve committed widget anchor repair `b538420`. No auto-retirement, commits or pushes.


---

## 1. Outcome

The accepted Supervisor design is implemented in `src/app/supervisor/`, covering every recommendation F1–F13 and the 22 observable acceptance scenarios (DESIGN:496-519):

- **Tasks** is the initial view. A labelled `Tasks n` / `Graph n` switch is shown at every size, and the Tasks/Graph choice is remembered per root.
- The **Graph** view uses the whole workarea and has a Task column. Nodes are 240×48 with two rows. Details open in a side panel, a bottom sheet or an overlay, depending on size. A details splitter resizes them by pointer and keyboard and exposes ARIA values.
- One attention model drives the summary bar counters, the queue, card and node badges, and the filter. It is built from core `snapshot.attention` plus explicit local conditions, with the tiers Decide, Recover and Notice.
- Other accepted changes: review-first details, recovery that shows one next step, two-line cards, the relationship path, dialog fixes, header naming, readable Activity/Diagnostics, single empty/offline states, narrow lane groups, and keyboard/accessibility consolidation.
- Unchanged: every authority guard (§3.4), exact revisions, explicit-Result acceptance, unconfirmed-operation and draft locks, the reported/observed distinction, and the rule that selection, reveal, view switching and resizing never change Herdr focus.
- Docs are migrated. Verification evidence is written to a **new** path under this folder. Accepted artifacts are never overwritten.

## 2. Evidence (current source, fresh reads 2026-10-07)

### 2.1 Authorities read

- `.omp/AGENTS.md:1-8`: UI changes are verified in the browser build against a disposable fixture. Native-only changes also need a native run.
- `.omp/RULES.md:1-8`: "fixed" only after running the scenario; disposable Herdr sessions only (`scripts/verify/ui_polish_runtime.py start|stop`).
- `CONTEXT.md:322-344` (§5.7), `DECISIONS.md:54-63` (Supervisor orchestration), `CODE_GUIDE.md:20-23,52-66`, `docs/supervisor-surfaces.md`, `docs/keyboard-shortcuts.md:9`.
- DESIGN in full (1-601). PRES demo logic: `panelMode`, `applyLayout`, `setView`, `showIn`, `graphKey`, `boardKey` (PRES:1381-1594). BOUNDED:1-43.
- Scout reports `SupervisorModelsScout`, `SupervisorNavigationScout`, `SupervisorVerificationScout`. Their ranges were re-read against the actual files. Corrections are noted inline: Actions is 190 lines, Dialogs is 107.

### 2.2 Current UI facts

| # | Fact | Source |
|---|---|---|
| E1 | `supervisorForest` builds run rows plus `${run}:${sub}` subagent rows. Visited sets keep cyclic or unreached rows exactly once. It is filtered to the selected root and non-closed runs. | `SupervisorView.tsx:16-38,104` |
| E2 | The graph maps rows to `graphLayout` nodes. Task refs exist only for `kind==="worker"` rows whose `task_id` resolves; they are a separate "Task links" column. Unassigned tasks have no node. | `SupervisorGraph.tsx:31-46,117-122` |
| E3 | `graphLayout` is a deterministic left-to-right layout: 180×36 cards, 36/8 gaps, 8 padding, lexical sibling order, missing or self parents become roots, a cycle loses the edge at its lexically smallest member, traversal is iterative, and parents sit at the centre of their span. | `graphLayout.ts:1-104`; tests `graphLayout.test.ts:5-95` |
| E4 | Graph keyboard: one roving stop via `useRovingList`, which moves DOM focus only. Escape clears selection. Nodes expose `aria-expanded`. There are no tree keys. `useRovingList` already accepts `onKey` for ← →. | `SupervisorGraph.tsx:67-77`; `useRovingList.ts:10-64` |
| E5 | `narrowView` and `graphHeight` are component state. The narrow switch (Board/Agents) shows only at ≤719 container or ≤600 viewport height. | `SupervisorView.tsx:65-66,303`; `supervisor.css:149,151-169,185-193` |
| E6 | The graph band is 220 px (100 px with attention) with `max-height:60vh`. Its `RowSplitter` "Resize agents overview" has min 80, max 0.6×window, and no `aria-value*`. | `supervisor.css:53,204`; `SupervisorView.tsx:306`; `RowSplitter.tsx:17-52` |
| E7 | The attention region is 44 / 164 / 110 px with `overflow:auto`. `hasAttention` is a third local predicate. | `supervisor.css:33,183,186,203`; `SupervisorView.tsx:226` |
| E8 | The card `needsAttention` predicate omits observed-blocked workers. The filter dims with `opacity:.58`. | `SupervisorView.tsx:249-250`; `supervisor.css:95` |
| E9 | `snapshot.attention` is never read by the UI. The test fixture always sets `attention: []`. | grep `src/app`; `SupervisorView.test.tsx:24` |
| E10 | `ScopeDrafts` is per session and per root, in memory, with lazy scopes. `disclosures.agents` is declared and unused. | `useSupervisorDrafts.ts:5-39` |
| E11 | Detail panel is `flex:0 0 340px; max-width:42%`. At narrow it becomes `inset:0` and the content is `visibility:hidden`. | `supervisor.css:109,165,179` |
| E12 | Focus moves to Close only when the measured width is between 0 and 720 (exclusive). An unmeasured 0 is treated as wide. | `SupervisorView.tsx:227-232` |
| E13 | `agentState` is the shared state interpretation. `AgentRecovery` renders four same-weight buttons. | `SupervisorActions.tsx:8-36,90-106` |
| E14 | The accept guard requires `run.stage==="reported"`, then disables on `busy \|\| !live \|\| !task \|\| diagnostic \|\| task.current_run_id !== run.run_id`, and sends `expected_task_revision = task.task_revision`. | `SupervisorActions.tsx:184` |
| E15 | Dialogs: portal, initial focus once, Escape when idle, Tab wrap, unconfirmed lock for non-edit modes. Edit with a deleted task keeps Save enabled. Dismiss copy is Keep tracking / Back / Cancel. | `SupervisorDialogs.tsx:26-41,42-79,82,94,105` |
| E16 | One panel has three names: header "Activity", aside "History", heading "Earlier". | `SupervisorView.tsx:277,326,333` |
| E17 | The question and answer flow uses a local `answered` check. Focus returns to the task or Start when the question disappears. The section's `aria-label="Needs you"` drives `questionHadFocus`. | `SupervisorView.tsx:193-201,267,290` |
| E18 | Only `onTerminal` reaches Herdr focus. Existing tests assert `onTerminal` is not called for graph and board navigation. | `SupervisorView.tsx:157-161`; tests `:245,433,491` |
| E19 | Snapshots are fenced by scope generation and revision. Mutations are fenced by exact hashes and operation IDs. | `useSupervisor.ts:6-15,41-96` |
| E20 | The DTO test harness mounts the real `SupervisorView` against a fake `CockpitClient`. | `SupervisorView.test.tsx:1-64` |

### 2.3 Core attention facts

- `derive_attention` per run (`projection.rs:371-508`):
  - Closed runs are skipped.
  - AwaitingPrepare → `awaits_prepare`; Ready → `awaits_execute`; Reported → `to_accept`.
  - SetupUnknown, LaunchUnknown and NeedsReview → `dispatch_unknown`. Error code `plan_changed` → `plan_changed`.
  - An unanswered NeedsInput (no non-stale answer at or after the report) → `needs_input`, for **any** run, root or worker.
  - Stored or Woken briefs older than 120 s → `brief_unread`.
  - Presence Missing → `exited_without_report`. EndpointChanged → `dispatch_unknown`, unless that was already emitted.
  - Present + blocked + no needs_input → `runtime_blocked`.
  - Working + no Result + idle or done for more than 300 s → `idle_without_report`.
  - Unobserved → nothing.
- A conflicted acceptance intent → `intent_conflict` with run_id and task_id (`projection.rs:90-103`). `TaskIntent` carries `intent_id`, `run_id` and `task_id` (`v1.ts`, TaskIntent).
- `RootSummary.needs_you` counts **every** attention entry whose run belongs to that root (`projection.rs:176-197`). That includes the supervisor-owned `awaits_*` and `to_accept` entries, and worker `needs_input`.
- `snapshot.attention` covers runs of **all** roots in the session, not only the selected one. Runs are filtered by session and the set of all root IDs (`projection.rs:45-51,79-89`). The board is only for the selected root (`projection.rs:135-169`).
- Generated types: `AttentionKind` and `Attention` (`v1.ts:1019-1021`), `RootSummary` (`v1.ts:943`), `Run` with `created_at` and `updated_at` (`v1.ts:977`).

### 2.4 Export references (changed exports)

No LSP tool is available in this session. References were taken with repository grep over `src/`. The parent's LSP check of `ScopeDrafts` agrees (consumers are only drafts, View, Graph and Actions).

| Export | Consumers | Disposition |
|---|---|---|
| `supervisorForest`, `ForestRow` (View) | View only (`:17,104`) | Deleted; replaced by `buildSupervisorGraph` (S2) |
| `SupervisorGraph`, `SupervisorGraphRow`, `SupervisorGraphProps` | View `:8,305` | Props replaced (S6); `SupervisorGraphRow` deleted |
| `graphLayout`, `GraphNode`, `GraphPosition`, `GraphEdge`, `GraphLayout` | Graph `:7,43`; `graphLayout.test.ts` | Extended (S2) |
| `RowSplitter` | View `:2,306` only | Deleted in S8 after `PanelSplitter` (S3) lands |
| `taskLanes`, `taskNeighbor` | View `:9,106,313,317` | `taskLanes` unchanged; `taskNeighbor` signature changes (S4); View call updated (S8) |
| `ScopeDrafts`, `newScopeDrafts`, `useSupervisorDrafts`, `messageDraft`, `TextDraft`, `EditDraft` | View, Actions `:5`, Graph `:8`, Dialogs `:7` | `ScopeDrafts` gains `view`; `disclosures.agents` removed (S8) |
| `agentState`, `AgentState` | View, Graph `:6,101`, Actions, tests `:7` | **Frozen**: signature and semantics unchanged |
| `AgentRecovery`, `ObservedEvidence`, `ReportedEvidence`, `RunDiagnostics`, `SupervisorActions`, `taskStatus`, `TextAction` | View `:10`; tests `:7` | Changed in S5 (props additive, see C8) |
| `SupervisorDialogs`, `SupervisorDialogState`, `StartDraft` | View `:11,54,339` | `onCheck` prop added (S7) |
| `copyText` (`library/clipboard.ts:1-9`) | reused by S7 and S10 | unchanged |

No module outside `src/app/supervisor/` imports these, except `App.tsx:54,1190`, which uses only `SupervisorView` with unchanged props.

## 3. Decisions

### 3.1 Contradictions resolved (latest bounded design wins)

| Topic | Stale text | Decision | Source |
|---|---|---|---|
| Node geometry | Appendix G07 "200 px nodes"; DESIGN:9 history "200×36"; current 180×36 | **240×48 two-row node**, CG 32, RG 56, PAD 8, header 28; row = mean of first and last child; 0.4-row gap before the first unassigned task; `y=round(8+56·row)` | DESIGN:176,206-223; BOUNDED:9 |
| Details max width | F4 "≤42% as today" (DESIGN:266); CSS `max-width:42%` | **Side panel default 340, min 280, max 50% of workarea width** | DESIGN:182, scenario 18 (:515) |
| Lanes | docs "five columns" (`supervisor-surfaces.md:11`) | **Six lanes** from `taskLanes` (Queued, Preparing, Ready, Working, Review, Done) with all six counts visible | `boardNavigation.ts:3-7`; DESIGN:392,506 |
| Navigation wording | "Locate", "focus in board/graph", Board/Agents | **Show in Graph / Show in Tasks**, **Tasks / Graph** | DESIGN:9,131,491; scenario 22 |
| Full-graph verification | `full-graph-verification.md` | Historical. BOUNDED is the concept evidence. Implementation evidence goes to a new path (§7). | BOUNDED:40 |

### 3.2 Architecture decisions

- **D1. One attention model, core-first.** New pure module `attention.ts` (S1). It reads core `snapshot.attention` for every kind core derives and applies the tier map below. It adds only explicit local conditions that core does not produce. It never recomputes core thresholds (120 s, 300 s), answer matching, delivery stages or presence inference. There is no production fallback when `snapshot.attention` is empty: tests supply realistic core entries instead (`examples/fixtures.md`). *Rejected:* keeping a local predicate for core kinds, which recreates the O2 drift and duplicates core.
- **D2. Tier map.**

  | Core kind | Tier |
  |---|---|
  | `needs_input` on the selected root run (`run.run_id===run.root_id`) | **Decide** |
  | `needs_input` on a worker | Supervisor-owned state ("Waiting for supervisor"), not queued |
  | `awaits_prepare`, `awaits_execute`, `to_accept` | Supervisor-owned state with core `since` age, not queued |
  | `runtime_blocked`, `dispatch_unknown`, `exited_without_report` | **Recover** |
  | `idle_without_report`, `brief_unread`, `plan_changed`, `intent_conflict` | **Notice** |

  Sources: DESIGN:146-151; Q1 and Q2 defaults (DESIGN:483-484); docs row 23.

  `plan_changed` and `brief_unread` are not named in F2. Notice is the conservative choice (visible, not urgent, no new action) under F2's "core-derived kinds such as idle-without-report".

  | Local condition | Tier | Source |
  |---|---|---|
  | `start_unknown` | **Recover** | `SupervisorView.tsx:283` |
  | `terminal_error`, `navigation_error` | **Recover** | `:284` |
  | `change_unconfirmed` (a `useSupervisor` error while connected) | **Recover** | `:284` |
  | `orphaned_worker` (an open run whose root is closed, including the closed-root descendants path) | **Recover** | `:88,285,286` |
  | `agent_status` (`agentState` kind failure, missing or unknown for a non-proposed run, root or descendant), only when no core Recover item exists for that run | **Recover** | `:192,226,286` |
  | `assignment` (pending or conflict) | **Notice** | `:202,296` |
  | `unidentified_items` | **Notice** | `:299` |
  | `notice` (local info) | **Notice** | `:283` |
  | `root_failed_report` | **Notice** | `:226` |

  Offline or unavailable observation is **not** a queue item. It is the single summary-bar banner (F11, DESIGN:377). "Stale evidence" (F2 Notice) is represented by that banner and by the existing per-row "· stale evidence" label in Activity (`SupervisorView.tsx:333`). Stale message records are not turned into queue rows, which would repeat F11's per-item noise.
- **D3. Dedupe.** Queue rows are keyed by `(tier, subject)`. The subject is `run:<id>`, `task:<id>` or `local:<kind>`, with each ID `encodeURIComponent`-encoded. A row keeps all of its sources in precedence order:
  - Recover: `exited_without_report` > `dispatch_unknown` > `runtime_blocked` > local `agent_status`.
  - Notice: `intent_conflict` > `assignment` > `idle_without_report` > `brief_unread` > `plan_changed` > `root_failed_report` > `unidentified_items` > `notice`.

  The row's `since` is the earliest core `since` among its sources, or null for local-only rows (no invented ages). Order is tier, then `since` ascending with nulls last, then id.
- **D4. Root selector semantics (conflict, see Q-A).** The default implementation shows `Label · N need you`, where N counts the **Decide** entries for that root in `snapshot.attention`, plus `· ⚠ recover` when that root has Recover-tier core entries. `RootSummary.needs_you` is not shown. Its broader meaning would mislabel supervisor-owned steps as user decisions.
- **D5. Unified graph model.** New pure module `topology.ts` (S2) builds nodes for:
  - the supervisor (an open root run of kind `supervisor` or `adopted`),
  - every open task (lane ≠ `accepted`),
  - every open run in the root,
  - every subagent when "Subagents" is on.

  Node IDs are collision-safe and namespaced: `run:`, `task:`, `sub:<run>:<sub>`, each component URI-encoded. Duplicate DTO IDs are deduped (first in sorted order wins). There are no fake assignments and no inferred edges:

  | Case | Placement |
  |---|---|
  | Task whose `current_run_id` is an open run in the root | Assigned |
  | Any other open task | Dotted "unassigned"; provenance "not assigned", or "worker closed" when the current run is closed |
  | Worker that is the current run of an open task and whose `parent_run_id` is the root, missing or closed | Child of the task node (solid "assigned" edge) |
  | Worker whose parent is another open worker | Child of that worker ("delegated" edge). If it is also the current run of an open task, the task keeps its Task-column slot under the supervisor and a non-tree **assigned link** is drawn. |
  | Worker without a matched open task | Child of its parent run; with a missing or closed parent it becomes a forest root, without inventing an edge |
  | Subagent | Child of its parent subagent if present in the same run, otherwise of its run (dashed) |
  | Closed root | Has no node (existing filter `:104`); its tasks and descendants become roots in their columns |

  Column floors: supervisor 0, task 1, worker 2, subagent 3. A node's column is `max(parentColumn+1, floor)`.

  Sibling order:
  - Supervisor children: assigned tasks by `task.line`, then taskless direct workers by `(created_at, run_id)`, then root subagents by id, then unassigned tasks by line. The first unassigned task gets `gapBefore 0.4`.
  - Worker children: subagents by id, then child runs by `(created_at, run_id)`.

  Reading order is the layout preorder: supervisor, then each task → worker → subagents, as DESIGN:240 requires.
- **D6. Layout reuse.** `graphLayout.ts` keeps its iterative traversal, missing-parent rule and cycle breaking by lexically smallest ID. It is extended with:
  - an optional `minColumn`, `order` tuple and `gapBefore` per node;
  - accepted geometry;
  - row-mean placement;
  - effective `parentOf`, preorder `order` and column count in the result.

  Nodes in the same column are always in disjoint subtrees (columns strictly increase along every parent edge). Their rows therefore differ by at least 1, which gives a pitch of at least 55 px after rounding, more than 48, so nodes never overlap. *Rejected:* a second layout engine, or a separate task layer joined by cross edges. The forest and its cycle guarantees are reused.
- **D7. One surface mounted.** Only the active view (Tasks or Graph) is rendered. Scroll offsets are saved on switch or unmount and restored on mount, explicitly (O13). This keeps the DOM free of hidden duplicate roving stops. *Rejected:* both mounted with `hidden`, which leaves two task representations, two roving lists, and still requires explicit offsets.
- **D8. Layout decided in JS, mirrored to CSS.** `useSupervisorLayout` (S3) is the single source for `narrow` (workarea width < 720), `short` (`window.innerHeight` ≤ 600), `compact` (< 480), queue mode and panel placement. It follows the existing ResizeObserver plus window-resize pattern (`App.tsx:550-554`, `ServerPopup.tsx:51-55`). `.supervisor-view` carries `data-view`, `data-narrow`, `data-short`, `data-panel` and `data-queue`, and structural CSS keys off these. An unmeasured width of 0 is treated as wide, matching E12. jsdom tests stay wide unless a test stubs the size. *Rejected:* container and media queries alone. JS needs the same mode for keyboard semantics (stacked lanes), for focus-to-Close, and for sheet bounds.
- **D9. Panel placement and bounds.**

  | Panel | Placement |
  |---|---|
  | `details` | `side` if not narrow; `sheet` if narrow, Graph view and workarea height ≥ 560; otherwise `overlay` |
  | `activity`, `diagnostics` | `side` if not narrow, else `overlay` |
  | `attention` | `overlay` only (queue overlay mode) |

  Bounds: side {min 280, max ⌊0.5·W⌋, default 340}; sheet {min 160, max ⌊0.75·H⌋, default ⌊0.5·H⌉}. Values persist in `ScopeDrafts.view.detailWidth` and `sheetHeight` (Q7 default: lifetime of the mounted workarea). The sheet is non-modal. The canvas gets a bottom spacer equal to the sheet height.
- **D10. Controlled `PanelSplitter`.** New component (S3) with orientation, `value/min/max`, `aria-valuenow/min/max/valuetext="N px"`, `aria-label="Resize details"`, `aria-controls`, pointer capture drag, Arrow ±16 (Shift ±48), Home and double-click reset. For the vertical side panel, ArrowLeft grows it; for the sheet, ArrowUp grows it (PRES:1401-1425; BOUNDED:28). `RowSplitter` is deleted with its only consumer (clean cutover). *Rejected:* adapting `RowSplitter`'s measure-the-target model. ARIA needs a controlled value.
- **D11. Reveal is explicit.** The pure `reveal.ts` (S3) computes the nearest instant scroll within **one** scroller, honouring insets for the sticky column header and the sheet. It sets only that scroller's `scrollLeft/Top` and then focuses with `preventScroll:true`. Reveal runs only for outside origins: view switch, queue Show in…, path row, strip chip, details Show in…, Subagents-off reparenting, and resize-covered nodes. A plain click on a visible node never scrolls (DESIGN:183,429-447). *Rejected:* `Element.scrollIntoView`, which also scrolls `overflow:hidden` ancestors such as `.supervisor-view` (`supervisor.css:1`).
- **D12. Presentational components own view-models.** S5, S6, S9, S10 and S11 take plain props defined in their own files. The View integration (S8) maps the attention, topology and drafts models onto them. This keeps the slices file-exclusive and lets them run in parallel.
- **D13. Disabled reasons.** Mutation buttons keep native `disabled` and gain a visible muted reason referenced by `aria-describedby` (F13 option "muted reason line"). The only exception is **Restart agent…** in recovery (F5), which uses `aria-disabled` plus an `aria-describedby` reason. Its click handler re-checks `canRestart(state, flags)` (restartable, connected, runtime live, not busy, has dispatch, not reported) before calling `onRestart`. This keeps native guard semantics everywhere else.
- **D14. Dimming.** Text stays at full opacity. The dimmed item lowers background and border emphasis and appends ", dimmed by attention filter" or ", dimmed by Space filter" to its accessible name (DESIGN:410,460).
- **D15. Copy choices** (exact strings are not given in DESIGN):

  | Surface | Copy |
  |---|---|
  | Empty root board heading | "No tasks yet", with the existing Open terminal action (F11, PRES:1069) |
  | Offline banner | "Connection lost · showing saved tasks · agent may still be running" (DESIGN:377) |
  | Observation unavailable banner | "Cannot check agents right now · showing saved tasks" (`agentState` wording, `SupervisorActions.tsx:16`) |
  | Plan-override dismiss | "Keep supervisor decision" (F8 "Keep …" convention) |
  | Recover-setup primary | Use existing worktree / Retry setup / Check setup (DESIGN:334) |
  | Column headers | Supervisor · Tasks · Workers · Subagents · Nested |
- **D16. Single-row expansion and focus.** The queue keeps exactly one expanded row in `ScopeDrafts.view.queueOpenRow`. The first Decide row expands by default when none is chosen. Expanding never moves DOM focus.
- **D17. Evidence location.** Implementation evidence goes to `planning/supervisor-implementation-2026-10-07/verification/` only. BOUNDED and all accepted files stay unchanged. This overrides the verification scout's suggestion to overwrite the bounded report.

### 3.3 Rejected alternatives (summary)

- Reimplementing core attention ages and answer matching in TS. Rejected by the parent's constraints.
- A separate DTO-only fallback for empty `attention` arrays.
- Keeping the graph-height band or splitter. F3 removes it.
- Side-by-side Board and Graph at ≥1600 px. Q6 default: no.
- Zoom, minimap or collapse. Q12 default: none.
- An indented outline at <480 px. Q10 default: canvas.
- A seventh "Attention" lane. DESIGN §7.
- Persisting the view choice or splitter sizes. Q7 default: per mounted workarea.

### 3.4 Authority guards that must not regress (every slice)

| ID | Guard | Current source |
|---|---|---|
| G1 | Answer = `message_send kind:"answer"` to the root only. Disabled when `busy \|\| !live \|\| !rootState.verified`. `TextAction` keeps text and operation ID on unconfirmed delivery and retries the same ID. | `SupervisorView.tsx:290`; `SupervisorActions.tsx:59-89` |
| G2 | Assignment resolve sends `expected_task_revision` = current canonical revision. Assign is disabled on `busy\|\|!live\|\|!canonical\|\|diagnostic\|\|root closed`; Keep unassigned on `busy\|\|!connected`. | `:296` |
| G3 | Acceptance-intent resolve uses the exact `intent_id` from `snapshot.intents`. Apply is disabled on `busy\|\|!live`; Keep on `busy\|\|!connected`. | `:298` |
| G4 | `tasks_assign_ids` sends `expected_doc_revision`. | `:299,334` |
| G5 | Accept: as in E14. | `SupervisorActions.tsx:184` |
| G6 | Send back and Durable note are non-retryable and locked after an unknown outcome. | `:183-184` |
| G7 | Plan override: exact `plan_revision`, reviewed-stale check, arm then confirm, Escape disarms, "Same-user policy, not an OS sandbox". | `:107-115,186` |
| G8 | Restart: setup_unknown → recovery dialog; plan_failed → reconcile; launch_unknown or needs_review → retry dialog; otherwise reconcile first, then `pendingRestart` verification. No automatic retry. Disabled when not restartable. | `SupervisorView.tsx:166-184`; `SupervisorActions.tsx:12,102` |
| G9 | Close tracking = `cancel_run` behind a dialog with no-kill wording. | `SupervisorDialogs.tsx:97-98` |
| G10 | Subagent Send is non-retryable. Cancel goes through a dialog. Controls require a non-closed run, `live`, and `status==="running"`. | `SupervisorActions.tsx:146,179-180` |
| G11 | Request stop requires a verified active supervisor and an unchecked task, and retries with the same message ID. | `:123-138,148` |
| G12 | Edit sends `expected_task_revision=draft.revision`. The draft is kept on conflict. **New:** Save disabled for a deleted task. | `SupervisorDialogs.tsx:56-59,94` |
| G13 | Start: `startLock`, `startUnknown` blocks; Existing Space must be available; Directory must be absolute; unconfirmed lock. | `SupervisorView.tsx:137-150`; `SupervisorDialogs.tsx:46-55` |
| G14 | Herdr focus changes only through `onTerminal`, gated by `live && !busy`. | `SupervisorView.tsx:157-161` |
| G15 | Snapshot scope and revision fencing. | `useSupervisor.ts:12-15,41-73` |
| G16 | Reported ≠ observed. A closed run shows no observed evidence. "unobserved" wording. Unavailable ≠ missing. | `SupervisorActions.tsx:48-58` |
| G17 | The Board is a projection of the canonical Markdown. The graph never fabricates assignment or parentage. | `DECISIONS.md:55` |

Locks that must survive: `TextAction` operation retention, the `RequestStop` operation, the dialog `unconfirmed` lock (non-edit modes), `startUnknown` plus "I have reviewed the previous start", and `pendingRestart`.

## 4. Open questions (material only; the plan proceeds with the recommendation)

| # | Question | Options | Recommendation |
|---|---|---|---|
| **Q-A** | **Conflict with accepted F9 wording.** DESIGN:348 says the selector shows "`· 2 need you` using `RootSummary.needs_you`", but core counts all attention, including supervisor-owned `awaits_*`, `to_accept` and worker questions (`projection.rs:176-197`). | (1) Show `RootSummary.needs_you` as "need you" (misleading). (2) Show it relabelled "N attention" (accurate but loses the user-decision meaning). (3) Count Decide entries for that root from `snapshot.attention`, which covers all roots (`projection.rs:45-51,79-89`), label "N need you", and add "⚠ recover" for Recover-tier core entries. | **(3)**: keeps the accepted user-facing meaning with an accurate count and needs no backend change. Parent may choose (2) if the accepted data source must be literal. |
| **Q-B** | Closed-tracking rows (F10/D20) require task counts; the current board covers only the selected root. | Bounded read-only snapshots for closed roots on archive-open, with current generation/session/root identity fences and explicit loading/unavailable states. | **Parent approved:** deliver all task counts through the existing API; label `updated_at` as updated, not closed. No omission or fabricated timestamp. |

The Q1–Q12 defaults from DESIGN:483-494 are adopted as written.

## 5. Shared interface contracts (fixed before any slice starts)

Full sketches are in [`examples/contracts.md`](examples/contracts.md). The signatures below are binding. Names must match exactly so the slices compose without design decisions.

**C1 `attention.ts` (S1):**

- `AttentionTier = "decide"|"recover"|"notice"`; `TIER_LABEL`; `TIER_ORDER`.
- `LocalCondition`: a union of the local kinds in D2.
- `AttentionSource = {origin:"core", kind:AttentionKind, messageSeq} | {origin:"local", condition:LocalCondition}`.
- `AttentionItem {id, tier, subject:{kind:"run",runId}|{kind:"task",taskId}|{kind:"workarea"}, runId, taskId, sources (non-empty, precedence order), since}`.
- `SupervisorOwned {kind:"awaits_prepare"|"awaits_execute"|"to_accept"|"needs_input", runId, taskId, since}`.
- `AttentionModel {items, counts, total, supervisorOwned, tierForRun(id), tierForTask(task), itemsForRun(id), itemsForTask(task), ownedForRun(id)}`.
- `deriveAttention({snapshot, rootId, local}): AttentionModel`.
- `rootAttentionSummary(snapshot, rootId): {decide, recover, notice}`.

**C2 `topology.ts` + `graphLayout.ts` (S2):**

- Layout types:
  - `GRAPH_GEOMETRY = {nodeWidth:240,nodeHeight:48,columnGap:32,rowPitch:56,padding:8,headerHeight:28}`.
  - `GraphNode {id,parentId,minColumn?,order?:readonly (string|number)[],gapBefore?}`.
  - `GraphPosition {id,column,row,x,y,width,height}`.
  - `GraphLayout {positions,edges,width,height,columns,order:string[],parentOf:ReadonlyMap<string,string|null>}`.
  - `edgePath(from,to)`.
- Topology types:
  - `nodeId.{run,task,subagent}`.
  - `TopologyNode {id,kind:"supervisor"|"task"|"worker"|"subagent",parentId,edge:"task"|"assigned"|"delegated"|"subagent"|"unassigned"|null,minColumn,run,task,subagent,assignedRunId,linkedTaskId,unassigned}`.
  - `TopologyLink {from,to,kind:"assigned"}`.
- Model and helpers:
  - `SupervisorGraphModel {nodes (reading order), byId, children, links, layout, hiddenCompletedTasks, counts:{supervisors,tasks,workers,subagents}}`.
  - `buildSupervisorGraph({snapshot, root, rootId, tasks, includeSubagents})`.
  - `firstChild(model,id)`, `chainIds(model,id)`, `pathNodes(model,id,includeChildren)`.
  - `selectionNodeId({selectedTask,selectedRun,selectedSubagent})`, `nodeSelection(node)`.
  - `nodeFacts(node, {snapshot, live, connected, runtimeLive, labelFor}) → {glyph: GlyphShape|"document", title, role, status, provenance, relation}`. It uses the **frozen** `agentState` and the existing raw-status mapping (`SupervisorGraph.tsx:100-112`).

**C3 `SupervisorGraph.tsx` props (S6):** `{model, snapshot, live, connected, runtimeLive, selectedNodeId, highlightedRunId, tierFor(node), dimFor(node): string|null (dim reason), showSubagents, onShowSubagents(next), sharedSpace, bottomInset, scrollRef, onSelect(node), onHover(runId|null), onEscape(): boolean}`.

**C4 `PanelSplitter.tsx` (S3):** `{label, controls, orientation:"vertical"|"horizontal", value, min, max, grow:1|-1, onChange(px), onReset(), className?}`.

**C5 `useSupervisorLayout.ts` (S3):**

- `SupervisorLayout {measured,width,height,narrow,short,compact,queueMode:"inline"|"overlay"}`.
- `useSupervisorLayout(ref)`.
- `PanelKind = "details"|"activity"|"diagnostics"|"attention"`.
- `panelPlacement(layout, view, panel) → "side"|"sheet"|"overlay"`.
- `panelBounds(layout, placement, saved) → {orientation,min,max,value,defaultValue}|null`.
- `queueCap(layout, view)`: 40% of height in Tasks, 30% in Graph (DESIGN:129,184).

**C6 `reveal.ts` (S3):** `Insets`, `ScrollOffset`, `nearestScroll(viewport, target, insets)` (pure), `revealNearest(scroller, element, insets?)`, `isCovered(scroller, element, insets)`, `readOffset(el)`, `writeOffset(el, offset)`.

**C7 `boardNavigation.ts` (S4):** `BoardArrangement = "lanes"|"stacked"`; `BoardNavOptions {arrangement, completedOpen, collapsedLanes}`; `taskNeighbor(tasks, id, key, options)`; `visibleTaskIds(tasks, options)`.

**C8 `SupervisorActions.tsx` (S5), additive props:**

- `SupervisorActions` gains:
  - `stateBlock: {sentence: string; tierLabel: "Decide"|"Recover"|"Notice"|null; waitingSince: string|null}`
  - `path: readonly PathRowView[]`
  - `crossView: {label:"Show in Graph"|"Show in Tasks"; onActivate(): void}|null`
  - `acceptanceConflict: boolean`
- New exports:
  - `PathRowView {key, role, label, facts:string[], current, depth, subagent:boolean, onActivate()}`
  - `observedStatus(run, observed, snapshot, live) → {word, glyph}`, extracted from `ObservedEvidence:55-56`
  - `reportAge(report)`, extracted from `ReportedEvidence:49-50`
  - `ProvenancePair`
  - `ProgressTrail`
  - `canRestart(state, {busy,connected,runtimeLive}, run)`
  - `recoveryActions(run, state, {busy,connected,runtimeLive}) → RecoveryAction[]`, where `RecoveryAction = {kind:"terminal"|"check"|"setup"|"retry_setup"|"restart"|"close", label, primary, consequence?, disabled, ariaDisabled?, reason?}`. This is the single ordered source for both `AgentRecovery` and the View queue rows, so the F5 ordering is not duplicated.
- `AgentRecovery` keeps its props. `agentState` and `taskStatus` are frozen.

**C9 `SupervisorDialogs.tsx` (S7):** an added prop `onCheck(run: Run): void`, used by "Check again first". No payload changes.

**C10 `SupervisorAttention.tsx` (S9):**

- `QueueAction {key,label,onActivate(invoker),disabled?,reason?,primary?,consequence?}`.
- `QueueRowView {id,tier,title,since:string|null,body?:ReactNode,actions:QueueAction[],showIn:QueueAction|null}`.
- `SupervisorSummary {rootLabel,stateLabel,stateGlyph,observedLine,banner,counts,queueMode,onCounter(tier, invoker),shortcut:QueueAction|null}`.
- `AttentionQueue {rows,expandedId,onExpand(id|null),capPx:number|null,variant:"inline"|"overlay",focusTier:AttentionTier|null,onFocusedTier():void}`.

**C11 `SupervisorActivity.tsx` (S10):**

- `ActivityRow {id, day, actor:"You"|"Supervisor"|"Dispatcher"|string, what, at, stale, link:{kind:"task",taskId}|{kind:"run",runId}|null, detail}`.
- `activityRows(snapshot, rootRuns, tasks)`, a pure function moved from `SupervisorView.tsx:204-219` with its redaction rules.
- `SupervisorActivity {rows, onLink(link)}`.
- `SupervisorDiagnostics {snapshot, rootRuns, busy, connected, onIdentify(), onCopyPath()}`.
- `ClosedTracking {closedRoots, runs, loadedRootId, taskCount, busy, dialogOpen, onView(rootId), returnTo:{label,onActivate}|null}`.

**C12 `SupervisorTasks.tsx` (S11):**

- `TaskCardView {task, worker, observed, snapshot, live, status, tier, dimReason, selected, linked, subagentCount, showSpace, sharedSpace}`.
- `TaskCard {view, rowId, tabIndex, onFocus(), onSelect(), onHover(entering)}`.
- `StripChip {runId,label,glyph,status,tier,selected}`.
- `AgentsStrip {heading, chips, subagentCount, onSelect(runId, invoker), onGraph()}`.

**C13 `ScopeDrafts` (S8):**

```ts
view: {
  mode: "tasks" | "graph";                            // default "tasks"
  offsets: { tasks: ScrollOffset | null; graph: ScrollOffset | null };
  laneScroll: Partial<Record<TaskLane, number>>;      // wide per-lane list scrollTop
  collapsedLanes: TaskLane[];                         // narrow lane groups the user closed; Done uses disclosures.completed
  detailWidth: number | null;
  sheetHeight: number | null;
  queueOpenRow: string | null;
}
```

`disclosures.agents` is removed.

**Stable DOM hooks (CSS and test contract):**

- Card buttons keep `data-row-id=<task_id>`. Graph nodes use `data-row-id=<nodeId>`.
- Regions: `aria-label` `Supervisor status` (summary), `Attention` (queue), `Workarea view` (switch group), `Agents` (strip), `Agent graph, scrollable` (scroller), `Agent relationships` (canvas), `Selected details`, `Activity`, `Diagnostics`, `Closed tracking`, `Tasks`.
- The Decide body keeps `aria-label="Needs you"` (E17).
- Class prefixes: `supervisor-summary*`, `supervisor-queue*`, `supervisor-viewbar*`, `supervisor-strip*`, `supervisor-graph-*`, `supervisor-sheet*`, `supervisor-panel-splitter`, `supervisor-lane-group*`, `supervisor-card*`, `supervisor-pair*`, `supervisor-state-block`, `supervisor-trail*`, `supervisor-operator*`, `supervisor-path*`, `supervisor-activity*`, `supervisor-diag*`.

## 6. Slices

### 6.1 Ownership and order

| Wave | Slice | Owner | Exclusive files (create/modify) | Depends on |
|---|---|---|---|---|
| 1 | **S1** Attention model | worker | `attention.ts`, `attention.test.ts` (new) | — |
| 1 | **S2** Topology and layout | worker | `graphLayout.ts`, `graphLayout.test.ts`, `topology.ts`, `topology.test.ts` (new) | — (imports frozen `agentState`, `taskLanes`) |
| 1 | **S3** Splitter, sizing, reveal | worker | `PanelSplitter.tsx`, `PanelSplitter.test.tsx`, `useSupervisorLayout.ts`, `useSupervisorLayout.test.ts`, `reveal.ts`, `reveal.test.ts` (all new) | — |
| 1 | **S4** Board navigation | worker | `boardNavigation.ts`, `boardNavigation.test.ts` (new) | — |
| 1 | **S5** Details, recovery, evidence | worker | `SupervisorActions.tsx`, `SupervisorActions.test.tsx` (new) | — |
| 1 | **S7** Dialogs | worker | `SupervisorDialogs.tsx`, `SupervisorDialogs.test.tsx` (new) | — |
| 1 | **S10** Activity, Diagnostics, archive | worker | `SupervisorActivity.tsx`, `SupervisorActivity.test.tsx` (new) | — |
| 2 | **S6** Graph renderer | worker | `SupervisorGraph.tsx`, `SupervisorGraph.test.tsx` (new) | S2, S1 (types), S3 (`reveal`) |
| 2 | **S9** Summary and queue | worker | `SupervisorAttention.tsx`, `SupervisorAttention.test.tsx` (new) | S1 (types) |
| 2 | **S11** Tasks presentation | worker | `SupervisorTasks.tsx`, `SupervisorTasks.test.tsx` (new) | S1 (types), S5 (`ProvenancePair`, `observedStatus`, `reportAge`) |
| 3 | **S8** Integration | **parent** | `SupervisorView.tsx`, `useSupervisorDrafts.ts`, `supervisor.css`, `SupervisorView.test.tsx`; delete `RowSplitter.tsx` | all of the above |
| 4 | **S12** Docs migration | parent or worker | `CONTEXT.md` §5.7, `DECISIONS.md:54`, `CODE_GUIDE.md:23`, `docs/keyboard-shortcuts.md:9`, `docs/supervisor-surfaces.md` | S8 |
| 5 | **S13** Verification | **parent** | `planning/supervisor-implementation-2026-10-07/verification/*` (new evidence only) | S8, S12 |

All paths are under `src/app/supervisor/` unless shown otherwise. All CSS goes into `supervisor.css` and is written only by S8. Workers use the class names fixed in §5 and do not edit CSS. The type-check break between waves is expected: View still calls the old `taskNeighbor` and `SupervisorGraph` props until S8. Per the parent's rules, no slice runs builds, tests or browsers. The parent verifies once, in S13.

Slice instructions shared by every worker:

- Reuse existing patterns: `useRovingList`, `StateGlyph`, `UiIcon`, `TextAction`, `ErrorSlot`, `copyText`.
- Do not modify files you do not own, accepted planning artifacts, Notes, or widget code (concurrent task `459acf87` owns widget files).
- Name the checks the parent should run.

### S1 — Attention model (F1, F2, F9 selector, F13 queue counts)

- **Goal:** one pure attention model that feeds the queue, badges, filter, counters, root selector and details state block.
- **Files:** `attention.ts`, `attention.test.ts`.
- **Contract:** C1.
- **Steps:**
  1. Implement `coreTier(kind, run)` per D2. `needs_input` returns Decide only for `run.run_id === run.root_id`; other runs return supervisor-owned.
  2. In `deriveAttention`, keep only core entries whose run's `root_id === rootId`, ignoring entries whose run is missing from `snapshot.runs`. Map each to a tier or to `SupervisorOwned`. Append local conditions. Drop local `agent_status` when a core Recover entry exists for the same run. Dedupe and order per D3. Build `byRun`/`byTask` indexes. `tierForTask(task)` is the minimum tier over items with `taskId === task.task.task_id` or `runId === task.current_run_id`.
  3. `rootAttentionSummary(snapshot, rootId)` counts core-only tiers for that root (for the selector, Q-A option 3).
  4. Tests:
     - every core kind maps to the right tier;
     - root vs worker `needs_input`;
     - `awaits_*` and `to_accept` are never items;
     - local plus core dedupe for one worker (exited, dispatch_unknown and agent_status give one Recover row whose primary source is `exited_without_report`);
     - ordering, including null `since` last;
     - ID encoding with `:` and `%` in IDs;
     - other-root entries excluded;
     - an empty `attention` array plus a local condition yields only the local row (no core fallback);
     - `rootAttentionSummary` for two roots.
- **Non-goals:** rendering, actions, recomputing thresholds or answer matching, offline banner logic.
- **Acceptance:** `bunx vitest run src/app/supervisor/attention.test.ts` passes (run by the parent in S13).

### S2 — Topology and layout (F3 graph model, F7 chain, scenario 14, 21 geometry)

- **Goal:** a unified, deterministic, collision-safe graph model with accepted geometry.
- **Files:** `graphLayout.ts`, `graphLayout.test.ts`, `topology.ts`, `topology.test.ts`.
- **Contracts:** C2. Geometry per §3.1.
- **Steps:**
  1. `graphLayout`: replace the constants with `GRAPH_GEOMETRY`. Sort siblings and roots by `(order tuple, id)`, falling back to id. Keep cycle breaking at the lexically smallest ID, independent of `order`. Compute `column = max(parentColumn+1, minColumn??0)` for children and `minColumn??0` for roots. Assign leaf rows in preorder: `next += gapBefore` when visiting a node with `gapBefore > 0` and `next > 0`. Internal row = (first child row + last child row) / 2. `x = 8 + 272·column`; `y = round(8 + 56·row)`. Return `order` (preorder IDs), `parentOf` (effective, after cycle and missing-parent pruning) and `columns`. Export `edgePath`: `M sx sy L ex ey` when level, otherwise `M sx sy C mx sy, mx ey, ex ey`. Keep iteration only, with no recursion.
  2. Update `graphLayout.test.ts`: recompute exact coordinates (the root/worker-a/worker-b case becomes root (8,36), workers (280,8) and (280,64), width 528, height 120). Keep the overlap, orphan, self-parent, cycle-determinism, frozen-input and 2,000-deep tests. Add `minColumn` skip-column, `order`, `gapBefore` 0.4, and a reproduction of the DESIGN static figure: the 18 nodes from DESIGN:210-221 produce the exact x/y in that table and canvas 1344×478.
  3. `topology.ts`:
     - `buildSupervisorGraph` implements D5 and calls `graphLayout(graphNodes)`.
     - `firstChild` uses the ordered `children` derived from the effective `parentOf`.
     - `chainIds(model, id)` returns the ancestors via `parentOf`, iteratively with a visited set, plus the linked task (worker) or the assigned worker (task), plus link endpoints.
     - `pathNodes(model, id, includeChildren)`: the chain without the supervisor, the linked task prepended for nested workers, then direct children when requested.
     - `nodeFacts` per DESIGN:196-202:
       - supervisor and worker: glyph and status from `agentState`/observation as in the current graph; provenance `Herdr · <Space>` or `Herdr`.
       - task: `"document"` glyph, the lane label from `taskLanes` as status, provenance assigned worker label / `not assigned` / `worker closed`.
       - subagent: glyph running→working, done→done, failed→blocked, otherwise unknown; provenance `OMP events · no terminal`.
       - `relation` text: "task of …" / "worker for …" / "subagent of …".
     - `counts` (supervisors, workers, subagents, tasks) and `hiddenCompletedTasks = tasks.filter(lane==="accepted").length`.
  4. `topology.test.ts`:
     - an unassigned open task becomes a dotted node at the bottom with the gap;
     - a worker without a task attaches to the supervisor in column 2, with no fake task;
     - a worker whose task is accepted is not drawn under a hidden task;
     - a nested worker gets a delegated edge, plus an assigned link when it holds a task;
     - nested subagents;
     - a subagent with a missing parent subagent attaches to its run;
     - a cyclic `parent_run_id` keeps all nodes, with an edge removed only per layout;
     - a run with a missing parent is a root with no edge;
     - a closed root yields root-less tasks and descendants;
     - Subagents off removes subagent nodes and counts;
     - duplicate IDs deduped;
     - ID encoding with colons;
     - reading order equals DESIGN:240 order;
     - `chainIds` and `pathNodes` for task, worker, nested worker and subagent;
     - `nodeFacts` unobserved vs fresh;
     - 60-node forest determinism (input order shuffled gives identical output).
- **Non-goals:** DOM, CSS, attention, selection state.
- **Acceptance:** `bunx vitest run src/app/supervisor/graphLayout.test.ts src/app/supervisor/topology.test.ts`.

### S3 — Splitter, layout sizing, reveal (F3 details, F12, F13 splitter, scenarios 15, 18, 20)

- **Files:** `PanelSplitter.tsx`, `PanelSplitter.test.tsx`, `useSupervisorLayout.ts`, `useSupervisorLayout.test.ts`, `reveal.ts`, `reveal.test.ts`.
- **Contracts:** C4, C5, C6. D8–D11.
- **Steps:**
  1. `PanelSplitter`: `role="separator"`, `aria-orientation`, `aria-label`, `aria-controls`, `aria-valuemin/max/now`, `aria-valuetext="${value} px"`, `tabIndex=0`, and `title="Drag to resize · double-click to reset"` (from `RowSplitter.tsx:49`).
     - Pointer: on pointerdown (button 0), call `setPointerCapture`, record the start client coordinate and value, and add `document.body.classList.add("is-resizing-panes")` (existing global, `RowSplitter.tsx:26,36`). On move, `onChange(clamp(start + delta·grow))` with delta on X for vertical and Y for horizontal. Release on up, cancel or lost capture.
     - Keys: vertical uses ArrowLeft/Right, horizontal uses ArrowUp/Down, ±16 (Shift ±48) signed by `grow` on that axis; Home calls `onReset`. Double-click calls `onReset`. Clamp to `[min, max]`.
  2. `useSupervisorLayout(ref)`:
     - Measure `ref.current.getBoundingClientRect()` with a ResizeObserver (guarded by `typeof ResizeObserver`) plus a window `resize` listener.
     - `measured = width > 0`; `narrow = measured && width < 720`; `short = window.innerHeight <= 600`; `compact = measured && width < 480`; `queueMode = narrow || short ? "overlay" : "inline"`.
     - Implement `panelPlacement`, `panelBounds` and `queueCap` as pure functions per D9.
  3. `reveal.ts`: `nearestScroll` computes the minimal delta that brings the target rect inside the viewport rect shrunk by insets (target taller than the viewport aligns its start). `revealNearest` reads rects relative to the scroller and writes only `scroller.scrollLeft/Top`. `isCovered` reports whether the target intersects the inset band (for the resize re-reveal). Offsets are read and written as `{left, top}`.
  4. Tests:
     - splitter ARIA attributes;
     - each key and direction for both orientations, with clamps;
     - Home and double-click reset;
     - pointer drag with stubbed `setPointerCapture` (pattern from `TerminalPane.test.tsx:833-840`);
     - layout thresholds at 719/720 width, 600/601 window height and 559/560 workarea height, plus unmeasured = wide;
     - bounds clamping (side max = ⌊0.5·W⌋, sheet default ⌊0.5·H⌉, max ⌊0.75·H⌋);
     - `nearestScroll` cases: already visible means no change, above, below, left, right, the sheet inset and the sticky header inset.
- **Non-goals:** wiring into the View, CSS.
- **Acceptance:** the three test files pass.

### S4 — Board navigation (F12 lane groups, scenario 9, 11)

- **Files:** `boardNavigation.ts`, `boardNavigation.test.ts`.
- **Contract:** C7.
- **Steps:**
  1. `visibleTaskIds(tasks, options)` returns IDs in `taskLanes` order, skipping Done unless `completedOpen` and, in `stacked`, skipping lanes in `collapsedLanes`. It replaces `SupervisorView.tsx:106`.
  2. `taskNeighbor` in `lanes` keeps exactly the existing semantics (`boardNavigation.ts:10-25`; Done gated by `completedOpen`). In `stacked`: ArrowUp/Down move along the flat visible list (clamped), Home/End go to the first/last flat item (PRES:1584-1587), Left/Right return `undefined`. The docblock "focus navigation only" stays.
  3. Tests: the existing lanes behaviour (port the cases from `SupervisorView.test.tsx:435-465` at unit level); stacked crossing lane boundaries; collapsed lanes skipped; Done closed skipped.
- **Non-goals:** markup and CSS.
- **Acceptance:** `boardNavigation.test.ts` passes.

### S5 — Details, recovery, evidence (F4, F5, F6 pair, F7 path rendering, F11 evidence, F13 reasons)

- **Files:** `SupervisorActions.tsx`, `SupervisorActions.test.tsx`.
- **Contract:** C8. **Frozen:** `agentState`, `taskStatus`, `Mutation`, `TextAction` semantics.
- **Steps:**
  1. Extract `observedStatus` and `reportAge` from `ObservedEvidence:55-56` and `ReportedEvidence:49-50` without changing semantics. Remove the per-card "Saved reports only" string (`:57`) and render "unobserved" with a hollow glyph instead (F11). Add `ProvenancePair({run, report, observed, snapshot, live, compact})`: chips `reported <age>` (muted, absolute time in `title` and accessible name) and `observed <glyph> <word>`. Return null for closed runs (G16).
  2. Add `ProgressTrail({run, task, snapshot})`, read-only:
     - Assigned (task has `current_run_id === run`);
     - Prepared (a `prepare` grant: "by supervisor" if `origin==="supervisor"`, else "by you (operator)");
     - Executing (an `execute` grant, work plan or init receipt);
     - Result reported (`run.result`, with outcome);
     - Accepted (`close_reason==="accepted"` and the task is checked and in the accepted lane).
  3. Overview order: State block (`stateBlock.sentence`, tier label chip, "since" age from `waitingSince`, `ProvenancePair` with full evidence lines), then the **Result** (moved from Activity `:171`: outcome, exact summary, reporter, time, "Awaiting review"/"Accepted"), then `ProgressTrail`, then Relationship path (`path` rows rendered as buttons, indented by `depth`, `aria-current` on the current row, dashed for subagents; row activation calls `onActivate` and nothing else), then the cross-view button (`crossView`), then the task description. The subagent overview states "In <worker> · no terminal of its own" once at the top (DESIGN:260).
  4. Activity segment: work plan, init receipt, subagent control receipts (unchanged semantics). Result no longer duplicated.
  5. Actions:
     - **Routine** group: Open terminal (fresh pane only), Follow-up (single label, no duplicate `h3`), Edit task…, Request stop… (inline confirm kept), Durable note (single label).
     - **Operator intervention** `<details>` labelled "Operator intervention — normally handled by the supervisor": Accept explicit result plus Send back (G5, G6 unchanged), Operator plan override (G7; dismiss label "Keep supervisor decision"), Close agent tracking….
     - The `open` state is local, forced open while plan override is armed (lift `armed` via an `onArmedChange` prop from `PlanOverride` into `SupervisorActions` state) or when `acceptanceConflict` is true.
     - Disabled reasons (D13): muted reason line plus `aria-describedby` for Accept (diagnostic / not current run / not live), Edit (not live / diagnostic), Follow-up (not live / closed), Request stop (supervisor not verified).
  6. `AgentRecovery` (F5):
     - State line plus `state.detail` (existing).
     - Primary by state, first in tab order: Check status / Check connection for missing, unknown, endpoint-changed or offline; Recover setup… for `setup_unknown`; Retry setup for `plan_failed`.
     - Restart agent… is secondary, with the muted consequence "May open another terminal", and uses `aria-disabled` plus `canRestart` (D13).
     - `AgentRecovery` renders exactly `recoveryActions(...)` in order. Callbacks are mapped by `kind` to the existing `onTerminal`, `onCheck`, `onRestart` and `onCloseTracking`.
     - Open terminal only with a fresh pane.
     - Close tracking… appears only when `state.kind !== "ready"` and is text-weight (`supervisor-quiet`). The healthy row shows Open terminal only (scenario 7).
     - Keep the reported/restartable explanations (`:104`).
  7. Tests (mount `SupervisorActions` and `AgentRecovery` directly with DTOs):
     - Overview order and Result present for a reported run;
     - Operator collapsed, with Accept disabled under a diagnostic, under `current_run_id` mismatch and when `!live`;
     - Accept payload carries the exact `task_revision`;
     - plan override only at `awaiting_prepare`/`ready`, and auto-open when armed;
     - progress trail labels supervisor vs operator grant origin;
     - subagent overview says "no terminal";
     - path row activation calls only `onActivate`;
     - recovery primary per state;
     - clicking an aria-disabled Restart calls nothing;
     - healthy row has no Close tracking;
     - `ProvenancePair` unobserved has no "Saved reports only";
     - closed run has no observed evidence.
- **Non-goals:** attention derivation, topology, CSS, View wiring.
- **Acceptance:** `SupervisorActions.test.tsx` passes; existing `agentState`/`taskStatus` assertions (`SupervisorView.test.tsx:67-125`) are unaffected.

### S7 — Dialogs (F8, scenario 8)

- **Files:** `SupervisorDialogs.tsx`, `SupervisorDialogs.test.tsx`.
- **Contract:** C9. The shared portal, initial-focus-once, Escape-when-idle and Tab-wrap behaviour (`:27-41`) stays.
- **Steps:**
  1. R01 Start:
     - Field-level errors (`aria-invalid`, `aria-describedby`) for Space and Directory instead of the shared slot, with the same validation as `:47-52`.
     - With no Spaces, the Space select is disabled with the muted reason "No Space is available right now".
     - A one-line destination summary above the footer: "New tab in <Space> · OMP starts without switching terminal focus" (Existing), "Opens <path> · …" (Directory), "Dedicated agent folder · …". This replaces the trailing `:90` paragraph.
  2. R02 Edit:
     - When stale (`currentTask.task_revision !== draft.revision`), show *Your draft* and *Current task* side by side, with **Keep my draft** (the existing revision-only rebase `:94`) and **Use current** (sets the draft title, body and revision to current, client-side only).
     - When the task is deleted (`!currentTask`), show "This task no longer exists", disable Save, and offer **Copy draft** via `copyText(title + "\n\n" + body)` with a status message.
     - Keep the existing unknown-outcome copy, since `mutateResult` returns null for both rejected and unknown (O8 [INFERENCE] not resolvable).
  3. R03 Restart: a muted line "Last observed: terminal gone / start unconfirmed / endpoint changed" derived from `run.dispatch.step` and the observation, plus a secondary **Check again first** that calls `onCheck(run)` and closes the dialog (read-only reconcile, no restart). Default focus stays on Back.
  4. R04 Close tracking: up to three descendant labels, then "and N more", then "Live descendants will still need supervision."
  5. R05 Recover setup: primary label from `dispatch.recovery`: `accept_existing_worktree` → "Use existing worktree", `retry_environment` → "Retry setup", otherwise "Check setup". The title stays "Recover setup". The payload is unchanged (`:68`).
  6. R06 Cancel subagent: dismiss "Keep running", primary "Request cancellation".
  7. Error slot gets class `is-empty` when there is no error (CSS collapses it in S8). Footer markup must allow wrapping.
  8. Tests:
     - each label;
     - Save disabled for a deleted task;
     - Use current replaces the draft fields and the revision;
     - Keep my draft sets the revision only;
     - Copy draft calls `copyText`;
     - Check again first calls `onCheck` and closes without dispatching `retry_launch`;
     - Escape is ignored while in flight;
     - Tab wraps;
     - unconfirmed non-edit locks the primary;
     - payloads unchanged for all six modes.
- **Non-goals:** View wiring (S8 passes `onCheck={check}`).
- **Acceptance:** `SupervisorDialogs.test.tsx` passes.

### S10 — Activity, Diagnostics, archive (F9 naming, F10, D20)

- **Files:** `SupervisorActivity.tsx`, `SupervisorActivity.test.tsx`.
- **Contract:** C11.
- **Steps:**
  1. Move the history derivation from `SupervisorView.tsx:204-219` into `activityRows`, keeping every redaction rule (system briefs summarized; receipt annotations summarized; grants as "Preparation/Execution authorized · <supervisor label | You (origin)>"). Add `day` (local date), `actor` and a `link`: task when the message is `assign-<task>`, or the run is a worker with `task_id`; otherwise the run.
  2. `SupervisorActivity`: heading "Activity", grouped by day, rows `actor chip · what happened · link button · age`, "· stale evidence" kept, link calls `onLink`.
  3. `SupervisorDiagnostics`:
     - Banner at the top: "Launch receipts and saved reports are not current process proof."
     - Summary table: canonical path with a Copy button (`copyText`), board diagnostics, unidentified items with an Identify button (`onIdentify`, disabled `busy||!connected` per G4), and per run: dispatch step · observation (presence/actual) · binding (bound session yes/no) · last delivery stage (the latest message `stage` to that run).
     - Then the existing `RunDiagnostics` disclosures and the runtime/intents JSON (moved from `SupervisorView.tsx:334`).
  4. `ClosedTracking` (Q-B): rows `label · updated <date> · n descendants still open` (+ `n tasks` when `loadedRootId === root`), the "View … tasks and history" action, and the explanation sentence once: "Tasks and history remain available. Closing tracking did not kill agents or remove resources."
  5. Tests:
     - redaction parity with `SupervisorView.test.tsx:494-510` (no bootstrap text, plan hash, revision or session secrets);
     - supervisor grant attribution;
     - day grouping;
     - link resolution;
     - Diagnostics summary first and raw records after;
     - Identify disabled when disconnected;
     - archive rows.
- **Non-goals:** panel placement and Escape (S8).
- **Acceptance:** `SupervisorActivity.test.tsx` passes.

### S6 — Graph renderer (F3, F7, F13 graph keys, scenarios 14, 16, 19, 21)

- **Files:** `SupervisorGraph.tsx`, `SupervisorGraph.test.tsx`.
- **Contract:** C3. Uses C1 types, C2 model and `nodeFacts`, and C6 `revealNearest`.
- **Steps:**
  1. Heading (sticky): `Agents · N connected · M subagents · K tasks` (+ ` · n completed tasks hidden` when > 0), or `Agents · unobserved` when not fresh (existing freshness rule `SupervisorGraph.tsx:28-29,78`). The Subagents checkbox calls `onShowSubagents`, and its `title` hint reads "Up/Down moves through agents, Left/Right along the chain".
  2. Scroller `div` (`role="region"`, `aria-label="Agent graph, scrollable"`, `ref=scrollRef`, CSS `overflow:auto`). It contains a sticky column header row (labels by column index per D15, `left = x + 2`) and the canvas (`role="group"`, `aria-label="Agent relationships"`, size from the layout).
  3. Edge layer: a single direct-child `svg.supervisor-graph-edges` drawing tree edges (`edgePath`) plus `links`. Classes: `is-task`, `is-assigned`, `is-delegated`, `is-subagent` (dashed), `is-unassigned` (dotted), `is-highlighted` (`chainIds` of the selected node plus `highlightedRunId`).
  4. Nodes in `model.nodes` order, so DOM order equals reading order. Each is a `<button data-row-id={id}>` absolutely positioned at 240×48 with `aria-expanded={selected}`.
     - Accessible name: `title, role, status, provenance[, tier][, relation][, dim reason]`.
     - `title`: full name plus `state.detail` (supervisor/worker) or summary (subagent).
     - Grid regions `.supervisor-graph-icon` (StateGlyph or `UiIcon name="file"`), `.supervisor-graph-title`, `.supervisor-graph-tier` (`TIER_LABEL` with glyph, only when the tier is non-null), `.supervisor-graph-meta > .supervisor-graph-status + .supervisor-graph-provenance`.
     - Modifier classes `is-task`, `is-subagent`, `is-unassigned`, `is-selected`, `is-highlighted`, `is-dimmed`.
     - Click calls `onSelect(node)`. Mouse enter/leave calls `onHover(run id | null)`.
  5. Roving: `useRovingList({rowIds: model.nodes ids, selectedId: selectedNodeId, onEscape, onKey})`. In `onKey`, ArrowLeft focuses `parentOf(id)` and ArrowRight focuses `firstChild(id)`, each when present, returning true. After any keyboard focus move, call `revealNearest(scroller, el, {top: 28 + header, bottom: bottomInset})`. Up/Down/Home/End come from the hook. Enter and Space are native clicks.
  6. Bottom spacer `div` of height `bottomInset`. Empty states: "No worker agents yet" (one muted line) when there is a supervisor and no workers; tasks still render. "No agents in this task scope." when there are no nodes.
  7. No terminal callback exists in this component (structural proof for G14).
  8. Tests (mount with a model from `buildSupervisorGraph` and `examples/fixtures.md` DTOs):
     - one tab stop;
     - DOM order equals `model.nodes` order;
     - ↓/↑/Home/End move;
     - ← goes to the parent and → to the first child (a subagent leaf → stays);
     - Enter selects (click) via `onSelect`;
     - Escape calls `onEscape`;
     - every node has the five regions, with the tier region only for tiered nodes;
     - the task node has the document icon and a dotted class when unassigned;
     - the subagent has "no terminal" in its visible metadata;
     - accessible name includes tier, relation and dim reason;
     - heading counts equal node counts, and `completed tasks hidden` shows;
     - unobserved wording when not live;
     - Subagents toggle calls back;
     - spacer height follows `bottomInset`.
- **Non-goals:** selection state, drafts, reveal orchestration across views, CSS.
- **Acceptance:** `SupervisorGraph.test.tsx` passes.

### S9 — Summary bar and attention queue (F1, F13 queue, scenarios 1, 3, 16)

- **Files:** `SupervisorAttention.tsx`, `SupervisorAttention.test.tsx`.
- **Contract:** C10.
- **Steps:**
  1. `SupervisorSummary` (`aria-label="Supervisor status"`):
     - glyph, root label, state, and `observedLine`;
     - counters as text plus glyph chips (`N needs you`, `N recover`, `N notice`), only for non-zero tiers, each a button calling `onCounter(tier, el)`; the count wrapper has `aria-live="polite"` (DESIGN:415,455);
     - the `banner` (single offline/unavailable status, F11) when set;
     - the `shortcut` action (Open terminal) when all counts are 0.
  2. `AttentionQueue` (`aria-label="Attention"`):
     - one roving list over row summary buttons (`useRovingList`, ids = row ids);
     - summary button shows the tier glyph and word, the title (one line naming the task or agent), the primary action and the age (from `since`, using `reportAge` semantics). It has `aria-expanded` and toggles `onExpand`.
     - the expanded body renders `body`, all actions (primary first, `disabled` plus reason line) and `showIn`.
     - Escape on an expanded row collapses it and stops propagation. On a collapsed row, Escape is not handled, so the View layering continues.
     - Inline variant: `max-height: capPx` with internal scroll and a visible "N more" line, computed from row geometry against `scrollTop + clientHeight` on scroll and resize. Overlay variant: header with Close (focus on mount handled by the View) and no cap.
     - `focusTier` focuses the first row of that tier and then calls `onFocusedTier`.
  3. Tests:
     - counters only non-zero, with the live region on the counts only;
     - Decide/Recover/Notice ordering preserved from the input;
     - one expanded row at a time (controlled);
     - Enter/Space toggle;
     - Escape collapses, then bubbles when collapsed;
     - ↑/↓/Home/End roving;
     - "N more" appears when rows overflow a stubbed height;
     - expanding does not move focus out of a focused textarea in another row;
     - disabled action shows its reason;
     - `showIn` renders its label verbatim.
- **Non-goals:** building rows (S8), tier logic (S1).
- **Acceptance:** `SupervisorAttention.test.tsx` passes.

### S11 — Tasks presentation (F3 strip, F6 cards, F7 child chip, F13 dim)

- **Files:** `SupervisorTasks.tsx`, `SupervisorTasks.test.tsx`.
- **Contract:** C12.
- **Steps:**
  1. `TaskCard` (the card stays one roving `<button data-row-id={task_id}>` inside an `li`):
     - Line 1: title.
     - Line 2 (state line): worker glyph and label, then the status text **only when it adds to the lane** (DESIGN:293). Without a worker, only the single status (drop "No progress reported yet" when the status already says not assigned, F11).
     - Tier badge: `TIER_LABEL` plus glyph, text never color-only.
     - `+N subagents` chip when `subagentCount > 0`.
     - `ProvenancePair` (compact) when a worker exists.
     - Space chip only when `showSpace`, with the shared-Space highlight.
     - Explicit failed report text stays visible on the card (existing invariant, test `SupervisorView.test.tsx:169-178`).
     - "Runtime Done · no result reported" stays as the state line when applicable (`SupervisorView.tsx:260`).
     - Blocked warning buttons are removed from the card (they move to the queue).
     - Accessible name = title + state line + tier + dim reason. Keep `aria-expanded`/`aria-controls="supervisor-detail"`.
  2. `AgentsStrip` (`aria-label="Agents"`, one line):
     - heading (`Agents · N observed` / `Agents · unobserved`);
     - chips in the given order, each a button calling `onSelect(runId, el)`, never a terminal;
     - `+N subagents`;
     - overflow collapses to `+k more` (measured with ResizeObserver; when unmeasured, show all);
     - a `Graph ›` button calling `onGraph`.
  3. Tests:
     - status omitted when equal to the lane word;
     - unassigned single line;
     - tier badge text;
     - dim reason in the accessible name;
     - failed report visible;
     - no blocked buttons on the card;
     - strip chips select only;
     - `Graph ›` calls back;
     - `+N subagents`.
- **Non-goals:** lane layout, roving list ownership (the View passes `tabIndex`/`onFocus`), CSS.
- **Acceptance:** `SupervisorTasks.test.tsx` passes.

### S8 — Integration (parent-owned; sequential sub-steps)

- **Goal:** wire every model and component into `SupervisorView`, migrate drafts and CSS, remove obsolete code, update the View tests.
- **Files:** `SupervisorView.tsx`, `useSupervisorDrafts.ts`, `supervisor.css`, `SupervisorView.test.tsx`; delete `RowSplitter.tsx`.

**S8a Drafts and structure.**

- `ScopeDrafts.view` per C13, with defaults in `newScopeDrafts`. Remove `disclosures.agents` (`useSupervisorDrafts.ts:14,18`).
- Delete `supervisorForest`/`ForestRow` (`SupervisorView.tsx:16-38`), `narrowView` and `graphHeight` (`:65-66`), the narrow switch (`:303`), the graph band and `RowSplitter` (`:304-307`) and `RowSplitter.tsx`.
- Add `useSupervisorLayout(workareaRef)`.
- Root data attributes per D8.
- Compute `graph = useMemo(buildSupervisorGraph(...), [snapshot, root, tasks, scope.showSubagents])` and `attention = useMemo(deriveAttention(...))`.

**S8b Attention wiring.**

- Build `local: LocalCondition[]` from `startUnknown`, `terminalError`, `navigationError`, `error && connected`, `notice`, orphaned workers (`:88`, and root closed with descendants `:286`), `agent_status` (root and the `failures` rule `:192`), `root_failed_report`, assignment intents (`:202`) and unidentified items.
- Replace `hasAttention` (`:226`), the card `needsAttention` (`:249`) and the filter with `attention`.
- Build a `QueueRowView` per item:
  - **Decide**: body = the existing question section, moved verbatim with `aria-label="Needs you"`, `TextAction` and G1 disabled conditions. The row is present iff a Decide item exists for the root. "Answer sent. Waiting for the agent." shows when `root.last_report.kind==="needs_input"` and no Decide item exists. The `questionHadFocus` effect (`:196-201`) keys on Decide presence.
  - **Recover**, run subjects: map `recoveryActions(run, agentState(...), flags)` (S5) to `QueueAction`s by `kind`, wired to the existing `navigate`, `check`, `restart` and close-dialog handlers (`SupervisorView.tsx:157-191`). Primary and order come from that function. `runtime_blocked` (state is "ready" with blocked) gets Check status (primary) plus Open terminal (`!live||!terminal` disabled), matching the current blocked warning (`:259,289`). Orphans get Open terminal plus "View saved task context" (`setRootId`, `:285`). Start unknown gets Check status plus "I have reviewed the previous start" (`:283`). Errors get Check status (`:284`).
  - **Notice**: `intent_conflict` joins `snapshot.intents` by `(run_id, task_id, state:"conflict")` for `intent_id`, with Apply/Keep (G3). Assignment gets Assign current task / Keep unassigned / Check assignment status (G2). Unidentified gets Identify task-file items (G4). Idle, brief, plan-changed and root-failed get Show in… only.
  - `showIn` in the current view when a target exists (card for a task or current-run worker in Tasks, node in Graph). In Tasks with no card target, offer "Show in Graph" (switches view, focus moves to the node).
- Render `SupervisorSummary` and either the inline `AttentionQueue` (queue mode inline, cap from `queueCap`) or the overlay in the panel slot (`panel==="attention"`; focus Close on open; Escape/Close returns focus to the invoking counter).
- Filter chip `Attention · {attention.total}` as `aria-pressed`, reusing `scope.attentionOnly`.
- Root selector (F9, Q-A default) uses `rootAttentionSummary`.

**S8c Views.**

- View bar (`role="group"`, `aria-label="Workarea view"`): `Tasks {openTasks.length}` and `Graph {supervisors+workers+subagents}`, both `aria-pressed`, plus the Attention chip and the Space select (same `scope` fields).
- Tasks view: at ≥720, `AgentsStrip` (chips: Recover/Decide/Notice runs first, then reading order). Then the Board:
  - `lanes` arrangement (wide; existing horizontal lanes with the Done disclosure), or
  - `stacked` (narrow; `<details>` per lane with summary `Label N`, open when non-empty and not in `collapsedLanes`; empty lanes are one line; Done follows `disclosures.completed`).
- Navigation via `taskNeighbor(..., {arrangement, completedOpen, collapsedLanes})` and `visibleTaskIds`.
- Empty root: the "No tasks yet" state with Open terminal (G14 gating `live && rootState.terminal`).
- Graph view: `SupervisorGraph` with `tierFor`, `dimFor` and `bottomInset = placement==="sheet" ? sheetHeight : 0`.

**S8d Selection, reveal and focus (§5.1 table, DESIGN:429-447).**

- `pendingReveal` ref and a `useLayoutEffect` executor (C6), honouring the origins in D11.
- View switch: save the outgoing offsets (board scroller, per-lane lists, graph scroller) to `scope.view`, set the mode, restore on mount, then reveal the selection only if it is outside the viewport. Focus stays on the segment.
- Strip `Graph ›` focuses the Graph segment and reveals.
- Details Show in…: switch, reveal and focus the target.
- Path rows: select and reveal, focus stays.
- Subagents off with a subagent selected: select its run, focus and reveal the worker node.
- Resize: re-reveal when `isCovered`.
- Selected item gone (run or subagent closed, or task removed): clear, then focus the nearest remaining row or the segment (extending `:113-132`).
- Narrow focus-to-Close effect (`:227-232`) moves focus only when `placement==="overlay"`.
- `detailInvoker` and `closeDetail` (`:234-243`) map selections to `nodeId` for graph invokers.
- Escape layering (`:268-273`): modal / armed / text fields, then queue row collapse or attention overlay, then details, then panels and archive, then hide Supervisor. Escape never changes the view.

**S8e Details panel.**

- `aside#supervisor-detail` with `placement` side (style `flex-basis: value px`), sheet (absolute bottom, `height: value px`, non-modal) or overlay (existing `inset:0`). `PanelSplitter` for side and sheet with bounds from `panelBounds`; `onChange` persists into `scope.view.detailWidth/sheetHeight`.
- `SupervisorActions` gets `stateBlock`:
  - from `attention.ownedForRun`: "Waiting for supervisor" for prepare/execute/question, "Supervisor is reviewing this result" for `to_accept`;
  - else from tier and `agentState`: "<label> is blocked, no question reported" for `runtime_blocked`, else the `agentState` label;
  - `tierLabel` and `waitingSince` from core `since`.
- `path` from `pathNodes` and `nodeFacts`, `crossView` per view, and `acceptanceConflict`.
- Activity/Diagnostics via S10 components. Archive via `ClosedTracking`.
- Dialogs get `onCheck={check}`.

**S8f Header and empty states (F9, F11).**

- Start agent gets class `supervisor-primary` only when there is no verified open root; otherwise `supervisor-secondary`.
- "Start options…" has a visible text label when `!layout.narrow`.
- "Hide Supervisor" replaces "Close Supervisor" (`:278`).
- Activity naming in button, `aside` `aria-label` and heading.
- Header order per DESIGN:353.
- Offline banner per D15.

**S8g CSS (single owner).**

- Remove the obsolete rules: `supervisor.css:33,52-58,64-65,113 (narrow-switch part),132-136,149,159-165,173,175-179,183,186-192,194-204,209-217`.
- Replace `.supervisor-task.is-dimmed{opacity:.58}` (`:95`) with D14.
- Add the rules for:
  - the summary bar and queue (inline cap via style);
  - the view bar (full row at <480);
  - the strip;
  - graph view: scroller fills, sticky heading and column header, node grid `20px minmax(0,1fr) auto` / rows `20px 16px` / areas `"i t x" ". m m"` / gap 8×2 / padding 4×8 / task 3 px left border / subagent dashed / unassigned dotted, icon 16×16 static, tier never shrinks, title and provenance ellipsis (DESIGN:192-204);
  - edges;
  - the sheet and spacer;
  - the side panel (`max-width` removed, size from style);
  - splitters (vertical cursor `col-resize`, horizontal `row-resize`, `:focus-visible` accent line);
  - lane groups;
  - two-line cards and pair chips;
  - state block, trail, operator details and path;
  - Activity and Diagnostics;
  - dialog footer wrap at <480 and `.supervisor-dialog-error-slot.is-empty{min-height:0;height:0}`;
  - short-height compact header (DESIGN:395).
- Structural rules key off the D8 data attributes. Keep the existing tokens and focus ring (`:28`).

**S8h View tests (`SupervisorView.test.tsx`).**

- Keep the DTO builders and add an `attention` parameter. Add `mockWorkareaSize(w, h)`, which stubs `getBoundingClientRect` and `ResizeObserver` (pattern from `viewerTestLayout.ts:3-7`).
- Update:
  - graph test `:219-247`: new node IDs, no "Task links", Graph view must be opened first;
  - splitter test `:251-269`: no "Resize agents overview"; "Resize details" present after selection;
  - `:126-149`: Tasks initial, no graph canvas;
  - History tests `:494-510`: renamed "Activity";
  - question tests `:169-178,359-390`: core `needs_input` entries supplied.
- Add the scenario tests listed in `COVERAGE.md` §3 (rows marked "View test").

- **Non-goals:** core or protocol changes, new orchestration actions, persistence, new shortcuts (DESIGN:37,423).
- **Acceptance:** all checks in §7.1 pass and all browser paths in §7.2 and §7.3 are recorded.

### S12 — Docs migration

Each change describes current behaviour. Removals stay recorded.

- `CONTEXT.md` §5.7, bullets at :326-328:
  - "Needs you" becomes the Decide/Recover/Notice attention queue fed by core attention.
  - "**Agents** is a secondary delegation forest" becomes "the **Graph** view (Tasks · Graph switch, whole workarea; supervisor → task → worker → subagent)".
  - "**History**" becomes "**Activity**".
  - Add: view selection, reveal and resizing never change Herdr focus; only Open terminal does.
- `DECISIONS.md:54`: replace "Agents, History and Diagnostics are secondary" with "Tasks is the initial view; the Graph view, Activity and Diagnostics are secondary". Add one rule bullet: one attention vocabulary from core `attention` plus explicit local conditions; supervisor-owned steps are not user queue items; per-root view choice lives in drafts (not persisted).
- `CODE_GUIDE.md:23`: list `attention.ts`, `topology.ts` (+ `graphLayout.ts`), `useSupervisorLayout.ts`, `reveal.ts`, `PanelSplitter.tsx`, `SupervisorAttention.tsx`, `SupervisorTasks.tsx`, `SupervisorActivity.tsx`.
- `docs/keyboard-shortcuts.md:9`:
  - "**Agents**, **History** and **Diagnostics** are secondary views" becomes "**Graph**, **Activity** and **Diagnostics** are secondary views".
  - Add the graph keys (Tab once, ↑/↓ reading order, ← parent, → first child, Enter selects, Escape closes details), splitter keys (Arrow ±16, Shift ±48, Home reset), and stacked-lane ↑/↓ at narrow widths.
  - Do not touch the generated block (`:11` marker).
- `docs/supervisor-surfaces.md`:
  - Update rows 4, 13, 20, 23, 27, 28 and 36 to the new surfaces.
  - Replace the "Readability refinement" lane note (:11) with the six-lane plus stacked-groups behaviour.
  - Append a 2026-10-07 verification paragraph pointing to `planning/supervisor-implementation-2026-10-07/verification/implementation-verification.md`, separating live from synthetic evidence.
  - Keep the intentional removals (:15,17).
- Acceptance: the §7.4 wording scan is clean; no generated shortcut block is edited.

### S13 — Verification (parent; see §7)

## 7. Verification

### 7.1 Commands (parent, once, after S8 and S12)

```bash
bunx vitest run src/app/supervisor        # all slice and View tests
bun run typecheck                         # tsc --noEmit (tsconfig.json:20 → src)
bun run test                              # full vitest suite (package.json:10)
bun run build                             # tsc + vite build; gateway serves dist/
```

### 7.2 Live disposable-browser path (actual Cockpit, Herdr, OMP; label `LIVE`)

Setup (skill `cockpit-browser-smoke-on-disposable-fixture`):

1. `bun run build`
2. `python3 scripts/verify/ui_polish_runtime.py start` prints `ROOT=/tmp/cpol-…`
3. `PORT` is the last `listening http://127.0.0.1:PORT` line in `$ROOT/gateway.log`.
4. Browser at `http://127.0.0.1:$PORT/`.
5. Top-bar Supervisor, then Start agent: a real OMP launches in the disposable Herdr session (`CONTEXT.md:352`). Wait for "Ready for a task".

Steps:

| Step | Action | Scenarios |
|---|---|---|
| L1 | At 1440×1000, 760×900, 360×800 and 1440×600: Tasks pressed, switch visible without scrolling, strip at ≥720, Graph opens the whole-workarea graph. Hide, return: view kept per root. | 4, 13 |
| L2 | Open terminal from the summary shortcut and instruct the supervisor in its disposable terminal: create three canonical tasks; delegate one to a worker; leave two unassigned; then escalate one NeedsInput question to the user. Agent behaviour is live and non-deterministic; record what actually happened. | 14 (live), 3 |
| L3 | Answer the Decide row: the row clears and focus goes to the selected or first task. The unconfirmed-delivery branch (draft and operation kept, "Retry same message") is not reproduced live, because the fixture script has no gateway restart command. It stays pinned by the DTO View tests (`SupervisorView.test.tsx:359-390`, kept in S8h) and is labelled unit evidence. | 3 |
| L4 | Graph: every open task (assigned and unassigned dotted), worker and any subagent visible; heading counts equal node counts; sticky headers while scrolling. Select a worker: side panel at 1440 (sheet at 360×800), offsets unchanged, node visible. Escape returns focus to the node. Drag the splitter with a real pointer (playwright mouse down/move/up on `[role=separator][aria-label="Resize details"]`) and use the keyboard: `aria-valuenow` and the panel size change within 280–50% (side) and 160–75% (sheet); graph `scrollLeft/Top` unchanged. | 14, 15, 18 |
| L5 | **Herdr focus proof.** Before and after this sequence capture the focus triple twice: `python3 scripts/verify/ui_polish_runtime.py rpc $ROOT session.snapshot '{}'` (method from `CONTEXT.md:111` [INFERENCE: exact focus field names]) and the gateway `SessionSnapshotResponse` `focused_space_id/focused_tab_id/focused_pane_id` (`v1` shape, `SupervisorView.test.tsx:26`). Sequence: Tab into graph, ↑↓←→, Enter, path row, details Show in Tasks then Show in Graph, queue Show in…, view switch, splitter drag and keys, Escape. The triples must be identical. **Positive control:** Open terminal changes the triple. | 19, 6 |
| L6 | Offline: stop the fixture Herdr server process owned by `$ROOT` (`herdr.pid`, as used by `stop` in `ui_polish_runtime.py:159-171`). One banner; cards and nodes show hollow glyph and "unobserved"; no "Saved reports only"; Start/Check disabled or busy semantics hold. | 10 |
| L7 | Missing terminal (if the L2 worker exists): close the worker's pane through Herdr in the disposable session. The Recover row shows Check status as primary, Restart… with its consequence, and Close tracking as quiet text; the healthy root row shows Open terminal only. | 7 |
| L8 | 360×640: summary bar visible, six lane counts visible without horizontal scroll, counters open the attention overlay with focus on Close, Escape returns to the counter, the details overlay first screen shows State (+ Result when present). 1440×600: switch and filters on one line, queue as overlay, graph viewport ≥300 px high, side panel. Return to Tasks: Board scroll, lane state, filters and selected card restored; details Show in Tasks focuses the card. Activity naming; Hide Supervisor label. Board arrows move focus only; splitter attributes; dimmed contrast measured with computed styles (record ratio). | 9, 20, 17, 12, 11 |
| L9 | Cleanup: `python3 scripts/verify/ui_polish_runtime.py stop $ROOT`, `rm -rf $ROOT`, close the browser. Confirm `~/.config/herdr/plugins.json` mtime is unchanged (skill step 7). The user's session is never touched. | — |

### 7.3 Labelled synthetic DTO harness (actual `SupervisorView` with fake client; label `SYNTHETIC DTO`, never presented as live)

Skill `cockpit-component-harness-screenshot`:

1. Create throwaway `supervisor-harness.html` and `supervisor-harness.tsx` at the repo root. The `.tsx` imports `./src/app/styles.css`, mounts `SupervisorView` with a fake `CockpitClient` (the same shape as `SupervisorView.test.tsx:43-55`) that serves the fixture from `?fixture=` (snippets in `examples/fixtures.md`), and passes `session`, `runtimeLive` and `active`.
2. Run `bunx vite --port 5199 --strictPort` as a managed service.
3. Drive it with `playwright-cli resize/goto/eval/screenshot`.
4. Delete the harness afterwards; `git status` must show no harness files.

| Fixture | Content | Scenarios |
|---|---|---|
| `attention-mix` | root question; worker missing (core `exited_without_report`); worker `runtime_blocked`; assignment conflict; `intent_conflict`; `idle_without_report`; reported Result under `to_accept` | 1 at 1440×900 and 760×900; 2; 16 at 1440×1000 and 360×800 |
| `topology-full` | the DESIGN 26-node demo shape: 11 open tasks incl. 2 unassigned, 9 workers incl. a nested worker holding a task, 5 subagents incl. nested; plus 2 accepted tasks (hidden count) | 14 exhaustive; 21 product half at 1440×1000, 760×900, 360×800, 1440×600: DOM geometry checks that the icon is 16×16 inside the 20 px slot, regions are disjoint, the tier is not clipped, all five glyph states and the document icon render |
| `review-result` | reported run with Result, supervisor and operator grants, diagnostic variant, mismatched `current_run_id` variant | 5 |
| `subagent-path` | worker → subagent → nested subagent | 6 |
| `recovery-states` | missing / endpoint_changed / launch_unknown / setup_unknown with each `recovery` kind / plan_failed / healthy | 7 |
| `dialogs` | deleted-task edit, stale edit, cancel subagent, setup recovery kinds, close with five descendants | 8 |
| `offline` | `runtime.status:"unavailable"` and `runtimeLive=false` | 10 |
| `empty-root` | verified root, zero tasks | F11 |
| `closed-root` | closed root with open descendants | F5 orphans, D20 |
| `forest-60` | 60 nodes | Q12 sanity: scroll and sticky headers, no overlap |

Each harness screenshot and check is recorded with `"evidence":"synthetic-dto"`.

### 7.4 Wording and literal scans (scenario 22)

```bash
grep -rniE "focus in board|focus in graph|show on board" src docs CONTEXT.md DECISIONS.md CODE_GUIDE.md   # expect no matches
grep -rnE ">(Board|Agents)<|\"Resize agents overview\"|\"Close Supervisor\"|aria-label=\"History\"|Earlier<" src/app/supervisor   # expect no matches
```

Then assert in the DOM (live L1/L8 plus View tests) that the only view-moving labels are "Show in Graph" and "Show in Tasks". The PRESENTATION half of scenarios 21 and 22 is covered by the unchanged accepted BOUNDED evidence (BOUNDED:24-32). It is not re-claimed.

### 7.5 Evidence output (new only)

`planning/supervisor-implementation-2026-10-07/verification/`:

- `implementation-verification.md`: commands with counts, live steps, synthetic steps, limits;
- `implementation-checks.json`: one record per check: `{scenario, viewport, evidence: "live" | "synthetic-dto" | "unit", result, details}`;
- `impl-live-*.png` and `impl-dto-*.png`.

Do not edit BOUNDED, `bounded-redesign-checks.json`, `full-graph-verification.md`, PRESENTATION or DESIGN.

## 8. Risks and how each slice guards them

| Risk | Where | Guard / verification |
|---|---|---|
| Authority regression (accept, plan override, restart, answer, intents) through re-layout or new disabled semantics | S5, S7, S8b | §3.4 guard table. Unit tests assert exact payloads and disabled states (S5, S7). View tests click every queue action with a fake client and assert action JSON. D13 limits `aria-disabled` to Restart, with a handler guard test. One independent review round of S5/S7/S8b diffs, high-severity only (`.omp/RULES.md:5`). |
| Herdr focus mutated by reveal or selection | S6, S8d | `SupervisorGraph` has no terminal prop. View tests assert `onTerminal` is not called across all §5.1 triggers. Live L5 before/after focus triples with a positive control. |
| Unconfirmed-operation locks lost when moving the Decide/assignment UI into queue rows | S8b | Decide body moved verbatim (`TextAction` plus scoped draft key `answer:<root>:<message>`). Existing tests `:359-390` kept. Draft survives root switch, hide and offline. |
| UI double-counts or drifts from core attention | S1 | No TS thresholds. Dedupe tests. `snapshot.attention` empty means no core rows (D1). Tier map tests. |
| Misleading "need you" counts | S1, S8b | Q-A default; `rootAttentionSummary` tested. |
| Fake topology (inferred assignment or parentage) or ID collisions | S2 | D5 rules; namespaced and encoded IDs; tests for nested, orphan, closed-root, duplicate and colon IDs. |
| Cyclic or deep ancestry hangs | S2 | Iterative layout and chain helpers; tests for cycles and depth 2,000. |
| jsdom vs real layout (container width, scroll, sticky, sheet) | S3, S8, S13 | Unmeasured = wide (E12). `mockWorkareaSize` tests for narrow. Real geometry only from live and DTO browser checks (§7.2–7.3). |
| `scrollIntoView` scrolling `overflow:hidden` ancestors | S3, S6, S8d | `revealNearest` writes only the target scroller (D11). Live check that `.supervisor-view` `scrollTop` stays 0. |
| Native WebKitGTK splitter or fractional-scale mismatch (skill `cockpit-webkitgtk-layout-mismatch`) | S3, S8g | Browser verification is required by `.omp/AGENTS.md:6` (shared frontend). **Recommended** one native smoke of the splitter and sheet; report it as unverified if skipped. |
| Concurrent product edits (widget anchor task `459acf87`, Notes design) | all | No shared files expected (`src/app/supervisor/` only). Parent coordinates if a conflict appears. Unrelated changes are left alone. |
| Accepted artifacts accidentally modified | S12, S13 | Evidence path fixed (D17). The parent checks that `git status planning/supervisor-design-review-2026-10-06` is clean before reporting. |
| Large forests (>50 nodes) | S2, S6 | Q12 default (none). `forest-60` DTO sanity check. Memoized model per snapshot. |
