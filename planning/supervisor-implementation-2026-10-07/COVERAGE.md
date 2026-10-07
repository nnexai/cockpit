# Coverage and verification map: F1–F13 and all 22 scenarios

This file is the companion to [`PLAN.md`](PLAN.md). Slice IDs S1–S13, decisions D1–D17, guards G1–G17 and contracts C1–C13 refer to PLAN.md. "View test" means a test added or updated in `SupervisorView.test.tsx` by S8h. Unit tests live in the slice-owned files named in PLAN §6.

Evidence classes:

- **unit**: Vitest jsdom with the DTO harness.
- **LIVE**: a disposable fixture with real Cockpit, Herdr and OMP (PLAN §7.2).
- **SYN**: the labelled synthetic DTO browser harness with the real `SupervisorView` and a fake client (PLAN §7.3).
- **scan**: a literal grep (PLAN §7.4).

Nothing below has been executed yet.

Parent review released implementation at inbox sequence 12. All closed-root task counts remain required: bounded archive-on-open snapshots with session/root/generation fences and honest loading/unavailable states; no fabricated closure date. Root selector uses Decide-derived need-you counts and accurate Recover indication. Permanent tests below are limited to consumer-visible behavioral boundaries/transitions/authority; incidental label/default/source/wiring checks use browser or throwaway evidence. Available LSP references precede export changes. Builds use isolated output until final shared integration is coordinated; preserve Notes ownership and widget fix `b538420`.


## 1. F1–F13: every accepted recommendation, mapped to the slices that deliver it

### F1 Attention queue (DESIGN:123-139)

| Recommendation | Slice | Automated check | Browser |
|---|---|---|---|
| Summary bar on one line: glyph, root, state, observed line, tier counters; zero counts show status plus Open terminal | S9, S8b | `SupervisorAttention.test` counters; View test "summary with zero counts shows Open terminal shortcut" | LIVE L1 |
| Queue only when items exist; order Decide→Recover→Notice, then `since` | S1, S9 | `attention.test` ordering; `SupervisorAttention.test` order preserved | SYN attention-mix |
| Each row: tier glyph, one-line problem naming the task or agent, one primary action, age | S9, S8b | `SupervisorAttention.test`; View test row titles | SYN attention-mix |
| Exactly one row expanded; first Decide row expanded by default | S9, S8b (D16) | `SupervisorAttention.test` single-expand; View test default expansion | SYN |
| Height `min(content, 40%)` (30% in Graph), then scroll with an "n more" row; never clips mid-row | S3 `queueCap`, S9 | `useSupervisorLayout.test` cap; `SupervisorAttention.test` "n more" | SYN 1440×900 and 760×900 (scenario 1) |
| Decide row embeds the literal question and `TextAction` exactly as today (G1) | S8b | existing `:359-390` kept; View test Decide row = core `needs_input` on root | LIVE L2–L3 |
| Graph view keeps the summary bar and queue; Show in Graph / Show in Tasks on every row | S8b, S8d | View test showIn label per view | SYN, LIVE L5 |
| Keyboard: roving Up/Down/Home/End; Enter/Space toggle; Escape collapses, then layered Escape; focus back to task or Start when the question goes; expansion never steals focus from a field | S9, S8d | `SupervisorAttention.test`; View test Escape layering; existing `:270-285` | LIVE L8 |
| Responsive: inline at ≥720 and >600 high, else counters open the overlay; Escape returns to the counter | S3, S8b | View test with `mockWorkareaSize(360,640)` | LIVE L8; SYN 360×800 |
| Authority: Answer, Assign/Keep and Apply/Keep keep exact revisions and disabled rules (G1–G3) | S8b | View tests asserting action JSON and disabled states | — |

### F2 One attention vocabulary and predicate (DESIGN:141-159)

