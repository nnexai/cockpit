# Stability and GitLab campaign autorun

This file is the executable handoff for one native OMP `/goal` campaign. It is preparation-only until the user invokes the exact start command below: it does not launch Cockpit, Herdr, a browser, a native build, a service, a worker, a GitLab mutation, or any other product work by itself.

## Start and progress

Start the single umbrella goal with this exact one-line command:

```text
/goal Execute planning/stability-and-gitlab-2026-09-20/AUTORUN.md
```

Optional read-only progress inspection (the orchestrator may run this itself; the user need not run it) is:

```sh
python3 planning/stability-and-gitlab-2026-09-20/campaign.py check
```

The helper is bookkeeping only. It never launches product work, claims a task, takes a lock, changes a fixture, starts a service, or substitutes for runtime proof. `check` validates metadata and recorded completed evidence/commits; pending work is allowed. `ready` reports eligible candidates together with active, blocked, and waiting work; it is not a safe concurrent dispatch batch. `complete` additionally refuses until every required task is done. Malformed or inconsistent state fails all three commands; unfinished valid work fails only `complete`. Use `--ledger PATH` only for an isolated, disposable bookkeeping smoke; the real campaign ledger remains `planning/stability-and-gitlab-2026-09-20/tasks.json`.

## Goal invariants

The native goal is one umbrella goal named for campaign `stability-and-gitlab-2026-09-20`. Keep it active while bounded increments are implemented, verified, committed, and recorded. **Never call `goal.complete` at an increment boundary.** Do not ask the user to restart the goal, relay worker messages, approve routine safe choices, or manually run the helper.

At campaign start, inspect native goal state. If the matching campaign goal is paused, resume it. If no goal exists, create this campaign goal. If an unrelated goal is active or paused, do not silently replace, complete, pause, or repurpose it; report the exact collision for deliberate resolution. Do not create competing campaign orchestrators. After an interruption, recover from the durable ledger and run evidence; do not replay completed work merely because a worker or session disappeared.

The completion outcome is all 25 original required tasks plus any documented in-scope required additions done, every original criterion passed on its required surface, every owned implementation/evidence change committed with real full SHA values, and `ACCEPT-01` complete. `LATER-GHPR` and `LATER-JIRA` remain `deferred`; do not promote, erase, or silently satisfy them. Keep GitHub issue #6 explicitly partial for those deferred provider scopes. A blocked platform, missing MR authorization, failed gate, unresolved observation, missing commit, or missing evidence keeps the umbrella goal incomplete.

### Explicit user scope amendments

OBS-012 authorizes uniquely marked disposable issue/MR fixtures in project `nnex.ai/integration` (86672117), including their dedicated non-main source branch/commit and controlled fixture content changes. Record exact ownership; production adapters remain GET-only. This does not authorize protected-branch changes, merges, approvals or remote deletion.

The user's 2026-09-22 scope decision removes macOS-specific execution and acceptance from this campaign. Preserve historical criteria and handoff artifacts for reference, but do not schedule macOS work or represent it as passed. No runner acquisition, emulation or substitute verification is required.

The campaign has 24 required task identities. macOS-specific execution and acceptance are outside scope; NATIVE-02 remains recorded as deferred. Linux-native behavior remains required in its owning tasks. Other browser/Herdr/provider/security/performance criteria, owned commits, cleanup and unresolved observations remain required. The final gate audits this explicit user-approved scope, not fictitious macOS passes.

OBS-014 supersedes the exhaustive-matrix requirement as a gate for each delivery increment. Deliver working fixes after lightweight, targeted proof of the affected path; do not delay near-ready deliverables for another broad audit or low-severity follow-up. Preserve the required product outcomes, protected-resource rules and task identities. A task may close for this adjusted delivery when its behavior is implemented, its relevant quick checks pass, its changes are committed and remaining minor findings/deeper unexecuted scenarios are recorded. Do not describe those scenarios as passed or claim full original matrix acceptance. A known data-loss/ownership hazard or broken primary action still blocks its affected path, not unrelated increments.

OBS-015 supersedes the remaining serial increment-delivery cadence: finish the remaining implementation as one coordinated batch, then have subagents test and verify the integrated changes in parallel, consolidate repairs and deliver. Do not continue per-task commit/review/smoke loops.

### Accepted consolidated implementation batch — Main, 2026-09-21

