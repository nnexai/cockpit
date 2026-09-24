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

The native goal tracks the full authorized campaign. Follow [repository execution rules](../../.omp/RULES.md); historical single-pass stop rules and per-task ceremony do not govern this campaign. Continue in-scope repairs and verification until the required work is complete or a genuine external blocker remains. Never call `goal.complete` at a task boundary.

Pause is sticky. A paused campaign goal never auto-resumes, including after interruption, recovery, a new worker, or a new orchestrator. Resume only on an explicit user instruction. Do not ask the user to restart the goal, relay worker messages, approve routine safe choices, or manually run the helper. At campaign start, inspect native goal state. If no matching goal exists, create this campaign goal; if it exists and is paused, leave it paused unless the user explicitly resumes it. If an unrelated goal is active or paused, do not silently replace, complete, pause, or repurpose it; report the exact collision for deliberate resolution. Do not create competing campaign orchestrators. Recover from the durable ledger and run evidence without replaying completed work.

The campaign is complete only when all 24 original required task identities (plus any user-authorized required additions) are done, every original in-scope criterion passes on its required surface, all owned implementation/evidence changes have real full-SHA commits, and `ACCEPT-01` is complete. `LATER-GHPR` and `LATER-JIRA` remain `deferred`; keep GitHub issue #6 explicitly partial.

### Explicit user scope amendments

OBS-012 authorizes uniquely marked disposable issue/MR fixtures in project `nnex.ai/integration` (86672117), including their dedicated non-main source branch/commit and controlled fixture content changes. Record exact ownership; production adapters remain GET-only. This does not authorize protected-branch changes, merges, approvals or remote deletion.

The user's 2026-09-22 scope decision removes macOS-specific execution and acceptance from this campaign. Preserve historical criteria and handoff artifacts for reference, but do not schedule macOS work or represent it as passed. No runner acquisition, emulation or substitute verification is required.

The campaign has 24 required task identities. macOS-specific execution and acceptance are outside scope; NATIVE-02 remains recorded as deferred. Linux-native behavior remains required in its owning tasks. Other browser/Herdr/provider/security/performance criteria, owned commits, cleanup and unresolved observations remain required. The final gate audits this explicit user-approved scope, not fictitious macOS passes.

