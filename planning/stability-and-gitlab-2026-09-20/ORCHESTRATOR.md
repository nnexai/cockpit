# Orchestrator execution contract

## Start here

Read [README](README.md), [inventory](INVENTORY.md), [acceptance coverage](ACCEPTANCE.md), [task ledger](tasks.json), and the selected task brief. This campaign is implementation-ready planning, not implementation proof. All required tasks start pending. Historical plans and issue-attached patches are evidence, not permission to replay old migrations or overwrite current code.

Read repository authorities (`CONTEXT.md`, `DECISIONS.md`, `CODE_GUIDE.md`) and `skill://incremental-delivery`. For Herdr-backed UI, also read `skill://cockpit-ui-parity` and both UI research authorities. Latest decisions override historical prose: current ANSI protocol-22 transport is not the old graphics experiment; Context/Review replace real extension panes; the browser is inline; legacy browser migration remains excluded. Do not restore old docks, extension runtime, exact patch-version gates, or old native scale defaults.

## Scope and finish line

Required: the 24 `required: true` tasks in `tasks.json`. Deferred: GitHub PR and Jira source expansion (`LATER-GHPR`, `LATER-JIRA`) and macOS-specific execution and acceptance (including `NATIVE-02`), following the user's explicit 2026-09-22 scope decision. None is silently complete. Linux-native behavior remains required in its owning tasks. GitHub #6 must remain open/partially satisfied after GitLab delivery unless its remaining provider scope is separately completed. No new credential store, settings product, remote-access platform, terminal graphics revival, or generic provider framework.

The campaign is complete only when every required task is `done`, each acceptance criterion has passing evidence on its required surface, all owned changes have commits, and ACCEPT-01 reconciles issues and resources. A needed MR fixture or a failed required verification leaves the affected task blocked, not done. Do all independently reachable work meanwhile. Scope reductions require the user's explicit decision, recorded in the observation ledger and task notes.

One-trigger autonomous execution is defined in [AUTORUN.md](AUTORUN.md). Keep one campaign goal active across accepted increments; do not require a new user command for each task. The goal completes only after every required acceptance criterion and the final campaign gate pass.

## Ledger and task states

`tasks.json` is the **only authoritative task-status ledger**. Briefs and the README do not duplicate mutable status. One orchestrator writes the ledger; workers report results but never mark themselves done.

- `pending`: not claimed. Ready when all `depends_on` entries are done and no external prerequisite blocks it.
- `in_progress`: one named worker/integration owner holds its declared locks. Set `owner` and `started_at` (UTC ISO-8601).
- `verifying`: edits integrated; integration owner is running review/static/runtime gates. Still holds locks.
- `blocked`: record precise `blockers`, attempted discovery, and the next unblock action. Keep `owner` non-null while partial edits or retained resources require its locks; the readiness helper conservatively retains all declared locks. Clear `owner` only after a safe committed checkpoint/cleanup leaves no partial edits or exclusive resources; record that release in notes. A blocked task never becomes ready automatically.
- `done`: passing evidence, completed review where required, and actual commit hashes recorded; set `completed_at`, clear blockers, release locks.
- `deferred`: outside required campaign scope; never auto-promote.

Reopen `done` work if later evidence invalidates it. Preserve old evidence and explain the regression in `notes`; clear completion metadata until reverified. A no-code task can be done only after the real acceptance scenario passes and its evidence record has a commit. There is no `done except native`, `mostly done`, or automatic pass for untested features.

Ledger field types: `owner` is a worker/integration-owner name or null; timestamps are UTC ISO-8601 strings or null; `blockers` and `notes` are arrays of concise strings; `evidence` is an array of committed path strings relative to this package (normally `runs/<run-id>/<TASK-ID>.md`); `commits` is an array of full Git SHA strings. Store detailed observations in the evidence file, not arbitrary objects in these arrays. IDs, `required`, dependencies, priority and locks are planning metadata: change them only with an explained scope/decomposition decision, updating coverage and preserving requirements.

Quick inspection from repository root:

```sh
jq -r '.tasks[] | [.id, .status, (.owner // "-"), .title] | @tsv' planning/stability-and-gitlab-2026-09-20/tasks.json
jq -r '. as $b | .tasks[] | select(.required and .status == "pending") | select(all(.depends_on[]; . as $dep | any($b.tasks[]; .id == $dep and .status == "done"))) | [.id, (.locks | join(",")), .title] | @tsv' planning/stability-and-gitlab-2026-09-20/tasks.json
```