| Recommendation | Slice | Automated check | Browser |
|---|---|---|---|
| Decide = unanswered root question only; worker questions are supervisor-owned | S1 (D2) | `attention.test` root vs worker `needs_input` | SYN |
| Recover kinds: dispatch, setup, review unknown; terminal gone; endpoint changed; runtime-blocked without question; exited; orphans; start unknown; navigation or terminal errors | S1 | `attention.test` tier table | SYN |
| Notice: assignment and acceptance conflicts, unidentified items, stale evidence (summary banner and Activity label), notices, idle/brief/plan-changed | S1 (D2) | `attention.test` | SYN |
| One TS model feeds queue, card badge, node badge and filter; core `snapshot.attention` is the source for core kinds; local conditions stay local and deduplicated | S1, S8b (D1, D3) | `attention.test` dedupe; View test blocked worker: Recover badge plus queue row plus not dimmed | SYN attention-mix |
| Badge reads Decide/Recover/Notice with glyph, never colour-only | S11, S6 | `SupervisorTasks.test`, `SupervisorGraph.test` | SYN |
| Filter = toggle chip `Attention · N` that dims, not removes | S8b | View test chip `aria-pressed` plus dim classes | LIVE L8 |
| `awaits_prepare`, `awaits_execute` and `to_accept` shown as ordinary state with age; no new timeout | S1, S5, S8e | `attention.test` never items; `SupervisorActions.test` state block "Waiting for supervisor" or "Supervisor is reviewing this result" with `since` | SYN review-result |
| Badge shortens at ≤479 px; full name stays in the accessible label | S8g, S11 | `SupervisorTasks.test` accessible name | SYN 360×800 |

### F3 Task-first work area and full-workarea Graph (DESIGN:161-244)

| Recommendation | Slice | Automated check | Browser |
|---|---|---|---|
| Segmented `Tasks n` / `Graph n` with `aria-pressed`, at every size, in a view bar with Attention and Space (replaces the "Tasks · n open" row) | S8c | View test labels and counts | LIVE L1 at four sizes |
| Tasks initial per root; choice remembered in `ScopeDrafts.view.mode`; switching never changes selection, filters or details | S8a, S8d (C13) | View test: per root, hide/return, first open Tasks | LIVE L1 |
| Tasks view: Board owns height; strip at ≥720 (`Agents · n observed`, chips attention-first, `+N subagents`, `+k more`, `Graph ›`); chips only select | S11, S8c | `SupervisorTasks.test`; View test chip selects run with no `onTerminal` | LIVE L1 |
| Graph view fills the workarea; no band; no graph-height splitter | S8a, S8g | View test no "Resize agents overview" | LIVE L1 |
| Columns supervisor→task→worker→subagent→nested; sticky headings; deterministic forest kept | S2, S6 (D5, D6) | `topology.test`, `graphLayout.test` | SYN topology-full |
| Nodes 240×48 with two rows and the five regions; document icon; dashed subagents; dotted unassigned | S2, S6, S8g | `graphLayout.test` geometry; `SupervisorGraph.test` regions | SYN scenario 21 |
| Edges: thin supervisor→task, solid assigned, dashed subagent; selected chain accent | S6 (`chainIds`) | `SupervisorGraph.test` edge classes | SYN |
| Unassigned at the bottom after a 0.4 gap; done tasks hidden with `n completed tasks hidden` | S2, S6 | `graphLayout.test` static figure; `SupervisorGraph.test` heading | SYN |
| Two-axis scroll, natural size, totals heading, `Agents · unobserved` | S6, S8g | `SupervisorGraph.test` | LIVE L4 |
| Selection mapping task/run/subagent, mutually exclusive, reset to Overview | S8d | existing selection tests plus View test node kinds | — |
| Details side panel 340/280/50% with splitter; sheet 50%/160/75% with spacer; overlay below 560 high; Tasks narrow overlay | S3, S8e (D9) | `useSupervisorLayout.test`; View test placements | LIVE L4; SYN 360×800 |
| Reveal only for outside origins or resize-covered nodes; a click on a visible node never scrolls | S3, S8d (D11) | `reveal.test`; View test no `scrollTop` change on click | LIVE L4 |
| Attention in Graph: counters, inline 30% or overlay, node tier badges, dim, Show in Graph; never auto-switches view or focus | S6, S8b | View test "attention appearing keeps view and focus" | SYN attention-mix |
| Return to Tasks restores Board scroll, lane state, filters and selected card (revealed) | S8d | View test with stubbed `scrollTop` values | LIVE L8 |
| Empty/offline/loading: "No worker agents yet", hollow unobserved, single banner, retry-load unchanged | S6, S8f | `SupervisorGraph.test`; View test offline | LIVE L6 |
| Keyboard: switch is two tab stops, no focus move; graph one roving stop with ↑↓ Home End ← parent → first child; Escape closes details with focus back to node and never changes view | S6, S8d | `SupervisorGraph.test`; View test Escape | LIVE L5 |
| Authority: observes only; Open terminal is the only Herdr focus changer (G14) | S6, S8 | `SupervisorGraph` has no terminal prop; View tests `onTerminal` not called | LIVE L5 |

