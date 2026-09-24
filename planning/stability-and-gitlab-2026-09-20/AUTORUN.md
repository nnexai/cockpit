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

The native goal is one umbrella goal named for campaign `stability-and-gitlab-2026-09-20`. Its objective stays incomplete until the full required scope and final gate pass. Work on one coherent repair or genuinely independent batch at a time, with explicit scope, owners, and an observable acceptance scenario. Assess progress by verified improvement or new diagnostic evidence, not tool activity. Stop an unproductive approach under the attempt rules below; stop execution for a user pause/handoff or an external blocker with no independent authorized work. **Never call `goal.complete` at a task, repair, or batch boundary.**

Do not infer deadlines from illustrative durations or impose default time limits. Honor only time or cost limits explicitly set by the user; carry such an agreement across workers, compaction, and handoffs. These instructions do not install a timer or disable harness continuation messages.

Pause is sticky. A paused campaign goal never auto-resumes, including after interruption, recovery, a new worker, or a new orchestrator. Resume only on an explicit user instruction. Do not ask the user to restart the goal, relay worker messages, approve routine safe choices, or manually run the helper. At campaign start, inspect native goal state. If no matching goal exists, create this campaign goal; if it exists and is paused, leave it paused unless the user explicitly resumes it. If an unrelated goal is active or paused, do not silently replace, complete, pause, or repurpose it; report the exact collision for deliberate resolution. Do not create competing campaign orchestrators. Recover from the durable ledger and run evidence without replaying completed work.

The campaign is complete only when all 24 original required task identities (plus any user-authorized required additions) are done, every original in-scope criterion passes on its required surface, all owned implementation/evidence changes have real full-SHA commits, and `ACCEPT-01` is complete. `LATER-GHPR` and `LATER-JIRA` remain `deferred`; do not promote, erase, or silently satisfy them. Keep GitHub issue #6 explicitly partial for those deferred provider scopes. Missing required evidence, authorization, commits, or unresolved observations keep the campaign incomplete. An unsuccessful repair is not campaign completion; keep affected criteria open while genuinely independent authorized work proceeds.

### Explicit user scope amendments

OBS-012 authorizes uniquely marked disposable issue/MR fixtures in project `nnex.ai/integration` (86672117), including their dedicated non-main source branch/commit and controlled fixture content changes. Record exact ownership; production adapters remain GET-only. This does not authorize protected-branch changes, merges, approvals or remote deletion.

The user's 2026-09-22 scope decision removes macOS-specific execution and acceptance from this campaign. Preserve historical criteria and handoff artifacts for reference, but do not schedule macOS work or represent it as passed. No runner acquisition, emulation or substitute verification is required.

The campaign has 24 required task identities. macOS-specific execution and acceptance are outside scope; NATIVE-02 remains recorded as deferred. Linux-native behavior remains required in its owning tasks. Other browser/Herdr/provider/security/performance criteria, owned commits, cleanup and unresolved observations remain required. The final gate audits this explicit user-approved scope, not fictitious macOS passes.

OBS-014 replaces exhaustive-matrix verification at each delivery boundary with focused proof, not with weaker safety or a reduced final success contract. For a repair, run relevant cheap checks and prove the affected user-visible path; preserve all original required outcomes and task identities. Record unexecuted deeper scenarios truthfully, never as passed. A known data-loss/ownership hazard or broken primary action blocks the affected criterion.

OBS-015's coordinated batch is historical context, not standing authority to continue an unlimited campaign batch or to resurrect the former per-task plan/review/verify/commit cadence. The current finite contract below supersedes earlier batch notes and historical increment instructions.

### Accepted consolidated implementation batch — Main, 2026-09-21

Baseline is commit `163ee85fba8c883f80b5b24871de84bd210a7823` plus the existing campaign-owned working changes. Preserve those changes. The issue/MR import/render/unchanged-refresh smoke has passed; the resource UI remains uncommitted. Cancellation and Review focused tests passed (46 total), terminal/setup focused tests passed (35 total), and the latest frontend typecheck passed. Do not repeat these during writing.

All remaining task outcomes stay assigned, using existing contracts rather than another architecture. Four disjoint writing boundaries:

