# Details & actions atlas

> Scope: `SupervisorActions.tsx` and the detail/activity/diagnostics/archive surfaces in `SupervisorView.tsx`. **CODE-DERIVED throughout:** this slice inspected source only; it did not exercise a runtime. Any runtime evidence belongs to the integration owner. All source-file line citations below are pinned to the frozen capture [`planning/supervisor-atlas-2026-10-06/evidence/SOURCE.md`](evidence/SOURCE.md), captured 2026-10-06T19:26:10.601Z; cite the original code-file line numbers as they appeared in that snapshot, not a later checkout. `docs/supervisor-surfaces.md` contains an earlier verification record, cited separately as documentation, not as a fresh observation here.

## Goal & users

Inventory what an operator can inspect and do after selecting a task, run, or internal OMP subagent, plus global Activity, Diagnostics, and closed-tracking archive. This is a current-implementation atlas, not a redesign. The detail panel is entered from board task rows or the agent graph and supports the sections Overview, Activity, and Actions [CODE-DERIVED: `src/app/supervisor/SupervisorView.tsx:233-250,305,326-333`; citations pinned to `evidence/SOURCE.md`].

## Evidence (existing patterns reused)

- **CODE-DERIVED:** The selected-detail `<aside>` is titled from selected task, subagent, or run and changes its `aria-label` between Selected details, Diagnostics, and History [CODE-DERIVED: `src/app/supervisor/SupervisorView.tsx:221-232,326-333`; pinned snapshot].
- **CODE-DERIVED:** Details use an Overview / Activity / Actions segmented nav with `aria-pressed`; its body is a scrollable panel. The shared `SupervisorActions` instance is keyed by selected task/run/subagent, so switching to a different selected item remounts it [CODE-DERIVED: `src/app/supervisor/SupervisorView.tsx:332`; pinned snapshot].
- **CODE-DERIVED:** Detail sections reuse grouped headings, exact pre-wrapped text, and monospace plan/JSON blocks [CODE-DERIVED: `src/app/supervisor/SupervisorActions.tsx:150-190`; `src/app/supervisor/supervisor.css:119-129`].
- **CODE-DERIVED:** Native `<details>/<summary>` disclose individual delivery records and assignment-conflict task text; Activity, Diagnostics, and archive are explicit header toggles [CODE-DERIVED: `src/app/supervisor/SupervisorActions.tsx:120`; `src/app/supervisor/SupervisorView.tsx:277-279,296,333-334`; pinned snapshot].
- **CODE-DERIVED:** `TextAction` retains drafts and operation identity after unconfirmed delivery, announces feedback using status/alert roles, and prevents editing while pending or while an operation remains unresolved [CODE-DERIVED: `src/app/supervisor/SupervisorActions.tsx:59-88`].
- **CODE-DERIVED:** Existing style tokens are CSS variables for color, sizing, borders, and focus; details have 340px basis / 42% max width, overflow scrolling, and narrow layouts overlay the detail panel [CODE-DERIVED: `src/app/supervisor/supervisor.css:1-29,113-129,155-169,188`]. No new component or token is asserted.
- **Documented, not freshly observed here:** `docs/supervisor-surfaces.md:32-53,71-83` describes intended shipped behavior and reports prior browser/native checks, their limits, and runtime evidence. Those statements remain documentation records rather than this slice's runtime observations.

## Flow (step list)

