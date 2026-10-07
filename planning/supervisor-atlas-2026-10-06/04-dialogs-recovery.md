# Supervisor dialogs and recovery — frozen-source atlas

**Citation boundary:** All Supervisor UI citations use original source-file line numbers pinned to `planning/supervisor-atlas-2026-10-06/evidence/SOURCE.md`, captured **2026-10-06T19:26:10.601Z**. They do not refer to subsequent checkout edits. Backend/protocol citations refer to the named source files. Every observation is **CODE-DERIVED**; no runtime scenario was exercised here.

## Stable IDs

| ID | Surface |
|---|---|
| R01 | Start agent dialog / Start options |
| R02 | Edit task dialog and stale/deleted conflicts |
| R03 | Restart agent confirmation |
| R04 | Close tracking confirmation |
| R05 | Setup recovery confirmation |
| R06 | Cancel subagent confirmation |
| R07 | Agent status and recovery card |
| R08 | Attention projection, Needs you, intent conflicts, orphaned workers |

## R01 — Start agent

**Entry/target.** Header **Start agent** immediately starts in the focused Space when available; otherwise opens options. **Start options…** opens the dialog and preselects focused Space when possible (`src/app/supervisor/SupervisorView.tsx:137-150,278-279`). Dialog offers optional name and Existing Space, Directory, Dedicated agent folder (`src/app/supervisor/SupervisorDialogs.tsx:85-89`). Dedicated-folder copy says “Creates a dedicated agent folder and Space. Existing project context is not copied.” The separate focus copy says “Starts OMP without switching your terminal focus.” (`src/app/supervisor/SupervisorDialogs.tsx:89-90`).

| Condition | CODE-DERIVED result |
|---|---|
| Existing Space selected but not among available `spaces` | Refuses with “Choose a currently available Space. No different destination will be selected automatically” (`src/app/supervisor/SupervisorDialogs.tsx:49`). Parent supplies `spaces=[]` when runtimeLive is false (`src/app/supervisor/SupervisorView.tsx:339`). |
| Directory does not trim to an absolute path | Refuses with “Enter an absolute directory path” (`src/app/supervisor/SupervisorDialogs.tsx:52`). |
| Directory accepted | Builds setup request with trimmed path, trimmed optional label or null, `task_name: null`, `focus: false` (`src/app/supervisor/SupervisorDialogs.tsx:53`). |
| Optional label empty | `supervisor_start` action uses trimmed label or null (`src/app/supervisor/SupervisorDialogs.tsx:55`). |
| Dedicated agent folder selected | No setup target is added; action carries optional label or null (`src/app/supervisor/SupervisorDialogs.tsx:55`). |
| Required connection unavailable | Submit reports required connection unavailable; primary disabled; draft/resources retained (`src/app/supervisor/SupervisorDialogs.tsx:43–44,105`). Parent availability requires connected; for start it also requires runtimeLive and fresh snapshot (`src/app/supervisor/SupervisorView.tsx:339`). |
| Missing/wrong mutation result | Modal sets unconfirmed and invokes parent callback; error warns prior request may have opened terminal. View tracks `startUnknown`, blocks start controls, offers Check status and explicit reviewed-start acknowledgment (`src/app/supervisor/SupervisorDialogs.tsx:65-75,105`; `src/app/supervisor/SupervisorView.tsx:58-59,137-150,278-279,283`). |
| Confirmed run result | Parent selects returned run and dialog closes (`src/app/supervisor/SupervisorDialogs.tsx:65-75`; `src/app/supervisor/SupervisorView.tsx:132-136`). |

Immediate start refuses absent snapshot/connection, busy, in-flight start, unresolved `startUnknown`, or a root already starting. An ambiguous result says a terminal may have opened. `startToken` triggers the same start path (`src/app/supervisor/SupervisorView.tsx:137-150,152-157`). Protocol setup and action validators are in `src/client/orchestrationProtocol.ts:40-62,155-170`.

## R02 — Edit task

Entry from Actions → **Edit task…**, disabled when busy, not live, or task has diagnostic (`src/app/supervisor/SupervisorActions.tsx:176`). Parent reuses/creates scoped draft from current title/body/revision (`src/app/supervisor/SupervisorView.tsx:185-188`). Dialog rejects blank title/missing board and submits `task_update` with the draft’s exact expected revision (`src/app/supervisor/SupervisorDialogs.tsx:56-60`; `src/client/orchestrationProtocol.ts:155-170`).

