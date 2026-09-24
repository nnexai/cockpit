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

The native goal tracks the full campaign, but a run executes one fixed user-authorized batch under `skill://incremental-delivery`. The objective stays incomplete until the full required scope and final gate pass. Freeze the batch and its verification list, implement across it once, integrate once, verify once, clean up and report. Then pause execution even if the campaign is incomplete. **Never call `goal.complete` at a task or batch boundary; pause the campaign goal instead.** An incomplete goal is not permission to start another acceptance subset or repair cycle.

Do not infer deadlines from illustrative durations or impose default time limits. Honor only time or cost limits explicitly set by the user; carry such an agreement across workers, compaction, and handoffs. These instructions do not install a timer or disable harness continuation messages.

Pause is sticky. A paused campaign goal never auto-resumes, including after interruption, recovery, a new worker, or a new orchestrator. Resume only on an explicit user instruction. Do not ask the user to restart the goal, relay worker messages, approve routine safe choices, or manually run the helper. At campaign start, inspect native goal state. If no matching goal exists, create this campaign goal; if it exists and is paused, leave it paused unless the user explicitly resumes it. If an unrelated goal is active or paused, do not silently replace, complete, pause, or repurpose it; report the exact collision for deliberate resolution. Do not create competing campaign orchestrators. Recover from the durable ledger and run evidence without replaying completed work.

The campaign is complete only when all 24 original required task identities (plus any user-authorized required additions) are done, every original in-scope criterion passes on its required surface, all owned implementation/evidence changes have real full-SHA commits, and `ACCEPT-01` is complete. `LATER-GHPR` and `LATER-JIRA` remain `deferred`; keep GitHub issue #6 explicitly partial. Missing evidence, authorization, commits or unresolved observations keep the campaign incomplete, but do not prevent a truthful batch report and pause. Finish safe independent work already in the frozen batch, not newly selected work.

### Explicit user scope amendments

OBS-012 authorizes uniquely marked disposable issue/MR fixtures in project `nnex.ai/integration` (86672117), including their dedicated non-main source branch/commit and controlled fixture content changes. Record exact ownership; production adapters remain GET-only. This does not authorize protected-branch changes, merges, approvals or remote deletion.

The user's 2026-09-22 scope decision removes macOS-specific execution and acceptance from this campaign. Preserve historical criteria and handoff artifacts for reference, but do not schedule macOS work or represent it as passed. No runner acquisition, emulation or substitute verification is required.

The campaign has 24 required task identities. macOS-specific execution and acceptance are outside scope; NATIVE-02 remains recorded as deferred. Linux-native behavior remains required in its owning tasks. Other browser/Herdr/provider/security/performance criteria, owned commits, cleanup and unresolved observations remain required. The final gate audits this explicit user-approved scope, not fictitious macOS passes.