1. **CODE-DERIVED:** Activate a task row or graph run/subagent. Selection clears the other selection kinds and resets the detail section to Overview [CODE-DERIVED: `src/app/supervisor/SupervisorView.tsx:233,305`; pinned snapshot].
2. **CODE-DERIVED:** Read exact task description plus run/subagent report and current observation in Overview; open a terminal only when a fresh observation says the pane is present [CODE-DERIVED: `src/app/supervisor/SupervisorActions.tsx:144-164`].
3. **CODE-DERIVED:** Choose Activity to read saved result, work plan, initialization report, or last subagent control receipt [CODE-DERIVED: `src/app/supervisor/SupervisorActions.tsx:166-174`].
4. **CODE-DERIVED:** Choose Actions for edit/stop, direct message or instruction, durable note, explicit result review, close tracking, or stage-eligible plan override [CODE-DERIVED: `src/app/supervisor/SupervisorActions.tsx:175-187`]. Confirmation workflows are cross-referenced, not restated here: **R02** Edit task, **R04** Close tracking, **R06** Cancel subagent.
5. **CODE-DERIVED:** Use the header Activity / Diagnostics / Closed tracking controls for global records or archived roots; selecting Activity or Diagnostics clears selected detail and mutually closes the other panel [CODE-DERIVED: `src/app/supervisor/SupervisorView.tsx:277-279`; pinned snapshot].
6. **CODE-DERIVED:** Close the panel by its header button or Escape; selection close restores focus to its original invoker if present, otherwise the matching row. Escape closes selected details first; with no selection, panel Escape clears Activity/Diagnostics and outer Escape clears remaining panels/archive or closes the view [CODE-DERIVED: `src/app/supervisor/SupervisorView.tsx:234-250,269-280,326-337`; pinned snapshot].

## Screens/components

### Selected detail (D01–D04)

| ID | Surface / contents | Conditional states and copy | Evidence |
|---|---|---|---|
| D01 | Detail header and section nav | Header name resolves task title, subagent label, or run label. Close button has accessible name “Close details” or “Close activity panel”. Sections Overview, Activity, Actions are buttons with pressed state. | **CODE-DERIVED:** `src/app/supervisor/SupervisorView.tsx:221-232,326-333`; pinned snapshot |
| D02 | Task Overview | Task description displays exact body; task diagnostic adds an error-styled diagnostic. When run is missing, the Overview fallback is “No worker progress reported yet.” | **CODE-DERIVED:** `src/app/supervisor/SupervisorActions.tsx:151-152,164` |
| D03 | Run Overview | Shows Supervisor/Task agent role and parent label, reported evidence and age, and observed Herdr status/time or “Saved reports only”. Worker `needs_input` adds “Waiting for supervisor” and report summary. Terminal button appears only when current fresh observation is present with pane ID; disabled while busy or not live. | **CODE-DERIVED:** `src/app/supervisor/SupervisorActions.tsx:48-58,153-164`; agent state freshness/provenance: `src/app/supervisor/SupervisorActions.tsx:9-35` |
| D04 | Subagent Overview | Shows role fallback “OMP subagent”, parent run (“In … · no terminal”), status/update timestamp, summary fallback “No summary reported.” Parent terminal button appears only if the parent run has a freshly observed pane. | **CODE-DERIVED:** `src/app/supervisor/SupervisorActions.tsx:153-164` |
| D05 | Run Activity | Conditional blocks: explicit Result outcome/summary/time and Accepted vs Awaiting review; Work plan exact text; Initialization report summary and reporter/time. Each record is absent when missing. | **CODE-DERIVED:** `src/app/supervisor/SupervisorActions.tsx:169-173` |
| D06 | Subagent Activity | Shows last control operation and stage, exact sent text for `send`, receipt error and timestamp. Copy distinguishes stored request from applied delivery and applied delivery from task completion. With no receipt: “No control receipt. No child terminal is assumed.” | **CODE-DERIVED:** `src/app/supervisor/SupervisorActions.tsx:167-169` |

### Selected task/run Actions (D07–D13)