1. Browser interaction: App browser/focus/recovery wiring and BrowserPane/UI-local helpers/styles; complete first-gesture/input/resize-barrier/draft/capture/feedback behavior. Preserve operation identity across uncertain feedback delivery; no automatic replay.
2. Browser runtime and geometry: browser-helper.mjs, framePresenter and transform; capture-bound geometry, viewport/control transitions, held input, helper security and bounded frame/decode release. Keep existing wire shapes; do not guess DPR or label current geometry as capture-time evidence.
3. Browser host and transports: Rust browser/config/helper/feedback/delivery modules and TypeScript client adapters; finish dependency, ownership, cancellation, bounded identity validation and durable feedback behavior. Do not edit helper JavaScript, UI or generic comment-paste implementation.
4. Local/source workflows: Context/Review/resources/project dialogs, TerminalPane, project/source/comment workflow backend; finish demonstrated retention/ownership defects and resource-panel density. Preserve the delivered setup, installer and read-only GitLab contracts.

Main exclusively owns protocol/generated integration, campaign records and runtime. No other writer touches App.tsx, the shared protocol or another boundary. Existing wire/API shapes remain the cross-slice contract; report indispensable interface changes to Main rather than creating parallel abstractions. Generic comment paste belongs to the workflow boundary; browser delivery consumes its existing API.

Writing workers read their affected original briefs and authorities, investigate and repair in one pass, and do not run shared gates while concurrent writers mutate the integration surface. Once writers settle, integrate once, perform focused static checks and actual-surface acceptance for the batch, consolidate verified failures into one repair pass, then rerun affected checks. Commit the verified result or a truthful checkpoint. Exclusive owners may run focused checks that do not collide with active writers or shared resources; shared gates wait until all concurrent edits settle. Use distinct owned sessions or serialize access to shared runtime. macOS-specific execution is outside campaign scope. Do not reopen a generic audit loop for incidental polish.

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

## Bounded repair and batch protocol

Sol retains consequential design and integration ownership; Luna is the default implementation owner. The parent may implement a bounded repair directly when a handoff adds no genuine concurrency. Keep current worker routing and advisor model unchanged. One coherent repair has one implementation owner; independent batch slices require disjoint paths/symbols and resource identities. Record scope, owners, acceptance scenario, non-goals, prerequisites, resource locks, and any explicit user limits in existing run evidence. Do not create a parallel status board or change task identities, original acceptance, or authorized scope.

Do not create a fresh formal plan for routine verification or a routine repair with understood scope. Resolve consequential design choices with the orchestrator; preserve the existing interfaces and task contract. For an independent batch, make ownership and shared interfaces explicit before dispatch. Use the selected task brief and relevant authority/evidence, not a per-task plan-acceptance ceremony.

Dispatch only the bounded owned work. During a concurrent writing wave, skip shared builds, tests, formatters, linters, services, and commits. Exclusive owners MAY run cheap focused checks that are confined to their own work and do not exercise shared mutable resources. Never run a shared gate while concurrent writers are changing its integration surface. No worker mutates the active/default Herdr session, user browser profiles, installed binaries, protected `main`, or unauthorized remote fixtures. Workers return exact changed paths, implemented behavior, unresolved risks, and checks already run.

Integrate the selected coherent repair or independent batch once after its writers settle. One integration owner retains shared protocol/generated files, campaign records, shared lifecycle boundaries, and unavoidable cross-task contracts. Review the integrated change against the bounded contract and existing authorities. Read-only review is reserved for a named, concrete security, data-loss, ownership, or concurrency risk; UI/native/architecture involvement alone does not require generic review. Preserve every original criterion, task identity, and authorized user scope.

After integration, run focused relevant static checks, then exercise the actual acceptance scenario on its required surface through authoritative response/event to visible success or actionable failure. Bug repairs use a safe reproduction or negative control where feasible; user-reported failures are ground truth. Browser criteria require browser proof; Linux-native commands/channels/startup/input/image decode require real native proof; Herdr semantics require a uniquely named disposable oracle where applicable. Tests/builds do not substitute for runtime proof. Main performs project-wide validation once after all assigned work settles.

Consolidate failures from this acceptance pass into at most one repair pass. Then rerun only affected checks and acceptance scenarios. If the same criterion has failed repairs twice, stop editing for that criterion: perform one materially distinguishing experiment or consult the configured advisor, or checkpoint the evidence and stop. Another repair requires new evidence. Do not lower acceptance, relabel failure as warning, fabricate fixtures, or claim unobserved outcomes. Classify the result as product defect, automation defect, stale runtime, or missing prerequisite; keep the affected task/criterion open or blocked as appropriate.

