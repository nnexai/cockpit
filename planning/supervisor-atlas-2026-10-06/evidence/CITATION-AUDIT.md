# Supervisor atlas citation audit

Correction pass after supervisor review. This is source verification, not a new runtime check. The earlier regex bounds check missed bare-number source-map cells and could not establish semantic alignment; it was insufficient and its prior blanket claim is superseded.

## Coordinates

The fenced source blocks are stripped of Markdown headings/fences before indexing. `source line 1` is the first code line, never the snapshot Markdown line. Examples: SupervisorView Markdown8→source1; Actions354→source1; Dialogs549→source1; Graph661→source1; CSS973→source1; App snapshot Markdown8→source1. Frozen View has341 source lines; source348 is invalid even though Markdown348 contains its closing brace.

## Exact literal verification

The integration owner inspected the source predicates/handlers represented below. A throwaway in-memory check located **116 literal substrings for all58 stable surface IDs** in the captured original-source strings and derived the line coordinates directly from the matched strings. Each table anchor is a literal substring, not pseudocode or a fabricated replacement statement. The proof below establishes the principal rendering/interaction branch for each ID; it is not an assertion that text matching alone proves every behavior. The four slice correction audits additionally reviewed prose, conditional tables, source maps and interaction/coverage claims for semantic alignment.