The current live-browser image-quality contract is canonical in [ACCEPTANCE.md](ACCEPTANCE.md#live-browser-image-quality) and applies to WEB-03/04/06. The user's amendment is “attempt to have a sharp image, allow lower resolution to keep up performance”: keep animation, hover, scroll, and input live; accept a current lower-density frame only when identity/geometry barriers pass; and never hold newer content for an older sharper image. This supersedes density-only historical failures as current acceptance criteria, without rewriting old run evidence or marking prior runs passed. Preserve pinned PNG/annotation fidelity and all other acceptance criteria.

OBS-014 replaces exhaustive-matrix verification at each delivery boundary with focused proof, not weaker safety or a reduced final success contract. Choose affected checks and representative required-surface journeys before the implementation pass; reuse valid prior evidence. Deeper unexecuted scenarios stay unverified. A known data-loss/ownership hazard or broken primary action fails its criterion; it does not automatically authorize another repair or an expanding permutation search.

OBS-015's coordinated batch is historical context, not standing authority to continue an unlimited campaign batch or to resurrect the former per-task plan/review/verify/commit cadence. The current finite contract below supersedes earlier batch notes and historical increment instructions.

### Historical consolidated implementation batch — Main, 2026-09-21

Baseline is commit `163ee85fba8c883f80b5b24871de84bd210a7823` plus the existing campaign-owned working changes. Preserve those changes. The issue/MR import/render/unchanged-refresh smoke has passed; the resource UI remains uncommitted. Cancellation and Review focused tests passed (46 total), terminal/setup focused tests passed (35 total), and the latest frontend typecheck passed. Do not repeat these during writing.

All remaining task outcomes stay assigned, using existing contracts rather than another architecture. Four disjoint writing boundaries:

1. Browser interaction: App browser/focus/recovery wiring and BrowserPane/UI-local helpers/styles; complete first-gesture/input/resize-barrier/draft/capture/feedback behavior. Preserve operation identity across uncertain feedback delivery; no automatic replay.
2. Browser runtime and geometry: browser-helper.mjs, framePresenter and transform; capture-bound geometry, viewport/control transitions, held input, helper security and bounded frame/decode release. Keep existing wire shapes; do not guess DPR or label current geometry as capture-time evidence.
3. Browser host and transports: Rust browser/config/helper/feedback/delivery modules and TypeScript client adapters; finish dependency, ownership, cancellation, bounded identity validation and durable feedback behavior. Do not edit helper JavaScript, UI or generic comment-paste implementation.
4. Local/source workflows: Context/Review/resources/project dialogs, TerminalPane, project/source/comment workflow backend; finish demonstrated retention/ownership defects and resource-panel density. Preserve the delivered setup, installer and read-only GitLab contracts.

Main exclusively owns protocol/generated integration, campaign records and runtime. No other writer touches App.tsx, the shared protocol or another boundary. Existing wire/API shapes remain the cross-slice contract; report indispensable interface changes to Main rather than creating parallel abstractions. Generic comment paste belongs to the workflow boundary; browser delivery consumes its existing API.

This historical ownership plan does not authorize another execution pass. On explicit resume, carry forward existing changes and evidence into the fixed batch below. Writers implement once without intermediate validation; Main integrates once and runs the frozen checks once. No post-verification repair wave, issue-by-issue native/browser cycle, or generic audit is authorized. macOS-specific execution remains outside scope.

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

## Fixed batch execution contract

This contract and `skill://incremental-delivery` supersede older per-task/per-increment repair, review, verify and continue instructions, including task briefs and run notes. The user's instruction is one implementation attempt across the issues, then consolidated verification once. Preserve all required outcomes; do not silently narrow the assignment to a small WEB-05 subset.

1. **Freeze:** Record the user-authorized outcomes, owners, source baseline and affected-scope verification list in the existing run record. For the remaining campaign, include the known remaining issues in the implementation pass, respecting dependencies and safety prerequisites. Do not create another plan file or status ledger. Previously passed evidence remains valid unless affected source/runtime assumptions changed; state that reason before rerunning.
2. **Implement:** Sol retains scope/integration; Luna remains the default implementation owner, with parent-owned bounded work when delegation adds no useful concurrency. Assign only disjoint slices with fixed interfaces. Writers skip builds, tests, linters, runtime acceptance and review cycles. Required generation is implementation. A minimal pre-edit diagnostic is allowed only for a specific unknown needed to implement, not to reconfirm user reports or expand acceptance.
3. **Integrate:** Integrate the batch once after writers settle. Keep one owner for protocol/generated files, campaign records and shared boundaries. No replacement workers or new sub-batches to reset the pass. Preserve current routing/advisor policy; consultation cannot grant more passes. Named-risk review, if needed, stays within this implementation pass.
4. **Verify:** On the settled result, run each planned relevant static check and actual-surface scenario once. Reuse existing fixtures and representative journeys; browser/native proof remains required where applicable. Observe the action, authoritative response and rendered result. Do not repair product code or harness and rerun after verification begins. A failed build leaves dependent runtime scenarios unrun, not tested against an older binary. Complete other safe independent planned checks.
5. **Report and stop:** Classify each result as passed, failed or unrun, with product/automation/stale-runtime/prerequisite reasons for failures. Clean up owned resources and verify exit. Commit only verified owned changes; preserve and label unverified edits. Report concrete delivered outcomes, commits, failures and remaining criteria; pause the native campaign goal. Another repair batch requires explicit user authorization, even after a successful subset or newly distinguishing evidence. Do not call campaign completion unless the full final gate passes.

Do not replace this batch-level stop with a per-criterion attempt counter. Switching criterion, calling the next check an “increment,” correcting a harness, gaining new evidence, consulting an advisor, replacing a worker or compacting context does not reset execution. `/goal` continuation, todo reminders, an open backlog and instruction maintenance are not resume authorization. Honor explicit user limits, never inferred deadlines.

## Progress, failures and live feedback

`tasks.json` remains the only authoritative task ledger. Reflect concrete batch outcomes promptly in visible todos/progress, tied to the parent task and evidence; distinguish implemented, verified and committed. Do not leave only giant unchanged campaign headings, create a second mutable ledger, or mark a parent done from a subset. Each batch report lists what changed and exactly what remains, even when a parent status legitimately stays open.

Queue newly discovered defects, edge cases and preferences in `OBSERVATIONS.md` and the owning task without enlarging the current pass. Finish only already-planned independent work. A failed check is an honest failed/unverified criterion, not proof of an external blocker and not permission for another experiment or repair. Preserve original acceptance, no fabricated fixtures or warning-only relabeling.

Explicit stop, handoff and workflow corrections interrupt execution immediately. Preserve edits/evidence, stop workers and clean up owned resources. Pause stays sticky across compaction, worker replacement and handoff. Carry the frozen batch, current phase and already-attempted checks forward; the next owner gets only the remaining pass, not a fresh allowance.

## Verification resource lifecycle

Record owned browser roots/processes, profiles, ports/sockets, helpers, services and sessions when launched. Reuse one isolated environment per required surface unless a planned concurrency/isolation case needs more; close a failed environment before replacing it. Use only uniquely named disposable Herdr sessions, never the user's active/default session or browser profile.

On success, failure, interruption or handoff, close owned tabs and terminate owned browser roots/helpers/drivers/servers; verify recorded process exit and released endpoints before claiming cleanup. Empty `browser.tabs()`, a released handle and successful stop output do not prove process exit. Never kill by broad browser names or touch shared/user resources. Report uncertain ownership and exact leftovers. Retaining verification resources needs explicit user authorization.

## External blockers and authorization

For missing MR fixture/branch authorization, unavailable tools, or missing permissions, discover the narrow prerequisite with read-only probes. macOS execution is outside campaign scope and is not a blocker. Block only the affected in-scope criterion, record attempted discovery and the next unblock action, and continue genuinely independent authorized work. Do not create/push a branch or MR, mutate protected `main`, change protections, merge/delete remote resources, revoke shared access, or broaden production-adapter writes merely to remove a blocker. If human action is unavoidable, make one precise grouped ask after reachable work is exhausted; do not busy-loop, repeatedly ask, or fabricate completion. Authorized disposable fixture mutations require before/after records, explicit ownership, cleanup, and no credential capture. A missing MR blocks MR acceptance only.

## Recovery and context maintenance

On explicit user-authorized resume, inspect the current worktree and ledger, active evidence, referenced commits, resource ownership, and lock state before selecting work. Recover abandoned ownership deliberately and preserve historical evidence. Never auto-resume a paused goal, infer completion from a heading/green test count/agent message/issue state, rerun done work without invalidating evidence, steal a live lock, or start a competing orchestrator.

## Final gate and stop behavior

Before attempting `goal.complete`, audit current required task evidence against affected source, run `campaign.py complete`, and verify every required task is `done` with passing criteria, real full commit SHAs, reachable evidence, resolved observations, and no active/blocked/waiting prerequisite. Reconcile `ACCEPT-01` against the in-scope integrated browser/Linux-native/provider journey, cleanup, issue dispositions, and required acceptance matrix; A23 remains explicitly excluded. Confirm `LATER-GHPR` and `LATER-JIRA` remain deferred and issue #6 is truthfully partial. Project-wide validation and final integrated smoke happen once on the settled tree. No historical summary or bounded-batch pass substitutes for this full campaign gate.

If the final gate passes, commit the verified final acceptance/ledger result and run the read-only `campaign.py complete` against that recorded result before completing the campaign goal; this metadata check is not a second product verification pass. If any required evidence, authorization, cleanup, issue disposition or commit is missing, report exact remaining work and pause with the goal incomplete. Do not repair/retest, select another subset or continue merely because the final gate failed.
