# Observation ledger

This records user intent and new findings, not a second task-status board. Task completion lives only in [tasks.json](tasks.json). Append stable observation IDs; never erase an unresolved observation to obtain a clean finish.

## Initial observations incorporated into task acceptance

| ID | Observation / decision | Acceptance owner | Disposition |
| --- | --- | --- | --- |
| OBS-001 | Fix known GitHub issues and stabilize existing daily-use functionality, not broad feature expansion. | All required tasks; ACCEPT-01 | Incorporated into scope; implementation not yet performed |
| OBS-002 | Random error messages should not appear when no real error occurred. | TERM-02, SYNC-01, WEB-02/04/07 | Fix invalid operations/stale publication at the source; preserve genuine failures |
| OBS-003 | Good performance and clean scrolling are explicit completion criteria. | TERM-03, VIEW-01, WEB-04/06, GLAB-03, PERF-01 | Measured real-surface gates required; no speculative rewrite |
| OBS-004 | Implement missing glab integration; user prepared CLI and test project. | GLAB-01/02/03/04 | Reuse existing source pipeline; preserve gh/tea |
| OBS-005 | Designated test project is https://gitlab.com/nnex.ai/integration; user permits test issue creation. | RUN-01, GLAB-04, ACCEPT-01 | Issue #1 created/read back; ownership marker recorded in inventory; leave open until last consumer |
| OBS-006 | Real GitLab issue URL returned as /-/work_items/1, while issues API reports issue_type issue. | GLAB-01, GLAB-04 | Required URL/type regression; other work-item kinds must not be coerced into issues |
| OBS-007 | User requests individually trackable tasks and orchestrator/subagent handoff. | This planning package | Central JSON status ledger, task briefs, locks, dependencies, evidence and closure policy supplied; no product completion claim |

## Appending live findings

For each new entry record: stable ID; date; reporter/evidence; exact observed action/result; affected task; blocking or queued; acceptance change; and final disposition with evidence/commit. Link a durable run record rather than session-only tool artifacts.

- **Blocking:** continuing would corrupt data, operate on an unauthorized resource, or implement a now-invalid contract. Stop/steer only the affected worker.
- **Queued:** ordinary regression, visual discrepancy, preference or additional acceptance case. Let unrelated work continue; integrate it into the next owning repair wave.
- **Scope change:** requires explicit user approval if it removes a required criterion, adds deferred provider products, or changes a safety/authority contract. Record approval, not an inferred permission.

An in-scope unresolved finding prevents campaign completion. Assign it to an existing task or add a separately owned required task and update dependency/coverage records. This section is intentionally empty of new findings at planning delivery; no additional runtime investigation is claimed.

### OBS-008 — fixture server inherited the real home

- 2026-09-20, Main. During TERM-02 oracle capture, accumulated `csg-tui` logs exposed an earlier integration-install action. Six protected integration/settings paths have matching 09:51:52Z modification timestamps, during RUN-01. No pre-probe copies were recorded; exact previous bytes and whether each install changed content are unknown.
- Blocking for protected-home restoration/reconciliation under ACCEPT-01; isolated product implementation may continue. Stopped owned TUI, server and gateway immediately after discovery. No automatic deletion/rollback of user files.
- RUN-01 containment correction verified: all guarded invocations receive owned HOME; the new negative control fails against baseline, 32 focused tests pass, actual server HOME is owned, and the six protected hashes are unchanged across restart. Main informed the user. This is not resolution of the earlier writes.
- Evidence and exact paths: `runs/run-20260920-a3e9b950/RUN-01.md`, Safety correction section. Final disposition remains unresolved; campaign completion requires verified restoration or the user's explicit decision to retain these integrations.

### OBS-009 — browser-first debugging

- 2026-09-20, user steering: make work/debugging efficient; establish the web solutions with Playwright CLI first.
- All remaining tasks: adopted persistent named CLI attachment to the sole owned Chromium page on the guarded web gateway. Shared frontend regressions run in batched, bounded browser scenarios. Native is reserved for native-specific work and final required acceptance, not the debugging loop. No acceptance criterion is removed.

### OBS-010 — terminal output loss during split zoom/restore

- 2026-09-20, Main, browser-only extended layout probe. A continuously mounted terminal became blank after zoom/restore; Herdr `pane.read` still returned the executed marker and the shell PID remained alive. Local xterm had `cols=60`, `rows=40`, `baseY=viewportY=169` and blank visible buffer rows, so this was not merely missing DOM paint.
- Queued to TERM-03, which explicitly owns terminal frame writes, output continuity, and split/zoom parity. Its original criteria 5–7 remain required; this finding prevents campaign completion until resolved. No zoom/output-continuity pass is claimed under TERM-02.
- Follow-up trace showed received frame grids and local fitted grids can differ. Replaying a real 60-column frame into a 30-column xterm lost a visible row and introduced scrollback; the correctly sized grid did neither. This confirms a grid mismatch hazard, not the complete cause of the original blanking. Distinguish pane-read history from visible frame contents and capture resize HTTP commands before editing.
- Evidence: `runs/run-20260920-a3e9b950/TERM-02.md`; raw owned `term02-browser-split-failure.json/png` and `term02-zoom-diagnostic.json/png`. Root-cause/baseline classification and final disposition remain open. No speculative frame-geometry product change has been applied.