### F4 Review-first details (DESIGN:246-268)

| Recommendation | Slice | Automated check | Browser |
|---|---|---|---|
| Keep three segments and Escape/focus behaviour | S5, S8e | existing `:401-434,435-493` | — |
| State block (responsibility sentence, tier, reported/observed pair) | S5, S8e | `SupervisorActions.test` | SYN review-result |
| Result in Overview: outcome, summary, reporter, time, Awaiting review/Accepted | S5 | `SupervisorActions.test` | SYN |
| Progress trail Assigned→Prepared (grant origin)→Executing→Result→Accepted | S5 | `SupervisorActions.test` supervisor vs operator origin | SYN |
| Routine group; Operator intervention collapsed, auto-open when armed or on result-review conflict | S5 | `SupervisorActions.test` | SYN; scenario 5 |
| Duplicate headings removed | S5 | `SupervisorActions.test` single label | — |
| Subagent details: role, parent, OMP events, receipts stored/applied/failed, "no terminal of its own" once | S5 | `SupervisorActions.test` | SYN subagent-path |
| Responsive: side ≤50% (D9 overrides "42%"); narrow overlay first screen State + Result | S8e, S8g | View test narrow overlay order | LIVE L8 |
| Authority G5 accept guard unchanged | S5 | `SupervisorActions.test` payload and disables | — |

### F5 Recovery cards (DESIGN:270-286)

| Recommendation | Slice | Automated check | Browser |
|---|---|---|---|
| Primary = safest step (Check status/connection; Recover setup…; Retry setup) | S5 | `SupervisorActions.test` per state | SYN recovery-states |
| Restart secondary with muted "May open another terminal"; disabled reason; `aria-disabled` with handler guard (D13) | S5 | `SupervisorActions.test` click on aria-disabled does nothing | SYN |
| Close tracking off the healthy row; quiet text on non-ready rows; stays in Operator intervention | S5 | `SupervisorActions.test` | SYN; LIVE L7 |
| Orphaned workers as queue rows with Open terminal and View saved task context | S1, S8b | View test closed root with descendants | SYN closed-root |
| ≤479 icon-only Close tracking keeps its accessible name | S8g | View test accessible name | SYN 360×800 |
| Authority G8: reconcile first, guarded retry dialog | S5, S8 | existing `:341-358` kept | — |

### F6 Card density (DESIGN:288-304)

| Recommendation | Slice | Automated check | Browser |
|---|---|---|---|
| Two-line card; status only when it adds to the lane | S11 | `SupervisorTasks.test` | LIVE L1, L8 |
| Paired chips `reported <age>` / `observed <glyph> word` with absolute time in title and name | S5 `ProvenancePair`, S11 | `SupervisorActions.test`, `SupervisorTasks.test` | SYN |
| Space chip only when it differs from the root Space or a filter is active; shared highlight kept | S8c, S11 | View test | SYN |
| Blocked warning moves to the queue; card shows Recover badge | S11, S8b | `SupervisorTasks.test` no buttons; View test | SYN attention-mix |
| Card stays one roving item with `aria-expanded`; name includes title, state, tier | S11, S8c | `SupervisorTasks.test`; existing `:435-493` | — |
| Explicit failed report stays visible (existing invariant) | S11 | existing `:169-178` kept | — |