| ID | Control and visibility | Disabled / pending / revision / result states | Evidence |
|---|---|---|---|
| D07 | **Edit task…** when a task is selected; invokes **R02** Edit task dialog. | Disabled while busy, disconnected/not live, or task has a diagnostic. | **CODE-DERIVED:** `src/app/supervisor/SupervisorActions.tsx:176`; edit dialog seed: `src/app/supervisor/SupervisorView.tsx:185-188`; detail wiring: `src/app/supervisor/SupervisorView.tsx:332`; pinned snapshot |
| D08 | **Request stop…** when selected task is unchecked, not closed, and there is a verified active root supervisor. | Inline confirmation asks “Ask the supervisor to stop this task? Existing files and Spaces stay.” Controls: “Request stop” / “Keep working”. Request goes to root supervisor, not directly to worker. Success says it is requested, not proof process stopped. Unconfirmed result retains same request identity and says retry keeps it; failures are alert role. Busy disables controls. | **CODE-DERIVED:** `src/app/supervisor/SupervisorActions.tsx:146-148,123-137` |
| D09 | **Message to subagent** + labeled textarea and **Send message** only on selected internal subagent. | Busy if common mutation busy, run closed/not live, or child status not `running`. Pending: “Sending…”. Confirmed: “Control request stored. Check its receipt for applied delivery.” Unconfirmed send is non-retryable: draft/op retained, button becomes “Check previous action first”; user can explicitly “unlock draft” after reviewing receipt/history. | **CODE-DERIVED:** `src/app/supervisor/SupervisorActions.tsx:59-88,178-180` |
| D10 | **Cancel subagent…** only on selected subagent; opens **R06** Cancel subagent dialog. | Disabled while busy, closed/not live, or subagent not running. Its meaning is lifetime cancellation, not a turn abort [documented distinction: `docs/supervisor-surfaces.md:37,49`]. | **CODE-DERIVED:** `src/app/supervisor/SupervisorActions.tsx:178-180`; callback routes to `subagent_cancel`: `src/app/supervisor/SupervisorView.tsx:332`; pinned snapshot |
| D11 | **Follow-up to agent** textarea + **Send follow-up**, for selected run (not subagent). Sends instruction to that run. | Disabled while busy or run is closed/not live. Pending and delivery-uncertain states use shared `TextAction`; uncertain request retains same message ID and text for explicit retry. Success: “Follow-up sent. Waiting for the agent.” | **CODE-DERIVED:** `src/app/supervisor/SupervisorActions.tsx:59-88,181-183` |
| D12 | **Durable note** textarea + **Save note** for selected run. | Disabled while busy or not live. Success “Note saved.” Non-retryable uncertain mutation: retain operation, show “Check previous action first”, allow explicit unlock after reviewing receipt/history. | **CODE-DERIVED:** `src/app/supervisor/SupervisorActions.tsx:59-88,183-184` |
| D13 | **Close agent tracking…** for any selected run not already closed; opens **R04** Close tracking dialog. | Disabled while busy or not live. Not a process kill: UI says tasks/history stay and agent/workers are not guaranteed stopped. | **CODE-DERIVED:** `src/app/supervisor/SupervisorActions.tsx:90-105,185`; dialog callback: `src/app/supervisor/SupervisorView.tsx:332`; pinned snapshot |

### Result review and plan override (D14–D16)