| State | CODE-DERIVED behavior |
|---|---|
| Current revision | Save is available subject to common gates; confirmed task result clears parent draft (`src/app/supervisor/SupervisorDialogs.tsx:65-75`; `src/app/supervisor/SupervisorView.tsx:339`). |
| Revision changed | Warning says task changed elsewhere; retains user draft; current task appears in review details. Review action updates expected revision only, not draft content. Disabled when locked/current task diagnostic (`src/app/supervisor/SupervisorDialogs.tsx:79-82,94`). |
| Task deleted from current board | `currentTask` null; stale warning remains but no rebase control. Draft still submits old task ID/revision if attempted (`src/app/supervisor/SupervisorDialogs.tsx:79-82,94`). |
| Board unavailable / blank title | Submit refuses, keeping draft (`src/app/supervisor/SupervisorDialogs.tsx:56-60`). |
| Missing/wrong result or exception | Draft retained. Edit does not set modal `unconfirmed` state (`src/app/supervisor/SupervisorDialogs.tsx:65-77`). |

Parent availability permits edit when connected even if runtime is not fresh; board/task snapshot must still be present (`src/app/supervisor/SupervisorView.tsx:339`).

## R03 — Restart agent

Entry handler: `src/app/supervisor/SupervisorView.tsx:166-185`. SetupUnknown opens R05; PlanFailed reconciles directly; LaunchUnknown/NeedsReview opens R03. Other restart requests reconcile first. The pending-restart effect waits for a fresh snapshot; a verified original produces “still connected/no new launch,” uncertainty opens the confirmation, and unavailable observation says no new launch was requested (`src/app/supervisor/SupervisorView.tsx:166-185`). Dialog warns previous agent may remain, restart creates another terminal, and active descendants remain while only selected run restarts (`src/app/supervisor/SupervisorDialogs.tsx:95-97`). **Restart anyway** emits `retry_launch` (`src/app/supervisor/SupervisorDialogs.tsx:62,105`; fixture assertion `src/app/supervisor/SupervisorView.test.tsx:262-278`).

Core allows retry only for LaunchUnknown/NeedsReview and rejects Closed/Reported. It increments attempt, clears prior location/session binding, marks prior briefs stale, preserves same run/root and transitions to Preparing/SetupPending (`crates/cockpit-core/src/orchestration.rs:883-926`). Unconfirmed mutation disarms non-edit dialog submission (`src/app/supervisor/SupervisorDialogs.tsx:65-77,105`).

## R04 — Close tracking

Confirmation says tasks/history/Spaces/worktrees remain and closing tracking does not guarantee agent/worker termination; active descendant count/copy says descendants stay open and need supervision (`src/app/supervisor/SupervisorDialogs.tsx:98-99`). Footer choices are Keep tracking and Close tracking (`src/app/supervisor/SupervisorDialogs.tsx:105`). Core closes tracking as Cancelled, writes advisory cancellation and annotation, and retains descendants/resources (`crates/cockpit-core/src/orchestration.rs:854-881`). Recovery card close control appears for nonclosed run and disables while busy/disconnected (`src/app/supervisor/SupervisorActions.tsx:102`). Parent modal availability requires connected (`src/app/supervisor/SupervisorView.tsx:339`). Archive and orphan surfaces preserve access to closed roots/remaining children (`src/app/supervisor/SupervisorView.tsx:87-88,279,285-286`).

## R05 — Recover setup

SetupUnknown opens R05; PlanFailed reconciles directly (`src/app/supervisor/SupervisorView.tsx:166-171`). Dialog says resources remain. `accept_existing_worktree` confirms reuse of recorded receipt; `retry_environment` explicitly retries uncertain setup after review; otherwise copy says reconcile/check only, not launch. Exact plan/path/effects/warnings appear when available (`src/app/supervisor/SupervisorDialogs.tsx:60-62,100-101`). Submit passes recorded recovery to `reconcile_run` (`src/app/supervisor/SupervisorDialogs.tsx:60-62`). Core requires an explicit choice for SetupUnknown, rejects null, and stores recovery choice (`crates/cockpit-core/src/orchestration.rs:927-980`). Valid enum values: `src/client/orchestrationProtocol.ts:37,169`. Unconfirmed result disarms dialog (`src/app/supervisor/SupervisorDialogs.tsx:65-77,105`).