### F7 Relationship path (DESIGN:306-322)

| Recommendation | Slice | Automated check | Browser |
|---|---|---|---|
| Path rows Task→Worker→Subagent with provenance; buttons; `aria-current` | S2 `pathNodes`, S5, S8e | `topology.test`, `SupervisorActions.test` | SYN subagent-path |
| `+N subagents` chip on cards | S11 | `SupervisorTasks.test` | SYN |
| Same chain as graph columns; Task links list removed | S2, S6 | `SupervisorGraph.test` no "Task links" | SYN |
| Outside selection reveals node and highlights chain | S8d, S6 | View test | LIVE L5 |
| `Herdr` / `OMP events` provenance on nodes; dashed subagent; "no terminal" in visible metadata | S2 `nodeFacts`, S6 | `SupervisorGraph.test` | SYN |
| Path row keeps focus; details Show in… moves DOM focus (not Herdr) | S8d | View test | LIVE L5 |
| No cross-tree authority (rows only select) | S5 | `SupervisorActions.test` `onActivate` only | — |

### F8 Dialogs (DESIGN:324-339)

| Recommendation | Slice | Automated check | Browser |
|---|---|---|---|
| Shared portal, focus restore, Tab wrap, Escape when idle kept | S7 | `SupervisorDialogs.test` | SYN dialogs |
| R01 inline field errors; destination summary line; disabled Space select with reason | S7 | `SupervisorDialogs.test` | SYN |
| R02 side-by-side stale with Keep my draft / Use current; deleted → Save disabled plus Copy draft (G12) | S7 | `SupervisorDialogs.test` | SYN; scenario 8 |
| R03 "Last observed" line; Check again first (read-only); focus on Back | S7, S8e (`onCheck`) | `SupervisorDialogs.test` no `retry_launch` | SYN |
| R04 up to three descendant names plus "and N more"; focus Keep tracking | S7 | `SupervisorDialogs.test` | SYN |
| R05 primary label by recovery kind | S7 | `SupervisorDialogs.test` | SYN |
| R06 Keep running / Request cancellation | S7 | `SupervisorDialogs.test` | SYN |
| Inline confirms follow "Keep …" (plan override "Keep supervisor decision") | S5 | `SupervisorActions.test` | — |
| Width `min(460px, 100vw−24px)`; footer wraps at ≤479; error slot collapses when empty; payloads unchanged | S7, S8g | `SupervisorDialogs.test` payloads | SYN 360×800 |

### F9 Header (DESIGN:341-355)

| Recommendation | Slice | Automated check | Browser |
|---|---|---|---|
| Start agent secondary when a verified open root exists, else primary; "Start options…" text at ≥720 | S8f | View test classes | LIVE L1 |
| Hide Supervisor label and title | S8f | View test | LIVE L8 |
| Selector `label · N need you` plus recover marker (Q-A default) | S1, S8b | `attention.test` `rootAttentionSummary`; View test with two roots | SYN |
| One name: Activity (button, region, heading) | S8f, S10 | View test (updated `:494-510`) | LIVE L8 |
| Header order per DESIGN:353; no new shortcuts | S8f | View test order | — |

### F10 Activity, Diagnostics, archive (DESIGN:357-370)