The second command reports dependency readiness only. Check platform/fixture prerequisites and locks before dispatching. Update fields through normal reviewed file edits; do not maintain a second checkbox board.

For automated execution prefer `python3 planning/stability-and-gitlab-2026-09-20/campaign.py ready`; unlike the jq dependency query it also accounts for held locks. `check` validates ledger and recorded completion evidence; `complete` additionally rejects any unfinished required task. These are read-only bookkeeping gates, not substitutes for scenario proof or independent review. Ready candidates may conflict with each other: claim/recheck or explicitly select disjoint ownership before dispatch.

## Scheduling and file ownership

Dependencies express prerequisite behavior/evidence. `locks` express potential write collisions, not extra semantic dependencies. Inspect the current source before assigning exclusive paths/symbols; lock groups are conservative, not permission to edit every file in them. The host/native/generated-protocol boundary has one integration owner. Acquire `protocol` whenever a worker discovers a wire-contract change, even if it was not anticipated by its task.

Lock groups:

| Lock | Main collision area |
| --- | --- |
| `app-shell` | `src/app/App.tsx`, pane projection and action integration |
| `terminal`, `herdr-adapter` | TerminalPane/input lifecycle; Herdr CLI/wire adapter respectively |
| `client-transports`, `host-streams` | shared client interface/browser/native adapters; gateway/Tauri stream lifecycle |
| `browser-ui`, `browser-helper`, `frame-presenter`, `browser-store` | BrowserPane; Node helper; frame presenter; core browser draft/feedback/delivery storage |
| `browser-runtime-launch`, `config-host` | core/host browser launch and dependency configuration/composition |
| `setup-ui`, `core-projects`, `filesystem-publication` | setup dialog; project/defaults/journals; companion no-replace filesystem writes |
| `source-identity`, `providers`, `protocol` | artifact/origin authority; provider adapters; Rust DTOs/generated TS/client validation |
| `context-review`, `comment-delivery` | Context/Review/source resources UI; durable comments and paste receipts |
| `installer` | native install script, tests, install documentation |
| `verification-runtime`, `gitlab-fixtures` | shared verification setup; mutations of authorized external test fixtures |

Documentation authority updates and generated files are integration-owner writes, serialized separately. `verification-runtime` is a default shared acceptance lock: an orchestrator may split it into run-specific locks only after assigning disjoint session/config/ports/displays/profiles and documenting those identities. Native and browser must never accidentally connect to the user's active session.

Suggested ready-work progression (not an extra dependency chain):

1. RUN-01 establishes baseline. TERM-01 fixes compatibility while NATIVE-01 and WEB-07 can independently prepare their outcomes.
2. TERM-02, SETUP-01, WEB-01, WEB-02, WEB-03 and VIEW-01 become dependency-ready. Choose disjoint ownership: TERM-02 and VIEW-01 can run together; WEB-02 and WEB-03 share files and cannot run as writers together; SETUP-01 and WEB-01 can collide with TERM-02 in App.
3. After SETUP-01, GLAB-01 can run alongside browser or terminal work **only** if neither owns shared core/protocol paths. SETUP-02 shares core-projects and is serialized with GLAB-01. MR/refresh increments follow their declared dependencies.
4. SYNC-01 owns several shared lifecycle seams; drain overlapping writers before running it. Independent installer/provider work can continue when their contracts do not cross those seams.
5. Finish terminal/browser/viewer correctness, authenticated GitLab proof, and local workflows. Run platform, security, performance and integrated gates on the resulting code, not a moving target.

Do not invent padding work to fill workers. A task spanning too many shared files can be split only into independently observable outcomes; update IDs, dependencies, issue coverage and acceptance ownership together, preserving original requirements. Never create a compile-only scaffold task and call its parent feature complete.

Before implementation, the selected increment must have an accepted plan under the mandatory planning round below. Its run evidence records the exact user-visible outcome, original acceptance criteria covered, non-goals, exclusive files, positive and negative scenario, required surfaces, and expected observable result. For broad tasks, execute one such increment at a time and preserve the remaining criteria. Append results and commit references after each accepted increment; keep the parent task `in_progress` until its complete original contract passes. Do not introduce a second mutable status ledger. Continue to the next increment/task without a user handoff.

## Mandatory planning round and dispatch gate