## R06 — Cancel subagent

Actions renders **Cancel subagent…** for a selected subagent and parent run; disabled if busy, controls unavailable, or status is not `running` (`src/app/supervisor/SupervisorActions.tsx:178-180`). Thus done/failed/cancelled children keep a visible disabled button. Confirmation says the request does not prove stop; OMP control receipt does (`src/app/supervisor/SupervisorDialogs.tsx:60,99`). The action is `subagent_control(cancel)` and modal closes only for confirmed `done` or `message` result (`src/app/supervisor/SupervisorDialogs.tsx:60-75`). Internal subagent has no separate terminal; detail exposes parent terminal and activity distinguishes stored from applied control/completion (`src/app/supervisor/SupervisorActions.tsx:151-173`; `crates/cockpit-core/src/orchestration/messages.rs:815-877`).

## Shared modal behavior and availability

The discriminated union has six modes (`src/app/supervisor/SupervisorDialogs.tsx:10-12`). Portal dialog has modal semantics/label/busy status, restores focus, focuses once on open, wraps Tab, closes on Escape only if not in-flight/global-busy, locks while pending, and renders error/unavailable copy that preserves draft/resources (`src/app/supervisor/SupervisorDialogs.tsx:17-38,80-85,104-107`). Parent renders only while active with snapshot and supplies availability (`src/app/supervisor/SupervisorView.tsx:339`). Session change clears dialog, resets start draft/start uncertainty/restart pending (`src/app/supervisor/SupervisorView.tsx:82-84`).

| Mode | Parent availability (`src/app/supervisor/SupervisorView.tsx:339`) | Unknown-result state |
|---|---|---|
| Start | Connected + runtimeLive + fresh snapshot | Modal disarmed; view-level startUnknown blocks repeats pending review. |
| Edit | Connected | Draft retained; modal not disarmed. |
| Restart/setup recovery/subagent cancel | Connected + runtimeLive + fresh snapshot | Modal disarmed. |
| Close tracking | Connected | Modal disarmed. |

## R07 — Agent status and recovery controls

`agentState` uses the matching run observation only when connected, runtimeLive and snapshot runtime fresh. Terminal means presence present plus pane ID. Restartable requires dispatch and either PlanFailed/SetupUnknown or an observation that is not Unobserved with `actual_omp=false` (`src/app/supervisor/SupervisorActions.tsx:9-13`). State precedence: closed; disconnected/offline; runtime unavailable; Missing; endpoint changed; Proposed/AwaitingPrepare; PlanFailed; SetupUnknown; LaunchUnknown/NeedsReview; verified bound actual OMP; starting; fallback unknown (`src/app/supervisor/SupervisorActions.tsx:14-35`).

| Evidence | UI status | Controls/constraints |
|---|---|---|
| Closed | Tracking closed; tasks/history kept, stop not guaranteed | No Check/recovery/Close control. |
| Disconnected/runtime not live | Connection lost; saved tasks; agent may still run | Check connection; no recovery card. |
| Runtime unavailable | Cannot check; no absence inferred | Check connection; no recovery card. |
| Missing presence | Agent terminal gone; tasks/history saved | Check; restart only if restartable; close tracking. |
| Endpoint changed | Cannot confirm agent | Check; no absence inferred. |
| Proposed/AwaitingPrepare | Waiting for supervisor, no worker process launched | Check; excluded from descendant recovery list (`src/app/supervisor/SupervisorView.tsx:192`). |
| PlanFailed + error | Agent did not start plus error | Retry setup/reconcile if restartable. |
| SetupUnknown | Setup unconfirmed/resources retained | Recover setup. |
| LaunchUnknown/NeedsReview | Start unconfirmed/possible duplicate | Restart confirmation if restartable. |
| Fresh present actual OMP + bound session + launched, or adopted without dispatch | Verified; root Managing tasks/Ready for a task/OMP connected; worker Agent connected or Preparing · OMP connected; blocked observation changes label | Open terminal if pane present; Check hidden for ready; Close tracking. |
| Preparing/Initializing/LaunchPending/LaunchIntent | Starting agent | Check, no recovery card. |
| Other/unbound | Cannot confirm; reports are not live process proof | Check; restart requires dispatch plus eligible observed state. |