The current live-browser image-quality contract is canonical in [ACCEPTANCE.md](ACCEPTANCE.md#live-browser-image-quality) and applies to WEB-03/04/06. The user's amendment is “attempt to have a sharp image, allow lower resolution to keep up performance”: keep animation, hover, scroll, and input live; accept a current lower-density frame only when identity/geometry barriers pass; and never hold newer content for an older sharper image. This supersedes density-only historical failures as current acceptance criteria, without rewriting old run evidence or marking prior runs passed. Preserve pinned PNG/annotation fidelity and all other acceptance criteria.

OBS-014 calls for focused proof during implementation, not exhaustive matrices at every delivery boundary. Reuse valid evidence; deeper unexecuted scenarios remain unverified until required final acceptance. Historical ownership plans and batch notes are evidence, not current execution limits.

## Bootstrap after the trigger

Perform this bootstrap once for the campaign (and again only when a recovery checkpoint says the authority set changed):

1. Read repository authorities `CONTEXT.md`, `DECISIONS.md`, and `CODE_GUIDE.md`. Before UI changes read `research/ui-design-direction.md` and `research/ui-implementation-constraints.md`. For browser automation, read `skill://playwright-cli`.
2. Read this campaign's `ORCHESTRATOR.md`, `README.md`, `INVENTORY.md`, `OBSERVATIONS.md`, `ACCEPTANCE.md`, `PLAN_VALIDATION.md`, and `tasks.json`. Re-inspect the actual worktree and protect pre-existing changes; dated inventory is not a runtime claim.
3. Confirm that the ledger is the only mutable task-status board. Read the full dependency graph and campaign acceptance map, then only the selected task briefs and relevant dependency evidence. Do not preload all 27 briefs. Preserve every selected brief's original requirements; consult affected briefs before changing scope/dependencies, and never rewrite criteria into a smaller substitute.
4. Run the read-only helper `check` to detect malformed metadata or missing/inconsistent completion records. Use `ready` only as a dependency hint after checking external prerequisites, locks, ownership, and actual source. Helper output never proves behavior or authorizes a claim.
5. Establish the campaign's source baseline from the current worktree and ledger. Record the baseline and protected pre-existing files in the first run evidence; never absorb unrelated changes into a task commit.

After bootstrap, each next selection reads only the relevant task brief, dependency evidence, observations, and authority sections needed for that task. Do not repeatedly load unrelated historical plans or treat old headings, screenshots, agent messages, test totals, or issue state as completion evidence.

## Required task set and dependency discipline

The required set is exactly:

`RUN-01`, `TERM-01`, `TERM-02`, `SYNC-01`, `TERM-03`, `VIEW-01`, `FLOW-01`, `SETUP-01`, `SETUP-02`, `GLAB-01`, `GLAB-02`, `GLAB-03`, `GLAB-04`, `WEB-01`, `WEB-02`, `WEB-03`, `WEB-04`, `WEB-05`, `WEB-06`, `WEB-07`, `WEB-08`, `NATIVE-01`, `PERF-01`, and `ACCEPT-01`. NATIVE-02 remains recorded as deferred, outside this required list.

Keep the exact `depends_on` values in `tasks.json` as the completion dependency graph. Under the user's implementation-first steering in OBS-011, implementation may advance against settled, integrated parent interfaces before the parent's exhaustive acceptance round; an independent slice may use an unchanged existing interface without waiting for unrelated parent UI proof. Record the interface handoff and exclusive writing ownership explicitly. This changes validation timing, not required outcomes: no task is `done` until its original criteria and completion dependencies pass. Never claim readiness from headings, waive safety/authorization prerequisites, or call a compile-only scaffold a completed parent.

## Execution

Work from the remaining ledger outcomes and current evidence. Assign disjoint source ownership and settle shared interfaces before delegation; Main owns integration and shared runtime verification. Writers do not run shared checks against concurrent edits.

Repair demonstrated in-scope defects, then verify affected static checks and real-surface journeys. A failed build leaves dependent runtime checks unrun until fixed; never verify an older binary as the changed product. Preserve all acceptance requirements and existing authorized fixture boundaries.

## Progress, failures and live feedback

`tasks.json` is the only authoritative task ledger. Visible progress must show concrete implemented, verified and committed outcomes tied to the parent task and evidence. Do not mark a parent done from subset success.

Record findings in the owning task and `OBSERVATIONS.md`. Repair in-scope failures; ask only for scope changes or unavailable permissions. Explicit user pauses take effect immediately and require explicit resume.

## Verification resource lifecycle

Follow `.omp/AGENTS.md` and `.omp/RULES.md` for disposable sessions and process-level cleanup. Record exact owned processes, profiles and endpoints in run evidence; report uncertain ownership or leftovers.

## External blockers and authorization

For missing MR fixture/branch authorization, unavailable tools, or missing permissions, discover the narrow prerequisite with read-only probes. macOS execution is outside campaign scope and is not a blocker. Block only the affected in-scope criterion, record attempted discovery and the next unblock action, and continue genuinely independent authorized work. Do not create/push a branch or MR, mutate protected `main`, change protections, merge/delete remote resources, revoke shared access, or broaden production-adapter writes merely to remove a blocker. If human action is unavoidable, make one precise grouped ask after reachable work is exhausted; do not busy-loop, repeatedly ask, or fabricate completion. Authorized disposable fixture mutations require before/after records, explicit ownership, cleanup, and no credential capture. A missing MR blocks MR acceptance only.

## Recovery and context maintenance

On explicit user-authorized resume, inspect the current worktree and ledger, active evidence, referenced commits, resource ownership, and lock state before selecting work. Recover abandoned ownership deliberately and preserve historical evidence. Never auto-resume a paused goal, infer completion from a heading/green test count/agent message/issue state, rerun done work without invalidating evidence, steal a live lock, or start a competing orchestrator.

## Final gate and stop behavior

Before `goal.complete`, reconcile all required task evidence, commits, observations, issue dispositions and cleanup against `ACCEPT-01` and the integrated browser/Linux-native/provider journey. A23/macOS remains excluded; `LATER-GHPR` and `LATER-JIRA` remain deferred; issue #6 stays truthfully partial. Run final integrated checks on settled code and `campaign.py complete` against the recorded result. Failed or missing criteria remain open and require repair or an exact external-blocker report, never a fabricated pass.