Planning is a just-in-time control for each selected dependency-ready task, including a no-code verification task; it is not speculative detailed planning for every task up front. After safe selection and before any worker edits, execution, service start, fixture mutation, or other implementation action, the strong orchestrator conducts a planning round and writes the plan into the existing `runs/<run-id>/<TASK-ID>.md` record. The plan is run evidence, not a new ledger state, status value, mutable board, or approval queue.

Planning and plan acceptance belong to **Sol or Astra**, never Luna or a cheaper implementation worker. The Sol/Astra orchestrator owns top-level decomposition and acceptance; it may request bounded read-only design input through the enabled Astra advisor when needed, following its consultation limits. Read-only scouts gather facts, not design decisions. A trivial or lone task does not require a planning subagent. Routine user approval is not a dispatch prerequisite.

The planning round must inspect the current source/worktree, repository authorities, selected brief, dependency evidence, observations, and relevant locks/resources. It resolves consequential unknowns before dispatch and records:

1. The current source/commit baseline and requirement-to-increment coverage, preserving every original acceptance criterion and explicit non-goal.
2. Exact writable paths and symbols, current patterns and callsites, shared seams, ownership/lock boundaries, and any API/data identities that must remain compatible.
3. The chosen design, APIs and invariants, with ordered bounded implementation steps and any parallel ownership that is genuinely disjoint.
4. Edge, error, lifecycle, concurrency, authorization, cleanup, and platform cases, plus positive and negative checks on each required real surface and their expected observations.
5. Required resources/permissions and fixture identities, the cheapest capable configured worker profile, its self-contained implementation recipe, and explicit escalation/stop boundaries.

Dispatch is hard-gated: no implementation worker or execution may start until the orchestrator has accepted the current plan and no design decision remains unresolved for the assigned slice. The worker receives the accepted recipe, authorities, baseline, dependencies, non-goals, checks, resources, and escalation boundaries; do not assume every task is safe for the weakest model or change global routing merely to economize. Missing prerequisites block only the affected slice while independent work continues.

Revalidate the relevant source, authorities, dependencies, locks, and plan baseline before every increment and when resuming. If source or interface evidence drifts, a dependency/authority changes, or the plan becomes stale, stop dispatch for the affected slice, reconcile and replan it before continuing. Nontrivial ambiguity, an unexpected interface/source change, or repeated failure returns the affected slice to strong planning; stop that slice, record the reason and evidence, and continue independent work. A successful planner or worker message never substitutes for plan acceptance or behavior proof.

No-code work follows the same gate: plan the exact evidence-producing action, source/authority basis, real-surface positive and negative checks, resource/authorization boundaries, and completion evidence before dispatch. Do not add task statuses or a second board to represent planning; `tasks.json` remains the only authoritative task-status ledger.

The concise autorun enforcement is in [AUTORUN.md](AUTORUN.md); this section is authoritative when the two descriptions meet.


## Worker contract

Use only enabled bundled agents and the retained `astra-advisor`; disabled project-specific profiles are not dependencies and must not be re-enabled or substituted silently. Inspect the configured model for an agent before assigning its role: an agent name or higher effort setting alone does not establish Sol/Astra planning capability. Use the cheapest capable bundled implementation worker for the accepted recipe, `scout` for factual exploration, and a read-only reviewer for risky integrated changes. If no Sol/Astra planner is available, block planning rather than downgrade it. Keep advisory depth available as required by repository rules. Do not change global model routing. A worker brief must include:

1. Exact task ID, outcome, issue links, source baseline and evidence distinctions.
2. Exclusive writable paths/symbols and non-goals; shared API/data identities fixed before siblings start.
3. Relevant task brief, authority files, prior dependency evidence, and acceptance criteria.
4. **Skip formatters, linters, builds, tests, services, and commits during a concurrent writing wave.** The integration owner runs each gate once after integration. No worker mutates user sessions, installs live binaries, or changes remote issues/MRs unless explicitly assigned a scoped fixture action.
5. Return changed paths, implemented behavior, unresolved risks, proposed focused checks, and consequential advisor recommendations. A successful worker exit is not acceptance.

Keep interpreting user intent at the orchestrator. Do not outsource top-level decomposition or let workers negotiate incompatible shared contracts. Never test while concurrent writers are changing the same integration surface.

## Verification and commit discipline

For every bug, capture a safe reproduction/negative control before repair, then show it no longer triggers. User-reported failures are ground truth: reproduction work establishes measurable acceptance, not whether to believe the report. Source-review candidates must be exercised before claiming a user-visible defect. Preserve passing behavior rather than rewrite a feature merely because the task exists.