Open terminal is rendered only with terminal evidence and disabled busy/disconnected/not-runtime-live. Check is shown for non-ready/non-closed and disabled busy. Recovery requires failure/missing/unknown, non-Reported and dispatch; disabled busy/disconnected/not-runtime-live/not-restartable. Label PlanFailed→Retry setup, SetupUnknown→Recover setup…, otherwise Restart agent…. Close appears for nonclosed and is disabled busy/disconnected. Reported recovery warns result must be reviewed/sent back; nonrestartable copy requires current terminal observation or suggests new tracked agent (`src/app/supervisor/SupervisorActions.tsx:90-104`).

## R08 — Attention, conflicts and orphaned workers

### Frozen source ownership

Frozen `SOURCE.md` includes SupervisorView, SupervisorActions, SupervisorDialogs, SupervisorGraph, graphLayout, RowSplitter, CSS, board navigation, `useSupervisor`, and `useSupervisorDrafts`; no TaskComposer/separate task-composer file is included. Assignment intent resolution remains in the captured view (`src/app/supervisor/SupervisorView.tsx:202-203,291-297`). Captured root-scoped focus handling stores initial snapshot and waits for root-scoped snapshot before focusing selected/first task or Start (`src/app/supervisor/SupervisorView.tsx:59,95-117`).

### Projection and attention truth

Projection joins durable state, canonical Markdown and runtime observation; does not persist runtime evidence or complete tasks from observation (`crates/cockpit-core/src/orchestration/projection.rs:8-15`). Snapshot includes roots/board/runs/messages/subagents/intents/assignment intents/runtime/unmanaged agents/attention (`crates/cockpit-core/src/orchestration/projection.rs:33-174`; `crates/cockpit-protocol/src/orchestration.rs:29-54`). Task lanes derive checked task/current run stage rather than runtime (`crates/cockpit-core/src/orchestration/projection.rs:173-222`). Frontend status prioritizes assignment intent, checked task, closed run, NeedsInput, reported/review and lane (`src/app/supervisor/SupervisorActions.tsx:37-46`).

| Derived attention | Condition |
|---|---|
| AwaitsPrepare / AwaitsExecute | Run stage AwaitingPrepare / Ready. |
| ToAccept | Run Reported; Result time preferred. |
| DispatchUnknown | SetupUnknown/LaunchUnknown/NeedsReview, or endpoint changed unless already represented by uncertain dispatch. |
| PlanChanged | Dispatch error code exactly `plan_changed`. |
| NeedsInput | Last report NeedsInput without nonstale Answer at/after it. |
| BriefUnread | Latest nonstale prepare/work brief Stored/Woken >120s; prepare additionally requires location. |
| ExitedWithoutReport | Fresh Missing observation. |
| RuntimeBlocked | Present + blocked, and no unanswered NeedsInput. |
| IdleWithoutReport | Working, no Result, Present idle/done, work brief exists and relevant report/brief/status-change time >300s. |
| IntentConflict | Acceptance intent Conflict. |

Rules `crates/cockpit-core/src/orchestration/projection.rs:371-505`; conflict added `crates/cockpit-core/src/orchestration/projection.rs:85-103`. No observation means no runtime-derived attention; it does not imply absence (`crates/cockpit-core/src/orchestration/projection.rs:463-505`).

### Aggregate, Needs you and per-task badges

Aggregate flags unanswered question, descendant failure/missing/unknown except Proposed/AwaitingPrepare, root nonready/blocked/failed Result, orphan children, assignment/acceptance conflicts, unidentified tasks, notices/unknown start/navigation/terminal/load errors (`src/app/supervisor/SupervisorView.tsx:191-210,226,284-299`). Per-task attention includes worker NeedsInput/failure/missing/unknown plus assignment/acceptance intent; filter dims other tasks (`src/app/supervisor/SupervisorView.tsx:249-250`).

**Needs you** shows report summary and reported-by/time; Answer action is disabled busy/not-live/root-unverified (`src/app/supervisor/SupervisorView.tsx:193-195,290`). Nonstale Answer at/after question resolves it (`src/app/supervisor/SupervisorView.tsx:195`; `crates/cockpit-core/src/orchestration/projection.rs:418-430`). Blocked without question uses separate warning/Open terminal/Check status (`src/app/supervisor/SupervisorView.tsx:289`). Runtime done without Result displays “Runtime Done · no result reported” (`src/app/supervisor/SupervisorView.tsx:260-261`).

