# Workarea & board inventory

**Slice:** Supervisor entry/header/root selection/start/assignment affordances, status/attention placement (cross-reference R), board lanes/cards/evidence/filter/completed disclosure, narrow Board/Agents and responsive layouts, keyboard/focus/closing, loading/error/no roots/empty lanes/pending/unknown states.

**Provenance:** All observations are **CODE-DERIVED**, not runtime observations. Supervisor component citations use original `src/app/supervisor/*` source line numbers, pinned to [`evidence/SOURCE.md`](evidence/SOURCE.md), captured 2026-10-06T19:26:10.601Z. App/shortcut citations use original source line numbers pinned to [`evidence/APP-SOURCE.md`](evidence/APP-SOURCE.md), captured 2026-10-06T19:27:41.815Z. Snapshot evidence is authoritative for this inventory, not later concurrent checkout content. No runtime/build/test activity was performed by this slice.

## W-ID inventory

### W1 — App entry, mounting, visibility, terminal hand-off

- **CODE-DERIVED:** App renders a pressed-state Supervisor button in the tab toolbar when a Space is selected; click toggles open/close. Commands has “Show Supervisor” and “Start agent”, both without assigned shortcuts. (`src/app/App.tsx:180-189,240,1188`; `src/app/input/shortcuts.ts:128-129`)
- Opening suppresses terminal attach focus, mounts Supervisor, sets local workarea to Supervisor; start variant records session and increments token. Close clears local workarea and suppression. Component stays mounted after open while session exists, keyed by session ID; `active` controls visibility. (`src/app/App.tsx:490-501,634-640,1189-1191`)
- App passes snapshot, runtime liveness, active/start token, callbacks, and navigation error. Managed terminal navigation requires live session/current pane membership and fresh observation matching run session/endpoint/pane/Space/tab. Focus is acknowledged only once App snapshot confirms focused pane; then view closes. Hidden view/session change/disconnect/focus error cancels and reports navigation error. (`src/app/App.tsx:641-672,1190`)
- View itself is hidden when inactive and labeled “Supervisor”; session change resets root/dialog/error/notice/start/restart/start draft state. (`src/app/supervisor/SupervisorView.tsx:264,87-91`)

### W2 — Header, root selector, start actions, archive

- Header: branch icon/brand; conditional root-state glyph/label; Agent selector only for >1 open root; Activity and Diagnostics icon buttons; conditional archive button; Start agent and Start options. Root select is disabled while busy/start-pending/dialog-open, and changing root clears terminal error. (`SupervisorView.tsx:274-279`)
- Root defaults to first open root. `initialRootSnapshot` prevents focus on the pre-root snapshot; focus waits for root-scoped snapshot. Closed root is available only when explicitly selected. (`SupervisorView.tsx:85-94,113-120`)
- Activity/Diagnostics use `aria-pressed`, clear task/run/subagent selection, toggle own disclosure and close the other. (`SupervisorView.tsx:277`; drafts `useSupervisorDrafts.ts:5-19`)
- Start title names focused Space or prompts location choice; says OMP starts without switching terminal focus. Disabled when busy/pending/no snapshot/no connection/start unknown/root starting; label becomes “Starting…”. Start options has same disabled conditions. (`SupervisorView.tsx:137-151,278`)
- Direct start targets focused existing Space; no destination opens start dialog. Lock and pending state prevent duplicate starts; unconfirmed response warns to check status before another start. (`SupervisorView.tsx:67,137-151`)
- Start options draft has empty label, existing location, no Space/directory; destination prefills when unset. Dialog receives Spaces only if runtime-live. (`SupervisorView.tsx:74,278,339`)
- Archive disclosure says tasks/history remain and close did not kill agents/remove resources; allows viewing each closed root and returning to first open root. (`SupervisorView.tsx:279`)

### W3 — Status and attention placement (cross-reference R)