| ID | Original source coordinate | Exact source substring |
|---|---|---|
| W1 | `App.tsx:240` | `onClick={onSupervisor}` |
| W1 | `App.tsx:1190` | `supervisorMounted && state.sessionId` |
| W2 | `SupervisorView.tsx:276` | `openRoots.length > 1` |
| W2 | `SupervisorView.tsx:279` | `aria-label="Closed tracking"` |
| W3 | `SupervisorView.tsx:282` | `aria-label="Supervisor status"` |
| W3 | `SupervisorView.tsx:226` | `const hasAttention =` |
| W4 | `SupervisorView.tsx:317` | `taskLanes.map` |
| W4 | `SupervisorView.tsx:318` | `Show completed tasks` |
| W5 | `SupervisorView.tsx:249` | `const needsAttention =` |
| W5 | `SupervisorView.tsx:309` | `Needs attention (dims other tasks)` |
| W6 | `supervisor.css:155` | `@container (max-width: 719px)` |
| W6 | `supervisor.css:197` | `@media (max-height: 600px)` |
| W7 | `SupervisorView.tsx:296` | `Task assignment pending` |
| W7 | `SupervisorView.tsx:294` | `task_assignment_resolve` |
| W8 | `SupervisorView.tsx:281` | `Loading Supervisor…` |
| W8 | `SupervisorView.tsx:323` | `Start an agent to manage your tasks.` |
| W9 | `SupervisorView.tsx:234` | `const closeDetail =` |
| W9 | `SupervisorView.tsx:269` | `event.nativeEvent.isComposing` |
| G01 | `SupervisorGraph.tsx:81` | `const connectedCount =` |
| G01 | `SupervisorGraph.tsx:85` | `Agents · {fresh` |
| G02 | `SupervisorGraph.tsx:30` | `const fresh =` |
| G02 | `SupervisorGraph.tsx:31` | `const observations =` |
| G03 | `SupervisorGraph.tsx:86` | `checked={scope.showSubagents}` |
| G03 | `SupervisorGraph.tsx:91` | `roving.focusRow(parent ?? undefined)` |
| G04 | `SupervisorGraph.tsx:32` | `const unmanaged =` |
| G04 | `SupervisorGraph.tsx:95` | `checked={scope.showUnmanaged}` |
| G05 | `SupervisorView.tsx:104` | `const forest =` |
| G05 | `SupervisorView.tsx:104` | `row.run.stage !== "closed"` |
| G06 | `SupervisorGraph.tsx:38` | `row.subagent.parent_subagent_id` |
| G06 | `SupervisorGraph.tsx:112` | `const role =` |
| G07 | `graphLayout.ts:6` | `const CARD_WIDTH` |
| G07 | `graphLayout.ts:7` | `const CARD_HEIGHT` |
| G08 | `SupervisorGraph.tsx:42` | `snapshot.board?.root_id !== row.run.root_id` |
| G08 | `SupervisorGraph.tsx:121` | `aria-label="Worker task references"` |
| G09 | `SupervisorGraph.tsx:106` | `const rawStatus =` |
| G09 | `SupervisorGraph.tsx:114` | `const evidence =` |
| G10 | `SupervisorGraph.tsx:113` | `const space =` |
| G10 | `SupervisorGraph.tsx:117` | `space ?? (row.subagent ? "no terminal" : "unobserved")` |
| G11 | `SupervisorGraph.tsx:115` | `aria-expanded={selected}` |
| G11 | `SupervisorGraph.tsx:115` | `onClick={() => onSelect(row.run, row.subagent)}` |
| G12 | `SupervisorGraph.tsx:51` | `const highlightedIds =` |
| G12 | `SupervisorGraph.tsx:58` | `const highlightedEdges =` |
| G13 | `SupervisorGraph.tsx:71` | `rowIds: [...visibleRows.map` |
| G13 | `SupervisorGraph.tsx:75` | `scope.selectedSubagent = null;` |
| G14 | `SupervisorGraph.tsx:132` | ``data-row-id={`other:${agent.pane_id}`}`` |
| G14 | `SupervisorGraph.tsx:128` | `Unmanaged · no task relationships` |
| G15 | `SupervisorGraph.tsx:120` | `No agents in this task scope.` |
| G15 | `SupervisorGraph.tsx:137` | `Other agents are unobserved while the connection is unavailable.` |
| G16 | `RowSplitter.tsx:24` | `setPointerCapture` |
| G16 | `RowSplitter.tsx:40` | `ArrowUp` |
| G17 | `supervisor.css:155` | `@container (max-width: 719px)` |
| G17 | `supervisor.css:197` | `@media (max-height: 600px)` |
| G18 | `supervisor.css:216` | `.supervisor-view.has-attention .supervisor-graph-band { height: 100px; }` |
| G18 | `supervisor.css:63` | `.supervisor-graph-node-heading strong` |
| G19 | `SupervisorView.tsx:276` | `openRoots.length > 1` |
| G19 | `SupervisorView.tsx:279` | `View {summary.label} tasks and history` |
| D01 | `SupervisorView.tsx:332` | `aria-label="Detail sections"` |
| D01 | `SupervisorView.tsx:331` | `className="supervisor-detail-header"` |
| D02 | `SupervisorActions.tsx:152` | `Task description` |
| D02 | `SupervisorActions.tsx:164` | `No worker progress reported yet.` |
| D03 | `SupervisorActions.tsx:51` | `Reported · {source}` |
| D03 | `SupervisorActions.tsx:57` | `Observed · Herdr` |
| D04 | `SupervisorActions.tsx:163` | `Open parent terminal` |
| D04 | `SupervisorActions.tsx:158` | `No summary reported.` |
| D05 | `SupervisorActions.tsx:170` | `Awaiting review` |
| D05 | `SupervisorActions.tsx:172` | `Initialization report` |
| D06 | `SupervisorActions.tsx:168` | `A stored request is not applied control.` |
| D06 | `SupervisorActions.tsx:168` | `No control receipt. No child terminal is assumed.` |
| D07 | `SupervisorActions.tsx:176` | `Edit task…` |
| D07 | `SupervisorActions.tsx:176` | `disabled={busy \|\| !live \|\| !!task.task.diagnostic}` |
| D08 | `SupervisorActions.tsx:137` | `Ask the supervisor to stop this task?` |
| D08 | `SupervisorActions.tsx:133` | `this is not proof the process stopped.` |
| D09 | `SupervisorActions.tsx:179` | `Message to subagent` |
| D09 | `SupervisorActions.tsx:179` | `Control request stored. Check its receipt for applied delivery.` |
| D10 | `SupervisorActions.tsx:180` | `Cancel subagent…` |
| D10 | `SupervisorActions.tsx:179` | `subagent.status !== "running"` |
| D11 | `SupervisorActions.tsx:182` | `Follow-up to agent` |
| D11 | `SupervisorActions.tsx:182` | `Follow-up sent. Waiting for the agent.` |
| D12 | `SupervisorActions.tsx:183` | `Durable note` |
| D12 | `SupervisorActions.tsx:183` | `Note saved.` |
| D13 | `SupervisorActions.tsx:185` | `Close agent tracking…` |
| D13 | `SupervisorActions.tsx:185` | `onCloseTracking(run)` |
| D14 | `SupervisorActions.tsx:184` | `expected_task_revision: task.task.task_revision` |
| D14 | `SupervisorActions.tsx:184` | `Accept explicit result` |
| D15 | `SupervisorActions.tsx:111` | `const stale = reviewed !== plan.plan_revision` |
| D15 | `SupervisorActions.tsx:114` | `Reviewed current plan` |
| D16 | `SupervisorActions.tsx:114` | `Optional execution note` |
| D16 | `SupervisorActions.tsx:114` | `note: note.text \|\| null` |
| D17 | `SupervisorView.tsx:204` | `const history =` |
| D17 | `SupervisorView.tsx:333` | `No recorded history in this scope.` |
| D18 | `SupervisorActions.tsx:119` | `Run, binding, setup and exact plans` |
| D18 | `SupervisorActions.tsx:120` | `Inbox delivery and provenance` |
| D19 | `SupervisorView.tsx:334` | `assignment_intents: snapshot.assignment_intents` |
| D19 | `SupervisorView.tsx:334` | `Fresh runtime and transaction records` |
| D20 | `SupervisorView.tsx:279` | `aria-label="Closed tracking"` |
| D20 | `SupervisorView.tsx:279` | `Return to {openRoots[0].label}` |
| D21 | `SupervisorActions.tsx:16` | `Cannot check the agent right now` |
| D21 | `SupervisorActions.tsx:35` | `No fresh, bound OMP process is confirmed.` |
| D22 | `SupervisorView.tsx:91` | `initialRootSnapshot.current = snapshot` |
| D22 | `SupervisorView.tsx:116` | `snapshot === initialRootSnapshot.current` |
| R01 | `SupervisorDialogs.tsx:49` | `Choose a currently available Space.` |
| R01 | `SupervisorDialogs.tsx:52` | `Enter an absolute directory path.` |
| R02 | `SupervisorDialogs.tsx:59` | `expected_task_revision: dialog.draft.revision` |
| R02 | `SupervisorDialogs.tsx:94` | `Task changed elsewhere.` |
| R03 | `SupervisorDialogs.tsx:62` | `retry_launch` |
| R03 | `SupervisorDialogs.tsx:96` | `could leave another agent running.` |
| R04 | `SupervisorDialogs.tsx:98` | `This does not guarantee the agent or its workers stop.` |
| R04 | `SupervisorDialogs.tsx:105` | `Keep tracking` |
| R05 | `SupervisorDialogs.tsx:61` | `action: "reconcile_run"` |
| R05 | `SupervisorDialogs.tsx:100` | `Confirm using the existing worktree receipt` |
| R06 | `SupervisorDialogs.tsx:60` | `op: { op: "cancel" }` |
| R06 | `SupervisorDialogs.tsx:99` | `not this request, confirms whether it stopped.` |
| R07 | `SupervisorActions.tsx:12` | `const restartable =` |
| R07 | `SupervisorActions.tsx:101` | `run.stage !== "reported" && run.dispatch` |
| R08 | `SupervisorView.tsx:298` | `Task changed during acceptance` |
| R08 | `SupervisorView.tsx:267` | `aria-label="Needs you"` |