| ID | Control and gating | State detail | Evidence |
|---|---|---|---|
| D14 | When run stage is `reported`, **Operator result review override** offers **Accept explicit result** and **Requested changes** textarea / **Send back**. | Accept sends the exact current `task.task.task_revision` as `expected_task_revision`; disabled if busy, not live, no task, task diagnostic, or task current run differs from selected run. The text says explicit result, not runtime Done; no additional confirmation control in this view. Send back is non-retryable after uncertain result and uses common retained-draft/receipt unlock handling. | **CODE-DERIVED:** `src/app/supervisor/SupervisorActions.tsx:184`; acceptance state labels in `src/app/supervisor/SupervisorActions.tsx:40-46,169-171` |
| D15 | **Operator plan override** only for selected run at `awaiting_prepare` (prepare plan) or `ready` (work plan). Renders exact plan, calls out operator replacing supervisor decision and same-user policy (not OS sandbox). | No plan: “No current plan is available.” Review is tied to `plan_revision`; changed revision shows warning + “Reviewed current plan” and disarms confirmation. Stable revision offers “Override prepare…” / “Override execute…”. Armed state asks “Authorize this exact plan?” and exposes “Confirm prepare/execute override” plus “Back”; Escape disarms. Confirm disabled while busy or armed revision no longer matches. | **CODE-DERIVED:** `src/app/supervisor/SupervisorActions.tsx:107-115`; stage gate: `src/app/supervisor/SupervisorActions.tsx:186` |
| D16 | Execute override has optional execution note textarea; prepare has no note field. | Note is read-only while busy. Grant call carries exact revision and execute note (or null); confirmation clears armed state; no returned result clears reviewed revision so review must be repeated. | **CODE-DERIVED:** `src/app/supervisor/SupervisorActions.tsx:107-115` |

### Global Activity, Diagnostics, archive (D17–D20)

| ID | Surface / records | Empty, stale, failure, and control states | Evidence |
|---|---|---|---|
| D17 | Header **Activity** icon toggles global History (“Earlier”); selecting it clears selected task/run/subagent and closes Diagnostics. Timeline merges messages addressed to runs belonging to the selected root, grants on those runs, and their annotations. Sorted newest-first. Message actor resolves operator to You, dispatcher to Dispatcher, run label or Agent. Brief/observation/task rows show readable summary and point to Diagnostics for exact delivery; report summaries/text shown otherwise. Grant label attributes authorization to supervisor or “You (origin)” and calls out exact plan provenance in Diagnostics. Result-review receipt annotations render as a concise “Result review note for …”. | `stale` messages append “· stale evidence”. Empty: “No recorded history in this scope.” Each event includes localized timestamp and detail. This is a root-scoped filtered history view, not every raw record. | **CODE-DERIVED:** `src/app/supervisor/SupervisorView.tsx:40-43,204-219,277,326,333`; pinned snapshot |
| D18 | Header **Diagnostics** icon toggles diagnostics and closes Activity; clears selection. Shows canonical task source path or “No selected task document”; unidentified task count/action; board diagnostics; per-run records; serialized runtime and transaction objects. | Identify task-file items disabled while busy or disconnected. Each run has JSON “Run, binding, setup and exact plans” plus expandable inbox messages to/from run with kind/stage/stale/timestamp and JSON. “Launch receipts and saved reports are not current process proof.” Raw data remains pre-wrapped in monospace blocks. | **CODE-DERIVED:** `src/app/supervisor/SupervisorView.tsx:277,326,334`; `src/app/supervisor/SupervisorActions.tsx:116-121`; pinned snapshot |
| D19 | Diagnostics transaction/runtime dump contains `runtime`, `intents`, `assignment_intents`. | Freshness/connection distinctions are present in the raw runtime record; source does not add separate error/empty text for absent run messages or empty diagnostics lists. Not a fresh-live status assertion. | **CODE-DERIVED:** `src/app/supervisor/SupervisorView.tsx:334`; runtime observation formatting: `src/app/supervisor/SupervisorActions.tsx:53-58`; pinned snapshot |
| D20 | Header **Closed tracking · N** appears only when closed roots exist; its `aria-expanded` toggles archive popover. Popover is labelled “Closed tracking”, says tasks/history remain and closing did not kill agents/remove resources, and offers **View {label} tasks and history** per closed root. When a closed root is selected and another root remains open, offers **Return to {label}**. | Selecting archived root changes root context and closes archive. View root buttons disabled while busy or dialog open; return disabled while busy. No explicit empty archive panel; trigger absent when no closed roots. | **CODE-DERIVED:** `src/app/supervisor/SupervisorView.tsx:85-88,277-279`; popover placement: `src/app/supervisor/supervisor.css:142-143`; pinned snapshot |