- Snapshot-present status region sits between header and board. Possible content: local notice; start-unknown check/review; terminal/navigation/hook errors and status check; orphaned/root/descendant recovery; root report/runtime evidence; blocked-without-question actions; root Needs you/answer; assignment intents; acceptance conflicts; unidentified task identity repair. (`SupervisorView.tsx:226,280-303`)
- Attention predicate includes unanswered root question, descendant failure/missing/unknown, non-ready/blocked/failed root, orphan worker, assignment intent, acceptance conflict, unidentified items, notice, start unknown, terminal/navigation/hook error. (`SupervisorView.tsx:192-203,226`)
- Region is 44px normally, zero when empty, 164px with attention, 110px under short-height attention; overflow scrolls and Needs you is ordered first. No-attention status compacts; observed details hide at narrow width. (`supervisor.css:33,183-198,206-216,230-239`)
- R slice owns detailed recovery/status behavior; this atlas records placement and conditions only. No live occurrence asserted.

### W4 — Board lanes, counts, empty lanes, Done disclosure

- Lane order: Queued, Preparing, Ready, Working, Review, Done (`accepted`). Each has accessible `{label} tasks`, dot, count; chevrons between non-final lanes are decorative. Done control toggles Show/Hide completed with `aria-expanded` and direction glyph. (`boardNavigation.ts:3-7`; `SupervisorView.tsx:317-321`)
- Done list is hidden when collapsed. Lane counts include all tasks; header open count excludes accepted. Empty lane says “No tasks”, including collapsed Done. (`SupervisorView.tsx:105-107,317-321`)
- Completed disclosure starts false per root scope; hidden tasks omitted from roving IDs and navigation. (`useSupervisorDrafts.ts:15,19`; `SupervisorView.tsx:106-107`; `boardNavigation.ts:10-12`)
- Board scrolls horizontally with snap; lane min width 220px where available, otherwise available container width minus padding. Lists scroll vertically; collapsed Done is 115px. (`supervisor.css:79-95`)

### W5 — Task cards, evidence, filters

- Each card is list item with full-width title/status button, optional attention badge and worker chip; button exposes expanded state, detail controls, row ID, roving tab index. Activation toggles task selection and clears selected run/subagent. (`SupervisorView.tsx:244-263`; CSS `supervisor.css:95-110`)
- Attention if worker needs-input/failure/missing/unknown, assignment intent, or acceptance conflict. Worker glyph uses present actual OMP recognized status (working/idle/blocked/done), otherwise unknown. (`SupervisorView.tsx:248-256`)
- Worker card can show Space/tab label (shared highlight), reported and observed evidence. No worker: “No progress reported yet”. Observed blocked without question: Open terminal/Check status; observed done without Result: “Runtime Done · no result reported”. (`SupervisorView.tsx:244-262`)
- “Needs attention (dims other tasks)” checkbox dims rather than removes cards. Space select has All Spaces and unique observed workspaces among root runs; nonmatching cards dim. (`SupervisorView.tsx:250-253,308-310`; CSS `supervisor.css:74-78,99`; draft defaults `useSupervisorDrafts.ts:13-19`)
- Hover links task to worker/workspace graph highlighting; selected/linked/dimmed have distinct styles. (`SupervisorView.tsx:220-226,250-253`; CSS `supervisor.css:95-112`)
- Arrow navigation changes focus only, not lane/selection. (`boardNavigation.ts:9-24`)

### W6 — Narrow Board/Agents and responsive layout

- With root, Board/Agents view switch uses `aria-pressed`, default Board; active class selects surface. (`SupervisorView.tsx:65,302-308`; CSS `supervisor.css:153-169`)
- At container ≤719px, switch appears and only chosen surface shows; graph splitter hidden and graph grows to available height. At viewport height ≤600px same switch/surface behavior. (`supervisor.css:155-173,197-205`)
- Narrow selected details overlay whole area; content becomes hidden/pointer-disabled. There is no task-input composer in frozen rendered markup. (`supervisor.css:169,188`; `SupervisorView.tsx:302-341`)
- Brand text hides at ≤719px; header controls reposition; below 480px panel buttons wrap; filters compact; narrow no-attention evidence truncates and observed details hide. (`supervisor.css:154-173,196,230-239`)
- Wide details panel flex-basis 340px, max 42%. Agent graph resizable (default 220px/min 80px/max 60vh); no task-input splitter. (`supervisor.css:52-54,113-120,136-140`; `SupervisorView.tsx:304-307`)