After integration, run narrow relevant static checks, retained behavior regressions, and the real user action through the authoritative response/event to the rendered success/failure state. Browser proof is required for shared UI; real Linux-native proof is required for native commands/channels/startup/platform input/image decode. macOS-specific behavior is outside this campaign's scope. Compare Herdr semantics in a uniquely named disposable TUI session. Run `skill://playwright-cli` when using that browser workflow, or the harness browser API for direct browser verification.

Existing commands (choose affected scope; do not run everything per task): `bun run typecheck`, `bun run test -- <files>`, `cargo test -p <package> <filter>`, `cargo fmt --all -- --check`, `node --check browser-runtime/browser-helper.mjs`. A wire change regenerates TypeScript with `cargo run -q -p cockpit-protocol --bin export-typescript -- --write src/protocol/generated/v1.ts`. Integrated gates include `bun run build`, `cargo test --workspace --exclude cockpit-tauri`, and `cargo check -p cockpit-tauri`; builds are not runtime proof. Native installer tests use the existing Python suite. Respect pinned toolchains and current project conventions.

Keep new permanent tests only for plausible behavioral failures; avoid source-text/default/wiring assertions. Straightforward feature proof can use disposable scripts. After a passing smoke, perform the task's necessary docs/test/scaffold cleanup as the final phase; do not pre-allocate unrelated cleanup projects. Update authorities only for deliberate behavior changes, not to overwrite historical evidence.

Copy [the evidence template](templates/evidence.md) to `runs/<run-id>/<TASK-ID>.md` and commit the compact evidence (no credentials or giant captures). Raw captures go in a run-owned artifact directory; record durable location/hash and observed results. Session-local `artifact://`/`agent://` links alone are not a durable handoff.

Commit only the task's owned changes after its gates pass. Record the resulting real commit SHA and evidence path in `tasks.json`; checkpoint ledger changes in a planning/status commit or the next owned commit. Do not attempt to embed a commit's own hash in itself or amend endlessly. Each implementation increment has its own reviewable commit; a documentation status checkpoint does not replace it.

## External resources and permissions

The protected default/active Herdr session, user's browser profiles, installed applications and manual gateways are never test fixtures. Reuse `scripts/verify/resource_guard.py` and existing verification helpers. Record every created session/socket/process/profile/repository/worktree/display and clean up only recorded ownership. Do not leave services running without an explicit retained-resource reason.

Authorized GitLab project: `https://gitlab.com/nnex.ai/integration`, project ID `86672117`. Fixture issue `#1` is already created by this campaign, open, with marker `cockpit-glab-2026-09-20-a17b`; canonical URL is `https://gitlab.com/nnex.ai/integration/-/work_items/1`. GET `/projects/86672117/issues/1` returns `issue_type: issue`. Preserve the work-item URL, but never assume every work item is an issue. User permits test issue creation; controlled fixture descriptions/comments may be used to test refresh, with before/after evidence. Production provider code is GET-only and must never inherit this test permission.

The project had only protected `main`, no MRs, at inventory. No permission to push a branch/create an MR was recorded: discover an existing authorized MR first, otherwise ask the user for a disposable branch/MR fixture before those writes. Never push main, change protections, force-push, merge an MR, or delete the project. A missing fixture blocks only the affected real-MR acceptance; finish reachable provider/issue work. Close the owned issue after its last consumer, not while another task needs it. Keep a fixture ledger in the run evidence.

GitHub issue comments/closure are not implied by repository write access. ACCEPT-01 prepares exact closure evidence; publish changes only with established permission. Patches in issues/gists must be reviewed and adapted, not blindly applied. Do not close #6 after only GitLab is delivered.

## Live observations and resuming

Append user observations to [OBSERVATIONS](OBSERVATIONS.md) with a stable ID, affected task and acceptance impact. Queue ordinary feedback; interrupt only the worker whose contract became unsafe/invalid. Every in-scope finding must be fixed in an existing task or tracked in an explicit new required task before campaign closure. No invisible follow-up bucket.

On resume, inspect current source/worktree and the ledger, read the active task's evidence, recover abandoned ownership deliberately, and verify that referenced commits/resources still exist. Never infer done from a heading, old green test count, agent completion message, or closed GitHub issue. Persist a compact handoff in the active run evidence before switching orchestrators.