### Conditional status and errors within detail flow (D21)

- **CODE-DERIVED:** Detail derives observed run only when runtime is `fresh` and `live`; reported and observed evidence are distinct. Closed runs suppress Observed evidence. Absence is not inferred from stale/unavailable observations; status labels include connection lost, unavailable observation, terminal gone, changed endpoint, unconfirmed setup/start, and tracking closed [CODE-DERIVED: `src/app/supervisor/SupervisorActions.tsx:9-35,48-58`].
- **CODE-DERIVED:** Global error region can report terminal navigation failure, navigation identity/location not confirmed, stale/disconnected refresh (saved tasks/drafts kept, agent may still run), or unconfirmed mutation (“Check status”); errors use alert role and Check status action [CODE-DERIVED: `src/app/supervisor/SupervisorView.tsx:284`]; pinned snapshot.
- **CODE-DERIVED:** A selected task whose current run is absent still has its task description and task action controls; run-only activity/actions are absent. A selected run without task has no task edit/stop/result-accept control [CODE-DERIVED: `src/app/supervisor/SupervisorActions.tsx:151-187`]; pinned snapshot.

### Initial root and focus handoff (D22)

- **D22 · CODE-DERIVED:** When the first snapshot lists open roots and no root is selected, the view records that snapshot and selects the first open root. Initial focus deliberately waits until a subsequent snapshot scoped to that selected root; it then focuses the selected task row if it still exists, otherwise the first task row, or Start agent when the scoped task list is empty. This avoids focusing against the pre-root board snapshot [CODE-DERIVED: `src/app/supervisor/SupervisorView.tsx:76,85-94,106-120`; pinned snapshot].
- **CODE-DERIVED:** If a focused task disappears from the updated list while active, selection is cleared when applicable, focus moves to the nearest next available row (or Start agent if no rows remain), and a status notice reports that focus moved [CODE-DERIVED: `src/app/supervisor/SupervisorView.tsx:121-132`; pinned snapshot].

## Interaction & keyboard

- **CODE-DERIVED:** Task-row selection toggles same task closed and clears run/subagent selection; graph selection likewise toggles same run/subagent and clears task selection. Each selection resets detail tab to Overview [CODE-DERIVED: `src/app/supervisor/SupervisorView.tsx:233,305`]; pinned snapshot.
- **CODE-DERIVED:** Section switches, header toggles, and action controls are native buttons, so pointer activation and standard button keyboard activation share the same handlers. This slice defines no custom shortcut beyond Escape [CODE-DERIVED: `src/app/supervisor/SupervisorView.tsx:269-280,326-339`; `src/app/supervisor/SupervisorActions.tsx:151-187`]; pinned snapshot.
- **CODE-DERIVED:** Escape in the detail panel closes selected detail and restores original invoking element if still attached; fallback focuses the selected row. With no selection, the panel Escape handler clears Activity/Diagnostics; outer Escape then clears remaining history/diagnostics/archive disclosures or closes the view. Inputs/textareas/select/contenteditable suppress outer Escape handling; plan override intercepts Escape only while confirmation is armed [CODE-DERIVED: `src/app/supervisor/SupervisorView.tsx:234-250,269-280,326-337`; `src/app/supervisor/SupervisorActions.tsx:112-114`]; pinned snapshot.
- **CODE-DERIVED:** At container width below 720px, detail becomes an absolute full-workarea panel, and the underlying workarea content is hidden/non-interactive while a panel is open [CODE-DERIVED: `src/app/supervisor/supervisor.css:155-169,188`]; pinned snapshot.
- **CODE-DERIVED:** The `SupervisorActions` instance is keyed by selected task/run/subagent and remounts when selection changes. This resets component-local armed/pending/error state; text draft records are scoped separately [CODE-DERIVED: `src/app/supervisor/SupervisorView.tsx:332`; `src/app/supervisor/SupervisorActions.tsx:63-66,108-109,124-126,149,179,182-184`]; pinned snapshot.
- **CODE-DERIVED:** At widths below 720px, opening selected detail or global Activity/Diagnostics focuses the detail header close button. No explicit focus movement is wired for switching Overview/Activity/Actions; no equivalent focus move is set for a wide-layout global panel [CODE-DERIVED: `src/app/supervisor/SupervisorView.tsx:227-239,331-332`]; pinned snapshot.