### W7 — Assignment intent and composer absence

- Frozen `SupervisorView` contains no task composer, task text draft, textarea, Give task control or composer wrapper. No TaskComposer extraction appears in snapshot; draft types contain message/edit maps, not task draft. Earlier composer claims are superseded. (`SupervisorView.tsx:1-14,45-48,302-341`; `useSupervisorDrafts.ts:3-19`)
- Assignment intent shows “Task assignment pending” or “Task changed elsewhere · Not assigned”; title or “The canonical task is not available yet”; canonical body disclosure when task exists. (`SupervisorView.tsx:291-297`)
- Conflict has “Assign current task” disabled when busy/not live/no canonical/diagnostic/closed, and “Keep unassigned” disabled when busy/disconnected. Non-conflict pending shows no resolution control. (`SupervisorView.tsx:291-297`)
- Resolver submits canonical revision if assigning; no local confirmation/update is set by this callback; visible result depends on snapshot refresh. (`SupervisorView.tsx:291-295`)
- Scoped message/edit drafts, selections, filters and disclosures persist in mounted-workarea memory keyed by root or `unselected`; registry resets on session change. (`useSupervisorDrafts.ts:3-40`; `SupervisorView.tsx:49-53,80-84`)

### W8 — Loading, load error, no roots, empty lanes

- No snapshot: Loading Supervisor/status copy. Hook error: Could not load Supervisor, terminals unchanged/connection failed, Retry load. (`SupervisorView.tsx:280-281`; CSS `supervisor.css:133-135`)
- Snapshot, no root: “Start an agent to manage your tasks.” with work-here-or-terminal copy; board/Agents absent, header start remains. (`src/app/supervisor/SupervisorView.tsx:85-94,302-323`)
- Root with no tasks still shows six lanes, each “No tasks”; no separate board empty message. (`SupervisorView.tsx:98,317-321`)
- Closed roots can be opened via archive with task/history; task composer is absent. (`src/app/supervisor/SupervisorView.tsx:85-94,279,302-323`)
- With snapshot, terminal/navigation/hook errors display in attention with Check status; disconnected text says drafts/resources retained and agent may still run. (`SupervisorView.tsx:284`)

### W9 — Focus, keyboard, closing, selection ownership

- Initial active focus waits until root-scoped snapshot; target selected task if available, else first task, else Start agent. `initialRootSnapshot` prevents focus on initial pre-root snapshot. (`SupervisorView.tsx:76,89-120`)
- Roving list Escape clears selected task else closes view. Up/Down within lane; Home/End first/last; Left/Right nearest nonempty adjacent lane, clamped index. Modified/composition keys bypass custom nav; focus movement does not change selection/lane. (`SupervisorView.tsx:105-107,310-317`; `boardNavigation.ts:9-24`)
- View Escape ignores composition, prevented event, dialog, text fields/select/contenteditable; otherwise closes detail, then Activity/Diagnostics/Archive, then view. Detail Escape closes detail or Activity/Diagnostics. (`SupervisorView.tsx:264-273,325-329`)
- Detail close returns focus to connected in-view invoker, else selected task/agent. Narrow detail open moves focus to panel close. (`SupervisorView.tsx:227-243`; CSS `supervisor.css:169`)
- If focused task disappears, selection clears and focus moves same-index available task or Start agent; announces moved focus. (`SupervisorView.tsx:121-132`)
- After question disappears/answered, focus returns to selected/first task or Start agent. After start verifies, focus returns only if still focused on start invoker. (`SupervisorView.tsx:133-156,196-201`)
- Dialog only renders active with snapshot; modal state callback notifies App. (`src/app/supervisor/SupervisorView.tsx:78-79,339`)

## Coverage / conditional-state matrix