| Recommendation | Slice | Automated check | Browser |
|---|---|---|---|
| Activity grouped by day; actor chip · what · task/run link · age; supervisor grants as rows; stale label | S10, S8e | `SupervisorActivity.test` | LIVE L8 |
| Diagnostics: summary table first (path with copy, board diagnostics, per-run dispatch/observation/binding/delivery), raw records after; banner on top | S10 | `SupervisorActivity.test` | SYN |
| Closed tracking rows (Q-B default) with existing action; explanation once | S10, S8e | `SupervisorActivity.test` | SYN closed-root |
| Focus as today (wide panel keeps focus on the toggle; narrow overlay → Close) | S8d | View test | — |
| Stored ≠ woken ≠ read ≠ acked wording | S10 | `SupervisorActivity.test` | — |

### F11 Empty, offline, loading (DESIGN:372-384)

| Recommendation | Slice | Automated check | Browser |
|---|---|---|---|
| One offline banner in summary bar; hollow glyph plus "unobserved"; no per-card "Saved reports only" | S5, S8f | `SupervisorActions.test`; View test offline | LIVE L6; SYN offline |
| Empty root board: one centered heading plus Open terminal; no paragraph (A9) | S8c | View test | SYN empty-root |
| Unassigned task: single status line | S11 | `SupervisorTasks.test` | — |
| Loading and retry-load unchanged; empty state has one tab stop | S8 | existing `:201-206` plus View test | — |

### F12 Narrow and short (DESIGN:386-403)

| Recommendation | Slice | Automated check | Browser |
|---|---|---|---|
| Summary bar plus counters as the only status element | S8b | View test narrow | LIVE L8 |
| Lane list: `details` per lane, non-empty open, empty one line, all six counts | S8c, S4 | View test with `mockWorkareaSize(360,640)` | LIVE L8 |
| Up/Down crosses lanes when stacked | S4 | `boardNavigation.test` | LIVE L8 |
| Tasks · Graph switch at every size; overlay from summary | S8c | View test | LIVE L1 |
| Details overlay first screen State plus Result | S5, S8e | View test | LIVE L8 |
| ≤600 high: summary bar plus one surface, compact header | S8g | — | LIVE L8 at 1440×600 |
| Graph narrow: canvas plus sheet (≥560 workarea) else overlay; selected node never hidden; close returns to node | S3, S8e | View test | LIVE L4 at 360×800; SYN |
| Short wide: switch and filters on one line; queue overlay; side panel | S8g, S3 | — | LIVE L8 (scenario 20) |

### F13 Keyboard and accessibility (DESIGN:405-415, §6)

| Recommendation | Slice | Automated check | Browser |
|---|---|---|---|
| Details splitter `separator` with orientation, `aria-valuemin/max/now/valuetext`, label, controls; Arrow ±16/48; Home reset | S3 | `PanelSplitter.test` | LIVE L4 |
| Dimming keeps full text opacity with lowered background; "dimmed by …" in name | S11, S6, S8g (D14) | `SupervisorTasks.test`, `SupervisorGraph.test` | LIVE L8 (computed contrast) |
| Disabled reasons (muted line plus `aria-describedby`) | S5, S7 (D13) | unit tests | — |
| Graph ← parent → first child, Graph view only | S6 | `SupervisorGraph.test` | LIVE L5 |
| Wide Activity/Diagnostics keep focus; announce via `role=status` notice | S8d | View test notice text | — |
| Glyph always paired with a word | S6, S11, S9 | unit tests | SYN |
| Queue roving plus polite count in summary only | S9 | `SupervisorAttention.test` | — |
| Landmarks and names (§6 list); view-change announcement "Graph view · N agents · M tasks" | S8 | View test region names and notice | — |

## 2. Intentional removals stay removed (A9)

| Removal | Guard |
|---|---|
| Bottom task composer and its resize handle | existing assertions `:262-267` kept |
| "Other agents" filter and unmanaged overview | existing `:229-231` adapted; `unmanaged_agents` not rendered |
| Empty-state paragraph | View test asserts heading only |
| Tooltip focus sentence | View test Start agent `title` is destination only |

## 3. All 22 acceptance scenarios (DESIGN:496-519)