Record scope, owner, baseline, acceptance scenario, checks and observed results, relevant runtime identity, owned resources and cleanup, failures, and remaining criteria in compact run evidence. Commit a verified repair or integrated batch after its relevant checks pass; record its real full SHA and evidence paths in `tasks.json` after the commit. An unsuccessful checkpoint may leave unverified product edits uncommitted and clearly labeled. Do not create separate plan, evidence, or status commits solely for ceremony. A task remains open until all original acceptance, dependencies, and required commits pass.

After verified progress, continue already-authorized work without another user prompt. Stop an inconclusive approach under the attempt rule, not merely because a fixed duration elapsed. Stop execution for a user pause/handoff, an explicitly agreed limit, or an external blocker with no independent authorized work. Do not launch replacement repair workers to evade failed-attempt history. Preserve ownership/locks while partial edits or exclusive resources remain; release only at a safe recorded checkpoint with no resource/write collision. A blocked task never becomes ready automatically.

## Failures, diagnosis, and live feedback

A failed check leaves its affected task/criterion open. Diagnose the failure boundary and assumptions rather than repeating the same repair. After two failed repairs of the same criterion, stop editing and perform one materially distinguishing experiment/advisor consultation or checkpoint; further repair requires new evidence. Record the result and classify product defect, automation defect, stale runtime, or missing prerequisite. Do not lower acceptance, relabel failure as warning, fabricate a fixture, or claim a build proves runtime behavior. No automatic retries or unbounded attempt clause is authorized.

Maintain a live feedback queue using stable observation IDs in `OBSERVATIONS.md` and the active run record. Queue ordinary regressions, visual discrepancies, and preferences for the owning repair without silently enlarging its scope. Genuinely independent authorized work may continue. Interrupt only an affected worker when its contract becomes unsafe or invalid, with one factual message naming the observation and changed acceptance. Preserve queued findings and assign each in-scope finding to an existing required task or documented addition before completion; actual scope expansion requires the user's decision.

Explicit user pause, stop, handoff, or workflow corrections are immediate control instructions, not queued observations. Preserve work and stop safely; todo reminders and automatic goal continuations are not user resume authorization.

## External blockers and authorization

For missing MR fixture/branch authorization, unavailable tools, or missing permissions, discover the narrow prerequisite with read-only probes. macOS execution is outside campaign scope and is not a blocker. Block only the affected in-scope criterion, record attempted discovery and the next unblock action, and continue genuinely independent authorized work. Do not create/push a branch or MR, mutate protected `main`, change protections, merge/delete remote resources, revoke shared access, or broaden production-adapter writes merely to remove a blocker. If human action is unavoidable, make one precise grouped ask after reachable work is exhausted; do not busy-loop, repeatedly ask, or fabricate completion. Authorized disposable fixture mutations require before/after records, explicit ownership, cleanup, and no credential capture. A missing MR blocks MR acceptance only.

## Recovery and context maintenance

On explicit user-authorized resume, inspect the current worktree and ledger, active evidence, referenced commits, resource ownership, and lock state before selecting work. Recover abandoned ownership deliberately and preserve historical evidence. Never auto-resume a paused goal, infer completion from a heading/green test count/agent message/issue state, rerun done work without invalidating evidence, steal a live lock, or start a competing orchestrator.

## Final gate and stop behavior

Before attempting `goal.complete`, audit current required task evidence against affected source, run `campaign.py complete`, and verify every required task is `done` with passing criteria, real full commit SHAs, reachable evidence, resolved observations, and no active/blocked/waiting prerequisite. Reconcile `ACCEPT-01` against the in-scope integrated browser/Linux-native/provider journey, cleanup, issue dispositions, and required acceptance matrix; A23 remains explicitly excluded. Confirm `LATER-GHPR` and `LATER-JIRA` remain deferred and issue #6 is truthfully partial. Project-wide validation and final integrated smoke happen once on the settled tree. No historical summary or bounded-batch pass substitutes for this full campaign gate.

If the final gate passes, commit the final acceptance/ledger checkpoint and rerun `campaign.py complete` before completing the campaign goal. If required evidence, authorization, cleanup, issue reconciliation, or commits are missing, keep the goal incomplete and record exact remaining work. Preserve locks where needed. Stop safely for an explicit user pause/limit or an external blocker with no justified independent work; do not repeat failed approaches without learning or report campaign completion because one batch succeeded.