| Surface | CODE-DERIVED condition | Visible result/control |
|---|---|---|
| Entry | Selected Space | App toolbar Supervisor pressed-state toggle; Commands Show Supervisor/Start agent. (`src/app/App.tsx:240,1072-1073,1188`; `src/app/input/shortcuts.ts:128-129`) |
| Root chooser | >1 open roots | Agent select; otherwise absent. (`SupervisorView.tsx:85-94,276`) |
| Start | Snapshot+connection and not busy/pending/unknown/starting | Start enabled, else disabled; pending label. (`SupervisorView.tsx:137-151,278`) |
| Destination | Focused live Space | Existing-Space start; otherwise start dialog. (`SupervisorView.tsx:102-103,137-140`) |
| Start uncertain | Run result unconfirmed | Status check/review acknowledgement; duplicate start blocked. (`SupervisorView.tsx:58,137-151,282`) |
| No snapshot | Loading vs hook error | Status loading or error + Retry load. (`SupervisorView.tsx:280-281`) |
| Snapshot, no root | No selected matching root | Empty start-agent state; no board. (`src/app/supervisor/SupervisorView.tsx:85-94,302-323`) |
| Empty lane | No tasks in lane | “No tasks”. (`SupervisorView.tsx:317-321`) |
| Root needs input | Needs-input report unanswered | Needs you/answer; omitted after answered. (`SupervisorView.tsx:193-201,290`) |
| Worker | current_run resolves | Worker/report/observation; otherwise no-progress copy. (`SupervisorView.tsx:244-262`) |
| Blocked worker | Observed blocked and no needs-input | Open terminal/Check status. (`SupervisorView.tsx:259`) |
| Done/no Result | Observed done, no result | Runtime Done · no result reported. (`SupervisorView.tsx:260`) |
| Attention filter | Checked, task not attention | Card dims, not removed. (`SupervisorView.tsx:248-253,308`) |
| Space filter | Workspace differs | Card dims. (`SupervisorView.tsx:250-253,308`) |
| Completed disclosure | Done lane | Hidden list until shown. (`SupervisorView.tsx:317-320`) |
| Assignment intent | Root has intent | Pending/conflict copy/body; buttons only in conflict. (`SupervisorView.tsx:291-297`) |
| Composer | Frozen view | No composer/task draft; no prior composer claims. (`SupervisorView.tsx:302-341`; `useSupervisorDrafts.ts:3-19`) |
| Unknown task IDs | unidentified item count >0 | Warning and identify action. (`SupervisorView.tsx:299`) |
| Narrow/short | Container ≤719px or viewport ≤600px height | Board/Agents switch, one surface; detail overlay. (`supervisor.css:155-173,197-205`) |

## Source map

Original source-file line numbers are pinned to the frozen evidence files:

- Supervisor view state/start/root/status/card/board/focus: `src/app/supervisor/SupervisorView.tsx:45-156,192-269,264-341` in [`evidence/SOURCE.md`](evidence/SOURCE.md).
- Lane order and keyboard neighbor: `src/app/supervisor/boardNavigation.ts:3-24` in `SOURCE.md`.
- Draft types/defaults/scopes: `src/app/supervisor/useSupervisorDrafts.ts:3-40` in `SOURCE.md`.
- Layout/status/card/responsive styles: `src/app/supervisor/supervisor.css:1-35,52-120,133-173,183-243` in `SOURCE.md`.
- App entry/mount/terminal navigation and shortcuts: `src/app/App.tsx:180-189,240,490-501,634-672,1072-1073,1188-1191`; `src/app/input/shortcuts.ts:128-129` in [`evidence/APP-SOURCE.md`](evidence/APP-SOURCE.md).
- `docs/supervisor-surfaces.md:5-9,17-30,45-53,67-83` is historical context outside the source snapshots, not runtime evidence.

## Runtime evidence boundary

No UI behavior was freshly observed live by this slice. All entries are descriptions of frozen source branches and UI copy. Integration owner supplies runtime evidence and capture timing. The frozen implementation has no task composer; earlier atlas drafts describing textarea, task draft retention or composer submit outcomes are superseded.

## Semantic citation audit

The correction pass reviewed prose, coverage tables and source maps using original source coordinates. App toolbar/mount references now target source240/1190, not snapshot Markdown247/1197. Empty-root coverage includes View323; modal rendering is View339.

The integration owner’s [literal-verified ledger](evidence/CITATION-AUDIT.md) records two exact source substrings for each W ID, with coordinates derived directly from captured source strings. Source-review evidence remains separate from runtime screenshots.