Baseline is commit `163ee85fba8c883f80b5b24871de84bd210a7823` plus the existing campaign-owned working changes. Preserve those changes. The issue/MR import/render/unchanged-refresh smoke has passed; the resource UI remains uncommitted. Cancellation and Review focused tests passed (46 total), terminal/setup focused tests passed (35 total), and the latest frontend typecheck passed. Do not repeat these during writing.

All remaining task outcomes stay assigned, using existing contracts rather than another architecture. Four disjoint writing boundaries:

1. Browser interaction: App browser/focus/recovery wiring and BrowserPane/UI-local helpers/styles; complete first-gesture/input/resize-barrier/draft/capture/feedback behavior. Preserve operation identity across uncertain feedback delivery; no automatic replay.
2. Browser runtime and geometry: browser-helper.mjs, framePresenter and transform; capture-bound geometry, viewport/control transitions, held input, helper security and bounded frame/decode release. Keep existing wire shapes; do not guess DPR or label current geometry as capture-time evidence.
3. Browser host and transports: Rust browser/config/helper/feedback/delivery modules and TypeScript client adapters; finish dependency, ownership, cancellation, bounded identity validation and durable feedback behavior. Do not edit helper JavaScript, UI or generic comment-paste implementation.
4. Local/source workflows: Context/Review/resources/project dialogs, TerminalPane, project/source/comment workflow backend; finish demonstrated retention/ownership defects and resource-panel density. Preserve the delivered setup, installer and read-only GitLab contracts.

Main exclusively owns protocol/generated integration, campaign records and runtime. No other writer touches App.tsx, the shared protocol or another boundary. Existing wire/API shapes remain the cross-slice contract; report indispensable interface changes to Main rather than creating parallel abstractions. Generic comment paste belongs to the workflow boundary; browser delivery consumes its existing API.

Writing workers read their affected original briefs and existing authorities, investigate and repair in one pass, and skip builds/tests/formatters/runtime/commits. They return exact changed paths and unresolved contract seams. Once writers settle, Main integrates once and dispatches independent bounded browser/Linux-native/workflow/provider/security/resource verification using distinct owned sessions or serialized shared-runtime access. macOS-specific execution is outside campaign scope. One consolidated repair pass follows observed failures; no new audit loop for incidental polish.

## Bootstrap after the trigger

Perform this bootstrap once for the campaign (and again only when a recovery checkpoint says the authority set changed):

1. Read repository authorities `CONTEXT.md`, `DECISIONS.md`, and `CODE_GUIDE.md`, then `skill://incremental-delivery`. Before UI changes read `research/ui-design-direction.md` and `research/ui-implementation-constraints.md`; for Herdr-backed UI also read `skill://cockpit-ui-parity`. For browser automation, read `skill://playwright-cli` before using that workflow.
2. Read this campaign's `ORCHESTRATOR.md`, `README.md`, `INVENTORY.md`, `OBSERVATIONS.md`, `ACCEPTANCE.md`, `PLAN_VALIDATION.md`, and `tasks.json`. Re-inspect the actual worktree and protect pre-existing changes; dated inventory is not a runtime claim.
3. Confirm that the ledger is the only mutable task-status board. Read the full dependency graph and campaign acceptance map, then only the selected task briefs and relevant dependency evidence. Do not preload all 27 briefs. Preserve every selected brief's original requirements; consult affected briefs before changing scope/dependencies, and never rewrite criteria into a smaller substitute.
4. Run the read-only helper `check` to detect malformed metadata or missing/inconsistent completion records. Use `ready` only as a dependency hint after checking external prerequisites, locks, ownership, and actual source. Helper output never proves behavior or authorizes a claim.
5. Establish the campaign's source baseline from the current worktree and ledger. Record the baseline and protected pre-existing files in the first run evidence; never absorb unrelated changes into a task commit.

After bootstrap, each next selection reads only the relevant task brief, dependency evidence, observations, and authority sections needed for that task. Do not repeatedly load unrelated historical plans or treat old headings, screenshots, agent messages, test totals, or issue state as completion evidence.

## Required task set and dependency discipline

The required set is exactly:

`RUN-01`, `TERM-01`, `TERM-02`, `SYNC-01`, `TERM-03`, `VIEW-01`, `FLOW-01`, `SETUP-01`, `SETUP-02`, `GLAB-01`, `GLAB-02`, `GLAB-03`, `GLAB-04`, `WEB-01`, `WEB-02`, `WEB-03`, `WEB-04`, `WEB-05`, `WEB-06`, `WEB-07`, `WEB-08`, `NATIVE-01`, `PERF-01`, and `ACCEPT-01`. NATIVE-02 remains recorded as deferred, outside this required list.