## Review corrections

- Graph source map no longer ends at original View348. Current root select is View276; archive is279; graph mount305 and outer view264.
- App toolbar and mount citations use original App240 and1190, not snapshot rows247/1197.
- Empty-root copy is View323; dialog render is339; selected detail remount is332.
- Root-selector predicates, scoped history, narrow focus transfer and modal validation references were checked against actual code branches, not just file bounds.
- Subagent graph nodes can display the parent observed Space; “no terminal” is the fallback when Space is absent, not an unconditional metadata label. Child role uses the supplied role or OMP subagent, not Supervisor. Unmanaged roving IDs include `other:`.
- An archived closed root does not imply an empty graph: its own node is filtered, but open descendants with the same root remain eligible.

Backend/protocol and the shared roving hook are outside the frozen Supervisor/App snapshots: their cited implementations were reviewed separately by the owning slice; lifecycle authority remains code-derived, not freshly exercised backend behavior. The main atlas enum range `src/protocol/generated/v1.ts:951-1019` was read and confirmed to contain task/run/dispatch/report/delivery/subagent/presence/runtime/attention definitions.

## Final mechanical checks

Seven atlas/audit documents were checked for explicit source-reference ranges against the original captured Supervisor/App code strings, not Markdown rows: **664 range occurrences**, zero out-of-bounds ranges, zero ambiguous shorthand citations and zero bare-number source-map cells. Comma lists and Unicode range dashes were included. Markdown link targets were checked for existence with zero missing targets. All116 literal anchor substrings in the table were found at the stated original source lines. These checks support the semantic review above; they do not turn text matching into runtime or lifecycle proof.