| # | Scenario | Slices | Automated (unit / View test) | Browser path | Evidence |
|---|---|---|---|---|---|
| 1 | Clipping: question, failed worker and assignment conflict at 1440×900 and 760×900; no row cut; Board keeps ≥50% height | S9, S3, S8b, S8g | `SupervisorAttention.test` "n more" and cap | SYN attention-mix at both sizes: DOM checks row rect inside queue scroll content, `board.height ≥ 0.5·workarea` | SYN plus unit |
| 2 | Blocked worker without question → Recover badge, queue row, not dimmed; Review task with Result and active supervisor in neither Decide nor Recover | S1, S11, S8b | `attention.test`; View test with core `runtime_blocked` and `to_accept` entries | SYN attention-mix | unit plus SYN |
| 3 | Answering clears the Decide row; focus to selected or first task; draft survives when unconfirmed | S8b | existing `:359-390` plus View test (core entry removed after the answer) | LIVE L2–L3 (answer path) | LIVE plus unit |
| 4 | Initial wide: summary bar, Tasks pressed, Graph visible, strip, lanes, no canvas; collapse/reopen keeps view per root; first open Tasks | S8a, S8c, S11 | View test | LIVE L1 at 1440×1000 | LIVE plus unit |
| 5 | Review task with Result: State, Result, trail, Routine; Operator collapsed; Accept disabled under diagnostic or mismatch; Plan override only at awaiting_prepare/ready | S5, S8e | `SupervisorActions.test` | SYN review-result | unit plus SYN |
| 6 | Subagent path with provenance and "no terminal"; activating a row selects without terminal focus | S2, S5, S8d | `topology.test`, `SupervisorActions.test`; View test `onTerminal` not called | SYN subagent-path; LIVE L5 focus triple if a live subagent exists | unit plus SYN (plus LIVE) |
| 7 | Missing terminal: Check status primary, Restart with consequence, Close tracking quiet; healthy row Open terminal only | S5, S8b | `SupervisorActions.test` | SYN recovery-states; LIVE L7 | unit plus SYN plus LIVE |
| 8 | Dialog labels; recovery-kind primary; deleted edit Save disabled; Escape idle only; Tab wraps; opener refocus | S7 | `SupervisorDialogs.test`; existing `:401-434` | SYN dialogs (keyboard) | unit plus SYN |
| 9 | 360×640: summary bar, six counts without horizontal scroll, overlay focus Close, Escape to chip, details first screen State plus Result | S4, S8b, S8c, S8e, S8g | View tests with `mockWorkareaSize(360,640)` | LIVE L8 | LIVE plus unit |
| 10 | Offline: one banner, hollow plus "unobserved", no "Saved reports only", Start/Check semantics | S5, S8f | View test (existing `:251-269` modes extended) | LIVE L6; SYN offline | LIVE plus SYN |
| 11 | Board arrows move focus only; splitter value attributes; dimmed readable | S4, S3, S8g | `boardNavigation.test`, `PanelSplitter.test`, existing `:435-493` | LIVE L8 (computed contrast ratio recorded) | unit plus LIVE |
| 12 | Activity in button, region, heading; Hide Supervisor | S8f, S10 | View test | LIVE L8 | LIVE plus unit |
| 13 | Switch visible without scrolling at 1440×1000, 760×900, 360×800, 1440×600; Tasks pressed first; Graph shows whole graph | S8c, S8g | View test | LIVE L1 (four sizes) | LIVE |
| 14 | Full topology: supervisor, every open task, every worker and subagent incl. nested; heading counts equal nodes; reachable by scroll; sticky headings | S2, S6 | `topology.test`, `SupervisorGraph.test` | SYN topology-full (exhaustive); LIVE L4 (real forest) | unit plus SYN plus LIVE |
| 15 | Worker selection: side (≥720) or sheet (≤719, ≥560); same offsets; node visible; replace on reselect; Escape back to node | S3, S8d, S8e | View tests | LIVE L4 (1440 and 360×800) | LIVE plus unit |
| 16 | Attention in Graph: counters, badges, dim, Show in Graph reveals; overlay at narrow or short with Escape to counter; no auto view or focus change | S6, S8b, S8d | View tests | SYN attention-mix (1440×1000, 360×800) | unit plus SYN |
| 17 | Return to Tasks restores scroll, lanes, filters, card (revealed); Show in Tasks focuses card | S8d | View test (stubbed offsets) | LIVE L8 | LIVE plus unit |
| 18 | Splitter drag and keys within 280–50% / 160–75%; `aria-valuenow` updates; graph offsets kept | S3, S8e | `PanelSplitter.test` (pointer plus keys) | LIVE L4 real pointer drag plus keys | LIVE plus unit |
| 19 | Graph one tab stop; ↑↓ reading order; ←→ chain; Enter selects; no Herdr focus change | S6, S8d | `SupervisorGraph.test`; View test `onTerminal` not called | LIVE L5 focus triples plus positive control | LIVE plus unit |
| 20 | 1440×600: switch and filters on one line; queue overlay; graph ≥300 px high; side panel | S3, S8g | `useSupervisorLayout.test` short threshold | LIVE L8 | LIVE |
| 21 | Node clarity (icon slot, title, tier, status, provenance; no overlap; glyph and tier unclipped; five glyphs plus doc icon) at four sizes | S2, S6, S8g | `graphLayout.test` geometry; `SupervisorGraph.test` regions | Product: SYN topology-full DOM geometry at four sizes. Presentation: unchanged accepted BOUNDED:24 evidence, not re-claimed. | SYN plus unit (+ accepted concept evidence) |
| 22 | Only Show in Graph / Show in Tasks move views; banned phrases absent; "focus" only for DOM or Herdr focus | S8, S12 | View test label scan of rendered buttons | scan §7.4; LIVE L1/L8 DOM | scan plus unit plus LIVE |