Keep the exact `depends_on` values in `tasks.json` as the completion dependency graph. Under the user's implementation-first steering in OBS-011, implementation may advance against settled, integrated parent interfaces before the parent's exhaustive acceptance round; an independent slice may use an unchanged existing interface without waiting for unrelated parent UI proof. Record the interface handoff and exclusive writing ownership explicitly. This changes validation timing, not required outcomes: no task is `done` until its original criteria and completion dependencies pass. Never claim readiness from headings, waive safety/authorization prerequisites, or call a compile-only scaffold a completed parent.

## Bounded increment protocol

For each increment, a **Sol or Astra orchestrator** owns intent, planning, plan acceptance, decomposition, integration, verification, commits, and ledger updates. Planning must not be delegated to Luna or another cheaper implementation model. Use only enabled bundled agents plus the retained Astra advisor under the worker contract in `ORCHESTRATOR.md`; do not restore disabled project-specific agents.

1. **Select safely.** Resume the owned unfinished increment/task first; do not abandon it merely because `ready` lists other candidates. Otherwise claim a pending dependency-ready task or a genuine disjoint wave whose paths, symbols, sessions, fixtures, ports, displays, and locks are exclusive. Each selected task gets its own just-in-time planning round and accepted plan before dispatch, including tasks in a disjoint wave. Acquire declared locks before dispatch. Record task ID, source baseline, owner, run ID, writable paths/symbols, non-goals, dependencies, prerequisites and resource identities. A `ready` listing is not a claim. Do not take over another live owner; investigate and record deliberate recovery for abandoned ownership.
2. **Define one observable outcome.** Split broad work just in time into concrete user-visible outcomes. Sequential increments may touch the same files; concurrent slices require disjoint ownership. Every increment has an exact positive check and an important negative/error/ownership check, with starting fixture, action and expected observation. Name each browser, Linux-native, macOS, Herdr, provider or integrated surface required by the original criterion. Keep every original criterion attached to an owning increment. Update dependencies/coverage/ledger metadata together only if actual task decomposition is necessary; never create compile-only milestones or a second status board.
3. **Plan, accept, and record.** For every selected dependency-ready task, including no-code verification, follow the authoritative mandatory planning round in `ORCHESTRATOR.md` before any worker edit, execution, service start, or fixture mutation. Inspect current source/authorities/dependency evidence; write the baseline, preserved requirement coverage, exact ownership and design, bounded recipe, real-surface checks, resources/authorization, and escalation/stop boundaries into the existing `runs/<run-id>/<TASK-ID>.md`. The strong orchestrator must accept the plan with no unresolved assigned-slice design decision. Never dispatch an unaccepted or stale plan; routine user approval is not required, and planning adds no task status or second board.
4. **Dispatch workers.** Dispatch only against the accepted current plan, with its self-contained recipe, exclusive path/symbol contract, relevant authorities and dependency evidence, acceptance criteria, and explicit non-goals. Use the cheapest capable configured worker; a trivial or lone task does not require a planning subagent, and global routing must not be changed to economize. Workers implement only their slice. During a concurrent writing wave workers skip formatters, linters, builds, tests, services, and commits; they do not mutate the active/default Herdr session, user browser profiles, installed binaries, protected `main`, or unauthorized remote fixtures. Workers return changed paths, behavior, risks, proposed focused checks, and any consequential advisor recommendation. A successful worker message is not acceptance.
5. **Integrate and review.** Drain conflicting writers before touching a shared seam. One integration owner owns protocol/generated files, shared lifecycle boundaries, and any unavoidable cross-task contract. Inspect the integrated diff against authorities and the bounded contract. Dispatch a read-only reviewer for security, concurrency, lifecycle, protocol, provider-authority, native, or UI-parity boundaries. Repair findings in the owning task; do not widen authorization or scope to make a gate pass.
6. **Verify the real surface.** Run the narrow relevant gates once after edits settle, then exercise the actual user action through its authoritative response/event to visible success or actionable failure. Bug repairs require a safe red-capable reproduction or negative control and post-fix proof. Browser criteria require browser proof; Tauri/native commands, channels, startup, input, or image decode require real native proof; macOS criteria require macOS proof, not Linux inference; Herdr semantics require a uniquely named disposable oracle where applicable. Tests/builds are not runtime proof by themselves. Main performs final project-wide validation; workers must not run shared validation while siblings are editing.
7. **Record and commit.** Capture exact commands/scenarios, observed results, platform/build/runtime identities, fixture/resource ownership, cleanup, redaction, review/advisor decisions, failures, and remaining criteria in compact run evidence. Commit only the bounded increment's owned implementation and evidence after that increment's declared criteria and gates pass. Record full commit SHA values and evidence paths in `tasks.json` after the commit; checkpoint that ledger update in a separate status commit or the next owned increment (no self-SHA loop). A task needing multiple increments remains `in_progress`/`verifying` until all its original criteria pass and every required commit is recorded.
8. **Continue immediately.** If the parent still has unmet criteria, retain its `in_progress` ownership/locks and execute its next bounded increment. Mark it `done` only after its complete original acceptance passes, then choose the next task without routine user confirmation. Release locks only after a safe checkpoint with no partial edits or exclusive resources. A blocked task with an owner retains locks; set `owner: null` only after a durable safe checkpoint proves release is safe, retaining its historical `started_at` and explaining the release in notes.