### Assignment intent resolution; no creation form in frozen slice

Assignment/acceptance intents are filtered for selected root (`src/app/supervisor/SupervisorView.tsx:202-203`). Panel shows pending or “Task changed elsewhere · Not assigned,” canonical task, and conflict choices **Assign current task** at current revision / **Keep unassigned**. Assign is disabled busy/not-live/no canonical/diagnostic/closed root; keep is disabled busy/disconnected (`src/app/supervisor/SupervisorView.tsx:291-297`). Protocol rejects assign=true with null expected revision (`src/client/orchestrationProtocol.ts:155-164`). No task-assignment composer appears in frozen UI inventory.

### Acceptance conflict and Result review

Reported Result is not auto-accepted. Reported-stage operator action is disabled busy/not-live/task absent/diagnostic/current-run mismatch and submits exact task revision (`src/app/supervisor/SupervisorActions.tsx:184`). Conflict panel offers **Apply acceptance to current task** (busy/not-live guard) and **Keep current task unchanged** (busy/disconnected guard) (`src/app/supervisor/SupervisorView.tsx:203,298`). Core apply requires Reported plus unchanged explicit successful Result/message identity, checks canonical task revision and then accepts/annotates or returns conflict (`crates/cockpit-core/src/orchestration.rs:997-1044`). SendBack/CancelRun are gated while acceptance intent exists (`crates/cockpit-core/src/orchestration.rs:259-275`).

### Orphaned workers

View defines orphaned workers as nonclosed descendants under closed roots (`src/app/supervisor/SupervisorView.tsx:87-88`). With no selected root it shows **Worker agents need control**, says parent close did not stop them and renders per-worker recovery plus **View saved task context**. Closed selected root with active descendants warns they remain open and need supervision; each gets recovery (`src/app/supervisor/SupervisorView.tsx:285-286`). Archive says tasks/history remain; close did not kill agents/remove resources (`src/app/supervisor/SupervisorView.tsx:279`). Close dialog/core preserve descendants (`src/app/supervisor/SupervisorDialogs.tsx:98`; `crates/cockpit-core/src/orchestration.rs:854-881`).

## Source map

| Explicit source range | Responsibility |
|---|---|
| `planning/supervisor-atlas-2026-10-06/evidence/SOURCE.md` (captured 2026-10-06T19:26:10.601Z) | Frozen UI source evidence; UI line citations below use original file lines at capture. |
| `src/app/supervisor/SupervisorDialogs.tsx:10-107` | Six modes, action construction, validation, confirmations, pending/unknown and focus behavior. |
| `src/app/supervisor/SupervisorActions.tsx:9-46,90-104,139-188` | Agent/task state, recovery card, subagent controls, result review. |
| `src/app/supervisor/SupervisorView.tsx:59-117,137-210,226,249-261,278-299,339-341` | Root focus, start/restart/recovery, attention/conflicts/orphan surfaces and modal availability. |
| `crates/cockpit-core/src/orchestration/projection.rs:8-174,173-222,371-505` | Durable/runtime join, task lanes and attention derivation. |
| `crates/cockpit-core/src/orchestration.rs:259-275,854-980,997-1044` | Acceptance conflict fences; close/retry/reconcile/accept-resolution semantics. |
| `crates/cockpit-protocol/src/orchestration.rs:29-54,91-149,168-206` | Snapshot, run, task, dispatch and protocol type definitions. |
| `src/client/orchestrationProtocol.ts:37-40,53-62,73-109,130-151,155-214` | Recovery/target/action/snapshot validation. |
| `src/app/supervisor/SupervisorView.test.tsx:262-278,298-362,363-389` | Fixture test code only; not runtime evidence. |

## Semantic citation audit

The correction pass reviewed prose, state tables and source-map entries against original UI coordinates, with core/protocol claims reviewed separately in their named implementations. Start validation targets Dialog49 (available Space),52–53 (absolute directory/request),55 (action label),89–90 (dedicated/no-focus copy),43–44/105 (availability guard/control).

The integration owner’s [literal-verified ledger](evidence/CITATION-AUDIT.md) records two exact source substrings for each R ID. Backend lifecycle behavior remains code-derived rather than freshly exercised.