## Accessibility

- **CODE-DERIVED:** Sections expose names (`Supervisor panels`, `Selected details`/`Diagnostics`/`History`, `Detail sections`, `Closed tracking`); toggles expose `aria-pressed` or `aria-expanded`; action text inputs use labels; async notices and errors use status/alert roles [CODE-DERIVED: `src/app/supervisor/SupervisorView.tsx:277-279,326-334`; `src/app/supervisor/SupervisorActions.tsx:81-87,95-105,123-137`]; pinned snapshot.
- **CODE-DERIVED:** Focus return is explicitly managed on detail close. At widths below 720px, opening selected detail or global Activity/Diagnostics focuses the detail header close button. No explicit focus movement is wired for switching Overview/Activity/Actions; no equivalent focus move is set for a wide-layout global panel [CODE-DERIVED: `src/app/supervisor/SupervisorView.tsx:227-239,234-250,331-332`]; pinned snapshot.
- **CODE-DERIVED:** `:focus-visible` outlines use `--focus-strong`; exact text wraps anywhere and plan/JSON data is wrapped in monospace blocks [CODE-DERIVED: `src/app/supervisor/supervisor.css:22-29,126-127`]. Contrast was not measured by this source review.

## Options considered

Not applicable to this current-state atlas: no design option or target behavior is being proposed.

## Open questions (options + recommendation)

1. **Dialog cross-references:** **R02** Edit task, **R04** Close tracking, and **R06** Cancel subagent are owned by the dialogs/recovery slice. Their content is intentionally not duplicated here.
2. **Runtime confirmation:** source can establish visibility/guard predicates but not whether exact controls, focus behavior, disabled states, or errors were encountered in a live session. **Recommendation:** integration owner supplies runtime evidence against this ID map; until then these entries remain CODE-DERIVED only.

## Acceptance scenarios (observable UI checks)

All scenarios below are source-derived coverage targets, **not tests run by this slice**.