## Failures, diagnosis, and live feedback

A failed gate leaves the increment open. Fix the root cause and rerun only the smallest affected gate after integration. Do not lower acceptance, relabel a failure as a warning, fabricate a fixture, or claim that a build proves runtime behavior. If the same approach fails again without new evidence, stop repeating it: inspect the failure boundary and assumptions, narrow the diagnosis, and consult the configured read-only advisor when the repository consultation rules require it. Record the diagnosis, decision, and verification in run evidence.

Maintain a live feedback queue using stable observation IDs in `OBSERVATIONS.md` and the active run record. Ordinary regressions, visual discrepancies, and preferences are queued for the owning repair increment; unrelated work continues. Interrupt only the worker whose contract became unsafe or invalid, with one factual message stating the observed result, invalid assumption, and changed acceptance. Never broadcast routine interruption or lose a queued finding. Assign every in-scope finding to an existing required task or a documented new required task before completion; only an actual scope expansion needs the user's decision.

## External blockers and authorization

For missing macOS execution, missing MR fixture/branch authorization, unavailable tools, or missing permissions, first discover the narrow tool/resource/prerequisite with read-only probes and finish all independent work. Block only the affected acceptance criterion/task, record the exact attempted discovery and next unblock action, and keep the umbrella goal incomplete. Do not create/push a branch or MR, mutate protected `main`, change protections, merge/delete remote resources, revoke shared access, or broaden production-adapter writes merely to remove a blocker. If a human action is genuinely unavoidable, make one precise grouped ask naming the exact resource, permission, scope, and affected criteria after reachable work is exhausted; do not busy-loop, repeatedly ask, or fabricate completion. Authorized disposable fixture mutations must have before/after records, explicit ownership, cleanup, and no credential capture. A missing MR blocks MR acceptance only; issue/browser/native/recovery work continues.

## Recovery and context maintenance

After each accepted increment, write a compact durable checkpoint in the committed run evidence: ledger state, current source/commit, active locks/owners, resources and cleanup, accepted criteria, queued observations, blockers, and the exact next selection rule. Use configured automatic context maintenance as needed. Manual `/handoff` is optional and never a user prerequisite; if used, preserve the same checkpoint and campaign-goal identity. On interruption, recover by checking the current worktree, ledger, active evidence, referenced commit reachability, resource ownership, and lock state before selecting work. Do not rerun a done task, steal a stale lock, or start a competing orchestrator without a deliberate recovery decision and durable record.

## Final gate and stop behavior

Before attempting `goal.complete`, run the current-state full campaign gate, not a historical summary: audit every required task's evidence against current affected source, run `campaign.py complete`, verify every required status is `done` with passing criteria, real full commit SHAs, reachable evidence, resolved observations, and no active/blocked/waiting prerequisite. Reconcile `ACCEPT-01` across the integrated browser/native/macOS/provider journey, cleanup, issue dispositions, and the A01–A25 matrix (A23 remains explicitly excluded). Confirm `LATER-GHPR` and `LATER-JIRA` remain deferred and issue #6 is truthfully partial. The integration owner performs final integrated validation and smoke verification on the settled tree; this autorun file makes no product/runtime claim.

If the final gate passes, commit the final acceptance/ledger checkpoint and rerun `campaign.py complete` on that settled state before completing the single campaign goal. If any criterion, commit, platform proof, fixture authorization, cleanup, or issue reconciliation is missing, do not complete the goal: leave the exact blocker in the ledger/run evidence, preserve locks where needed, continue independent reachable increments, or stop safely awaiting the one precise prerequisite. Never stop merely because a worker finished, a task boundary was reached, a helper reported ready, or a user would need to relay the next step.
