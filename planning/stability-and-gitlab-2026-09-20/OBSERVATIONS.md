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
- Evidence and exact paths: `runs/run-20260920-a3e9b950/RUN-01.md`, Safety correction section.
- Final disposition, 2026-09-21: the user explicitly selected **Retain current files** after being told no pre-run copies were found. Leave all six current integration/settings files unchanged; no further HOME writes, speculative reconstruction or deletion. This resolves the disposition requirement, not the historical containment failure. The isolated-HOME correction remains required.

### OBS-009 — browser-first debugging

- 2026-09-20, user steering: make work/debugging efficient; establish the web solutions with Playwright CLI first.
- All remaining tasks: adopted persistent named CLI attachment to the sole owned Chromium page on the guarded web gateway. Shared frontend regressions run in batched, bounded browser scenarios. Native is reserved for native-specific work and final required acceptance, not the debugging loop. No acceptance criterion is removed.

### OBS-010 — terminal output loss during split zoom/restore

- 2026-09-20, Main, browser-only extended layout probe. A continuously mounted terminal became blank after zoom/restore; Herdr `pane.read` still returned the executed marker and the shell PID remained alive. Local xterm had `cols=60`, `rows=40`, `baseY=viewportY=169` and blank visible buffer rows, so this was not merely missing DOM paint.
- Queued to TERM-03, which explicitly owns terminal frame writes, output continuity, and split/zoom parity. Its original criteria 5–7 remain required; this finding prevents campaign completion until resolved. No zoom/output-continuity pass is claimed under TERM-02.
- Follow-up trace showed received frame grids and local fitted grids can differ. Replaying a real 60-column frame into a 30-column xterm lost a visible row and introduced scrollback; the correctly sized grid did neither. This confirms a grid mismatch hazard, not the complete cause of the original blanking. Distinguish pane-read history from visible frame contents and capture resize HTTP commands before editing.
- Evidence: `runs/run-20260920-a3e9b950/TERM-02.md`; raw owned `term02-browser-split-failure.json/png` and `term02-zoom-diagnostic.json/png`. Root-cause/baseline classification and final disposition remain open. No speculative frame-geometry product change has been applied.
- Consolidated disposition: frame-grid handling was repaired; the final browser worker observed `TERM02-LIVE-CONTENT` before and after zoom/restore, and terminal regression suites passed. This resolves the bounded zoom/restore regression under OBS-014; broader terminal input/scroll matrices are not implied. See `runs/run-20260920-a3e9b950/BATCH-FINAL.md`.

### OBS-011 — minimize intermediate validation

- 2026-09-21, user steering: too many iterations; keep validation minimal during implementation and do the long, hard rounds at the end.
- All remaining tasks: batch independent implementation and integrate once; use only the smallest immediate regression/smoke checks, reuse existing fixtures, and rerun only affected failures. Defer full suites, exhaustive browser/native/TUI matrices, sustained performance and packaging acceptance to the final integration round.
- Timing changes, not scope reduction: original criteria remain required. Implemented or smoke-checked work is not full task acceptance; no done status or campaign-completion claim until the required final evidence exists.
- Follow-up user steering: required browser checks should be delegated rather than occupy Main's implementation path. The prepared VIEW-01 smoke has not run and is deferred. Main integrates and advances implementation; bounded checks run off that path, with no expanded per-task matrices.
- Completion dependencies remain unchanged. To make final-round acceptance possible without serializing implementation behind every full matrix, settled interface handoffs may unblock dependent code work; unchanged, independently owned interfaces may be implemented concurrently. Record handoffs/ownership and keep incomplete acceptance explicit. No parent/task is marked done from implementation alone.

### OBS-012 — authorized disposable GitLab fixtures

- 2026-09-21, user: “feel free to create mr and issue fixture in glab for testing”.
- Main will create uniquely marked issue/MR fixtures in the already authorized `nnex.ai/integration` project (86672117), including the dedicated non-main branch/commit needed for the MR. Controlled title/body/comment changes are restricted to these new test fixtures. Record IDs, before/after state and cleanup ownership before acceptance use.
- No main/protected-branch writes, merges, approvals, protection changes, unrelated project mutations or credential export. Production provider requests remain GET-only. Fixture creation removes the missing-authorization blocker; successful real acceptance still requires execution evidence. Do not delete remote resources without separate authority; retain/close owned fixtures with explicit accounting.

### OBS-013 — macOS verification handed to the user

- 2026-09-21, user: “macos will stay unavailable so only do what can be done. do not force verification etc. include what can be done and i will verify on a mac”.
- Implement and verify reachable work on the available platform. Do not acquire/emulate a macOS runner or repeat substitute checks to force Darwin/native acceptance.
- Deliver the macOS implementation plus a concrete handoff checklist identifying unexecuted platform-specific checks and known limitations. These checks are user-owned, not passed; unavailable macOS execution no longer blocks delivery of the reachable campaign scope. Preserve original criteria as the checklist rather than deleting or falsely satisfying them.

### OBS-014 — lightweight checks and earlier deliverables

- 2026-09-21, user: the goal is solving the issue and having deliverables; small fixes can follow, and slow verification must not delay momentum. Quick checks should cover most work.
- Supersedes exhaustive verification as an increment-delivery gate. Prioritize working, near-ready fixes; use the compiler plus a narrow real-path smoke or relevant regression, then commit. Do not launch another broad rewrite/review matrix before delivering those increments.
- Preserve expected product behavior and safety/ownership constraints. Known data-loss, unauthorized-action and broken-main-path defects remain blockers for their affected path; minor findings and unexecuted exhaustive scenarios are explicit follow-ups, not reasons to hold unrelated working changes.
- Report exactly what was exercised. Do not call deferred browser/native/performance matrices passed or use a compile-only result as UI proof. Keep all task identities and the macOS user handoff.