1. **CODE-DERIVED:** Select task with a current worker, task diagnostic, and report: confirm task body, diagnostic, reported evidence, separately labelled observed evidence, and terminal action only when fresh pane observation exists [CODE-DERIVED: `src/app/supervisor/SupervisorActions.tsx:48-58,151-164`].
2. **CODE-DERIVED:** Select a task with no current run, a run without a task, a subagent with no control receipt, and one with failed send receipt: verify conditional absence/fallbacks, no assumed child terminal, and exact stored control/error detail [CODE-DERIVED: `src/app/supervisor/SupervisorActions.tsx:151-169,178-180`].
3. **CODE-DERIVED:** For a running task under a verified root, open Request stop and choose Keep working; repeat and request stop with unconfirmed delivery. Confirm cancel path, retained request identity, and copy that delivery is not proof of process stop [CODE-DERIVED: `src/app/supervisor/SupervisorActions.tsx:123-137,146-148`].
4. **CODE-DERIVED:** For each eligible run stage, inspect action gates: reported task with current revision; reported result with missing/diagnostic/mismatched task; awaiting_prepare and ready with current/missing/changed plan revision. Confirm accept carries current exact revision, plan revision change requires review again, and execute note is optional [CODE-DERIVED: `src/app/supervisor/SupervisorActions.tsx:107-115,184,186`].
5. **CODE-DERIVED:** Exercise text action pending, confirmed, uncertain, retryable and non-retryable operation states for follow-up, note, send-back, and subagent message; confirm retained draft behavior and explicit receipt-review unlock where applicable [CODE-DERIVED: `src/app/supervisor/SupervisorActions.tsx:59-88,178-184`].
6. **CODE-DERIVED:** Open Activity with no events, stale message, supervisor grant, operator-origin grant, annotation, and review receipt note; confirm sort, actor attribution, stale marker and empty copy [CODE-DERIVED: `src/app/supervisor/SupervisorView.tsx:40-43,204-226,333`].
7. **CODE-DERIVED:** Open Diagnostics with/without board diagnostics and unidentified items; expand a run message record; confirm JSON covers run/binding/setup/plans, message provenance, runtime and transaction intents, and no claim that saved receipt proves live process [CODE-DERIVED: `src/app/supervisor/SupervisorView.tsx:334`; `src/app/supervisor/SupervisorActions.tsx:116-121`].
8. **CODE-DERIVED:** Open archive with zero/one/multiple closed roots; verify trigger absent at zero, label/count and correct archived tasks/history, return to open root, busy/dialog disabling, and Escape/close behavior [CODE-DERIVED: `src/app/supervisor/SupervisorView.tsx:85-88,277-280`; `src/app/supervisor/supervisor.css:142-143`].
9. **CODE-DERIVED:** Close details after selection, with original invoker still mounted and removed; verify focus restoration to invoker or row fallback. At narrow width verify detail replaces underlying interaction surface; no claim of runtime layout verification by this slice [CODE-DERIVED: `src/app/supervisor/SupervisorView.tsx:227-250,326-337`; `src/app/supervisor/supervisor.css:155-169,188`].

10. **CODE-DERIVED:** Load initial snapshot with open roots and delayed root-scoped board snapshot, including empty and nonempty tasks; verify focus waits for the selected root's snapshot, then goes to the selected/first task or Start agent. Remove the focused row and confirm focus moves to the nearest task or Start agent with the announced notice [CODE-DERIVED: `src/app/supervisor/SupervisorView.tsx:76,85-94,106-139`]; pinned snapshot.

## Source map (frozen source snapshot)

| Source | Role in this slice |
|---|---|
| `src/app/supervisor/SupervisorActions.tsx:1-190` | Overview, Activity, Actions, plan overrides, diagnostics records, text actions, and lifecycle evidence. |
| `src/app/supervisor/SupervisorView.tsx:52-139,140-198,200-250,271-341` | Owns selection/detail state, event history, entry points, focus return, global panels/archive, and dialog availability. Includes delayed initial-root focus gating (`src/app/supervisor/SupervisorView.tsx:76,89-94,113-120`). |
| `src/app/supervisor/supervisor.css:113-143,155-173,188` | Detail sizing/scroll, archive placement, and narrow-panel behavior. |
| `docs/supervisor-surfaces.md:32-53,71-83` | Previously recorded feature-surface/verification context; not a fresh runtime observation by this slice. |

All source citations in this report are original source-file line numbers frozen in [`evidence/SOURCE.md`](evidence/SOURCE.md), not live-checkout references. The snapshot's `SupervisorView.tsx` imports no `TaskComposer` and contains no composer render (`src/app/supervisor/SupervisorView.tsx:8-21,271-341`); no extracted TaskComposer owner is in this evidence set.

## Examples (paths)

No mock or prototype: this assignment is a code-derived atlas of existing UI, not a proposed design.

## Semantic citation audit

The correction pass reviewed prose, evidence/coverage tables, interaction/accessibility claims and source maps using original file coordinates. The selected-detail remount is View332, not339. History is selected-root-scoped; narrow global panels explicitly focus the close button, unlike wide global panels.

The integration owner’s [literal-verified ledger](evidence/CITATION-AUDIT.md) records two exact source substrings for each D ID. Illustrative ellipsis snippets are not used as exact-source proof. Runtime coverage remains unchanged.