## 4. Authority guards: where each is re-proved

| Guard | Proof after the change |
|---|---|
| G1 Answer | existing `:359-390`; View test Decide row disabled when not live or unverified |
| G2 Assignment resolve | existing `:391-400` moved into queue row |
| G3 Intent resolve | View test `intent_conflict` row → `intent_resolve` with the joined `intent_id` |
| G4 Identify IDs | `SupervisorActivity.test` plus View test queue Notice row |
| G5 Accept | `SupervisorActions.test` payload and three disable reasons |
| G6 Send back / note lock | `SupervisorActions.test` non-retryable lock |
| G7 Plan override | `SupervisorActions.test` stale/arm/confirm and Escape disarm |
| G8 Restart | existing `:341-358`; `SupervisorActions.test` aria-disabled guard; `SupervisorDialogs.test` Check again first |
| G9 Close tracking | `SupervisorDialogs.test` payload `cancel_run` plus no-kill copy |
| G10 Subagent controls | `SupervisorActions.test` running/live gating |
| G11 Request stop | `SupervisorActions.test` same operation ID retry |
| G12 Edit | existing `:401-434`; `SupervisorDialogs.test` deleted, Use current, Keep my draft |
| G13 Start | existing `:309-340` |
| G14 Herdr focus | View tests `onTerminal` not called across all §5.1 triggers; LIVE L5 |
| G15 Snapshot fencing | `useSupervisor.test.ts` unchanged |
| G16 Reported ≠ observed | `SupervisorActions.test` `ProvenancePair`; existing `:179-200,207-218` |
| G17 Canonical projection / no fake topology | `topology.test` |

## 5. Docs touched (S12)

`CONTEXT.md` §5.7; `DECISIONS.md:54` plus one new Supervisor UI rule bullet; `CODE_GUIDE.md:23`; `docs/keyboard-shortcuts.md:9` (hand-written paragraph only); `docs/supervisor-surfaces.md` rows 4, 13, 20, 23, 27, 28, 36, the lane note at :11, and a new verification paragraph. Accepted planning artifacts are untouched.