### OBS-015 — one implementation batch, then delegated verification

- 2026-09-21, user: “this is taking far too long. just fix everything in one go and then use that as a starting point to have subagents work through, test through and verify the changes.”
- Stop the serial task-by-task smoke/review/commit loop. Complete the remaining implementation in one coordinated batch, integrate the full working tree, then dispatch parallel subagent verification and one consolidated repair pass.
- Preserve existing working changes and completed proof; do not restart them. No builds, tests, formatters or runtime verification during the writing batch. Main owns shared integration and final delivery.
- Keep the original product scope, owned-resource safety, explicitly deferred providers and user-owned macOS checks. Report real verification results and remaining limitations; batching is not permission to claim unexecuted checks.

### OBS-016 — conclude the delivery within ten minutes

- 2026-09-21, user: “come to an end - i expect you to be done within the next 10 minutes”.
- End further broad probes, collect current worker results, preserve evidence, stop owned runtimes and commit the consolidated batch. Do not relabel unresolved failures as passes to meet the deadline.
- `BATCH-FINAL.md` records the delivered fixes, actual checks and remaining scroll-barrier/native annotation acknowledgement failures. Full campaign acceptance remains incomplete; macOS remains user-owned verification.

### OBS-017 — plugin launch confirmation races foreground exec

- 2026-09-22, user reported `mutation_applied_snapshot_failed` with `launch_receipt_unverified`: the opened pane executable or process generation did not match the installed entrypoint. The mutation must not be replayed.
- The adapter checked foreground process evidence once immediately after `plugin.pane.open`. The installed Reviewr command uses a shell `exec`; a startup/foreground-discovery transition can therefore be rejected before the final executable is visible. Read-only inspection found the installed and running Reviewr executable paths equal; no user pane was changed.
- Repair: bound confirmation to one second, re-read only the opened pane's process evidence on the pinned endpoint, retain executable/generation verification, and revalidate terminal/Space/tab identity after the wait. Timeout and real mismatches retain the existing resync-only error. Removed the race-prone classification `expect`; no automatic second open.
- Verification: 94 Herdr adapter tests passed. The delayed-foreground regression fails with the exact reported error when the wait is disabled, then passes with confirmation restored; an unmatched-process timeout remains rejected. In disposable real Herdr session `receipt-td7b34fz`, Reviewr launch returned VerifiedLaunch for `w1:p2`, `pid=2358539:start=15663731`; pane count changed exactly 1 -> 2, and resync retained the same verified identity.
- Cleanup: disposable Herdr stopped, its root and temporary Rust smoke entrypoint removed. Existing user panes and the protected/default session were untouched. Linux runtime proof only; macOS execution is not claimed.
- `cargo build -p cockpit-host -p cockpit-tauri --bins` passed for the repaired adapter. These are rebuilt development binaries, not an automatic replacement or restart of the user's installed/running application.

### OBS-018 — ordinary browser navigation, scrolling and Enter remain unusable

- 2026-09-22, user: browser recovery/toast buttons are unusable; click navigation and scrolling often strand “Showing last confirmed frame”; reproduction page `https://en.wikipedia.org/wiki/Language`; Enter does not work in inputs.
- Accepted repair scope against clean baseline `7111c6b`: restore ordinary live browsing, usable local recovery controls and page Enter semantics. Runtime worker owns helper frame/geometry/key dispatch; UI worker owns BrowserPane/presenter/browser styling and focused regressions. Main owns integration, disposable runtime proof and one verified commit. Existing wire shapes and capture/ownership guards remain authoritative; no replay of rejected input or mutation.
- Acceptance: physically scroll Wikipedia repeatedly and follow a page link with continuing fresh rendering; exercise nested scroll, form Enter and textarea newline on an owned fixture; physically activate local recovery controls. Only uniquely named disposable Herdr/browser resources may be used; protected/default session and user profiles remain untouched. Broader campaign acceptance remains incomplete.
- Implemented: compositor-scroll metadata mismatch triggers event-driven capture reconciliation; source frame credit is released independently of navigation; screencast starts/stops are serialized to prevent duplicate startup. Enter sends Chromium's CR text. Frontend retains one newer frame until matching metadata, discards obsolete generations, preserves matching frames on no-op viewport/title changes, and preserves queued key releases across scroll. Recovery buttons accept pointer events; empty automatic drafts no longer produce recovery prompts.
- Review corrections applied: no unverified screenshot in compositor-scroll repair; no old-generation frame deferral; unchanged viewport notifications cannot erase a usable frame; scroll transitions retain keyboard-release jobs. Pending captures, stream cleanup and error reporting remain intact.
- Passed: `node --check browser-runtime/browser-helper.mjs`; `bun run build`; 60 focused tests across framePresenter and App suites. Actual owned browser attachment reached “Live browser view” after fixing the reproduced `Page.startScreencast: Screencast is already active` startup race.
- User stop directive: “this is the last test you get. we are out of time. fix what you can and commit”. This supersedes the remaining runtime acceptance gate for this commit. The final Wikipedia automation did not complete: URL selection and driver evaluation failed during the check. Repeated Wikipedia scrolling/link navigation, physical recovery-button activation, form/textarea Enter and native behavior are **not verified** by this increment; no claim that the original browser symptoms are fully resolved.
- Verification used only session `cockpit-browse-2mo4ksz2`, owned root `/tmp/cockpit-browse-2mo4ksz2`, gateway 37719 and fixture 37729. Owned Chromium closed through Cockpit; owned services stopped during delivery cleanup. No installed application replacement, user-session mutation or native rebuild in this increment.
