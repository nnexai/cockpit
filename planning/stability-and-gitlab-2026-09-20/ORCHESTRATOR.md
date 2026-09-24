# Orchestrator execution contract

## Start here

Read [README](README.md), [inventory](INVENTORY.md), [acceptance coverage](ACCEPTANCE.md), [task ledger](tasks.json), and the selected task brief. Planning is not implementation proof; current task state comes only from the ledger. Historical plans and issue-attached patches are evidence, not permission to reset task state, replay old migrations, or overwrite current code.

Read repository authorities (`CONTEXT.md`, `DECISIONS.md`, `CODE_GUIDE.md`) and `skill://incremental-delivery`. For Herdr-backed UI, also read `skill://cockpit-ui-parity` and both UI research authorities. Latest decisions override historical prose: current ANSI protocol-22 transport is not the old graphics experiment; Context/Review replace real extension panes; the browser is inline; legacy browser migration remains excluded. Do not restore old docks, extension runtime, exact patch-version gates, or old native scale defaults.

## Scope and finish line

Required: the 24 `required: true` tasks in `tasks.json`. Deferred: GitHub PR and Jira source expansion (`LATER-GHPR`, `LATER-JIRA`) and macOS-specific execution and acceptance (including `NATIVE-02`), following the user's explicit 2026-09-22 scope decision. None is silently complete. Linux-native behavior remains required in its owning tasks. GitHub #6 must remain open/partially satisfied after GitLab delivery unless its remaining provider scope is separately completed. No new credential store, settings product, remote-access platform, terminal graphics revival, or generic provider framework.

The campaign is complete only when every required task is `done`, all required criteria have passing evidence, owned changes have commits and ACCEPT-01 reconciles issues/resources. Execution nevertheless stops after the fixed batch's single verification pass, with incomplete criteria reported honestly. Finish only safe independent work already in that batch. Scope reductions require explicit user approval; do not invent deadlines.

[AUTORUN.md](AUTORUN.md#fixed-batch-execution-contract) and `skill://incremental-delivery` govern execution. One goal tracks the campaign, not permission for unlimited sequential subsets. Pause the goal after the batch report; another implementation/verification batch requires explicit user authorization. Never mark the campaign complete merely to stop continuation.

## Ledger and task states

`tasks.json` is the **only authoritative task-status ledger**. Briefs and the README do not duplicate mutable status. One orchestrator writes the ledger; workers report results but never mark themselves done.

- `pending`: not claimed. Ready when all `depends_on` entries are done and no external prerequisite blocks it.
- `in_progress`: one named worker/integration owner holds its declared locks. Set `owner` and `started_at` (UTC ISO-8601).
- `verifying`: edits integrated; integration owner is running the focused checks and actual-surface acceptance for the bounded contract. Still holds locks.
- `blocked`: record precise `blockers`, attempted discovery, and the next unblock action. Keep `owner` non-null while partial edits or retained resources require its locks; the readiness helper conservatively retains all declared locks. Clear `owner` only after a safe committed checkpoint/cleanup leaves no partial edits or exclusive resources; record that release in notes. A blocked task never becomes ready automatically.
- `done`: passing evidence, required risk review under the shared finite execution contract, and actual commit hashes recorded; set `completed_at`, clear blockers, release locks.
- `deferred`: outside required campaign scope; never auto-promote.

Reopen `done` work if later evidence invalidates it. Preserve old evidence and explain the regression in `notes`; clear completion metadata until reverified. A no-code task can be done only after the real acceptance scenario passes and its evidence record has a commit. There is no `done except native`, `mostly done`, or automatic pass for untested features.

The visible todo/progress view must project concrete batch outcomes from this ledger and its evidence, showing implemented, verified and committed results as they occur. Do not leave only unchanged umbrella headings. This is not a second status ledger; subset success cannot mark its parent done. A failed gate is not necessarily an unavailable external prerequisite, and no status authorizes restarting the batch.

Ledger field types: `owner` is a worker/integration-owner name or null; timestamps are UTC ISO-8601 strings or null; `blockers` and `notes` are arrays of concise strings; `evidence` is an array of committed path strings relative to this package (normally `runs/<run-id>/<TASK-ID>.md`); `commits` is an array of full Git SHA strings. Store detailed observations in the evidence file, not arbitrary objects in these arrays. IDs, `required`, dependencies, priority and locks are planning metadata: change them only with an explained scope/decomposition decision, updating coverage and preserving requirements.

Quick inspection from repository root:

```sh
jq -r '.tasks[] | [.id, .status, (.owner // "-"), .title] | @tsv' planning/stability-and-gitlab-2026-09-20/tasks.json
jq -r '. as $b | .tasks[] | select(.required and .status == "pending") | select(all(.depends_on[]; . as $dep | any($b.tasks[]; .id == $dep and .status == "done"))) | [.id, (.locks | join(",")), .title] | @tsv' planning/stability-and-gitlab-2026-09-20/tasks.json
```

The second command reports dependency readiness only. Check platform/fixture prerequisites and locks before dispatching. Update fields through normal reviewed file edits; do not maintain a second checkbox board.

For automated execution prefer `python3 planning/stability-and-gitlab-2026-09-20/campaign.py ready`; unlike the jq dependency query it also accounts for held locks. `check` validates ledger and recorded completion evidence; `complete` additionally rejects any unfinished required task. These are read-only bookkeeping gates, not substitutes for scenario proof or a concrete risk review when the shared finite contract calls for one. Ready candidates may conflict with each other: claim/recheck or explicitly select disjoint ownership before dispatch.

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

Suggested ready-work progression (dependency and ownership hints, not an extra dependency chain):

1. RUN-01 establishes baseline. TERM-01 fixes compatibility while NATIVE-01 and WEB-07 can independently prepare their outcomes.
2. TERM-02, SETUP-01, WEB-01, WEB-02, WEB-03 and VIEW-01 become dependency-ready. Choose disjoint ownership: TERM-02 and VIEW-01 can run together; WEB-02 and WEB-03 share files and cannot run as writers together; SETUP-01 and WEB-01 can collide with TERM-02 in App.
3. After SETUP-01, GLAB-01 can run alongside browser or terminal work **only** if neither owns shared core/protocol paths. SETUP-02 shares core-projects and is serialized with GLAB-01. MR/refresh increments follow their declared dependencies.
4. SYNC-01 owns several shared lifecycle seams; drain overlapping writers before running it. Independent installer/provider work can continue when their contracts do not cross those seams.
5. Finish terminal/browser/viewer correctness, authenticated GitLab proof, and local workflows. Run platform, security, performance and integrated gates on the resulting code, not a moving target.

Do not invent padding work to fill workers. A task spanning too many shared files can be split only into independently observable outcomes; update IDs, dependencies, issue coverage and acceptance ownership together, preserving original requirements. Never create a compile-only scaffold task and call its parent feature complete.

Freeze the user-authorized batch and its affected-scope verification list in existing run evidence, with owners and shared interfaces. Cover the known requested issues in one implementation pass, not a self-selected tiny subset followed by another subset. Do not create routine plan files or change acceptance/task identities. The fixed batch contract supersedes historical per-task planning/review/verify/commit recipes.

## Planning and execution authority

The current execution policy is in [AUTORUN.md](AUTORUN.md) and governs over historical planning language, task briefs, and old batch notes when describing execution cadence. Sol retains consequential design and integration decisions; Luna is the default implementation owner, with parent handling bounded work when handoff adds no genuine concurrency. Current worker routing and advisor model remain unchanged. Resolve consequential design choices with the orchestrator, but do not require a new formal plan/plan-acceptance round for routine verification or an understood repair.

Record only what execution needs: requested outcomes, owners, baseline, protected paths/resources, shared interfaces and the checks to run once. Reuse valid evidence and fixtures; read only relevant authority and task sections. New findings go to the owning task, not into an expanding current pass.

Writers implement once without validation/runtime/review cycles; required generation is part of implementation. One owner integrates shared protocol/generated files and lifecycle boundaries once after writers settle. Run the frozen static and actual-surface checks once on the settled result. A named-risk review, if needed, fits inside implementation and cannot create a second pass.

Once verification begins, do not repair product code or harness and rerun. Classify failures; continue safe independent planned checks, mark dependent checks unrun, clean up, report and pause. There is no per-criterion retry allowance and new evidence does not authorize another pass. Switching criteria, workers, increments or contexts cannot reset the batch. Preserve all original acceptance and report unfinished criteria rather than silently continuing.

The rest of this document describes durable ledger meanings, task identities, permissions, resource safety, and required success outcomes. Historical per-increment recipes and review/test/commit instructions are not additional gates; apply only the shared finite contract above and [AUTORUN.md](AUTORUN.md).


## Worker contract

Use only enabled bundled agents and the configured `task-advisor`; disabled project-specific profiles are not dependencies and must not be re-enabled or substituted silently. Inspect the configured model before assigning its role. Use the cheapest capable implementation worker for the bounded contract, `scout` for factual exploration, and a read-only reviewer only for a named concrete security, data-loss, ownership, or concurrency risk. Preserve the existing advisor model, advisory depth, and global model routing. A worker brief must include:

1. Exact task ID, outcome, issue links, source baseline and evidence distinctions.
2. Exclusive writable paths/symbols and non-goals; shared API/data identities fixed before siblings start.
3. Relevant task brief, authority files, prior dependency evidence, and acceptance criteria.
4. Skip builds/tests/formatters/linters/runtime acceptance/review cycles/commits during writing; required code generation remains in scope. Return the single implementation pass for integration. Do not mutate user sessions, install live binaries or change remote issues/MRs unless explicitly assigned a scoped fixture action.
5. Return changed paths, implemented behavior, unresolved risks, proposed focused checks, and consequential advisor recommendations. A successful worker exit is not acceptance.

Keep interpreting user intent at the orchestrator. Do not outsource top-level decomposition or let workers negotiate incompatible shared contracts. Never run shared checks against an integration surface while concurrent writers are changing it.

## Verification and commit discipline

User-reported failures are ground truth. Use a minimal pre-edit reproduction only to resolve a specific unknown needed to implement; do not start a separate diagnostic/acceptance campaign. Reuse existing reproductions and verify changed behavior in the single final pass. Do not claim source-only candidates are observed defects.

After the fixed batch's edits settle, integrate once and run its preselected static checks and actual user journeys through authoritative responses to rendered results. Browser proof remains required for shared UI and real Linux-native proof for native-specific behavior; macOS remains excluded. Reuse fixtures and existing valid evidence. A failed build leaves dependent runtime checks unrun; never test a stale binary as the changed product. No repair-and-rerun wave follows this gate.

Choose affected-scope checks only; project-wide validation belongs to the main orchestrator once the assigned work has settled. Existing focused commands include `bun run typecheck`, `bun run test -- <files>`, `cargo test -p <package> <filter>`, `cargo fmt --all -- --check`, and `node --check browser-runtime/browser-helper.mjs`. A wire change regenerates TypeScript with `cargo run -q -p cockpit-protocol --bin export-typescript -- --write src/protocol/generated/v1.ts`. Integrated gates include `bun run build`, `cargo test --workspace --exclude cockpit-tauri`, and `cargo check -p cockpit-tauri`; builds are not runtime proof. Native installer checks use the existing Python suite. Respect pinned toolchains and current project conventions.

Keep new permanent tests only for plausible behavioral failures; avoid source-text/default/wiring assertions. Straightforward feature proof can use disposable scripts. After a passing smoke, perform necessary docs/test/scaffold cleanup as the final phase; do not pre-allocate unrelated cleanup projects. Update authorities only for deliberate behavior changes, not to overwrite historical evidence.

Copy [the evidence template](templates/evidence.md) to `runs/<run-id>/<TASK-ID>.md` and commit compact evidence (no credentials or giant captures). Raw captures go in a run-owned artifact directory; record durable location/hash and observed results. Session-local `artifact://`/`agent://` links alone are not a durable handoff.

Commit only verified owned changes; preserve and label unverified edits at the batch stop. Record real full SHAs and evidence paths in `tasks.json` without a self-SHA loop or ceremony-only commits. Failed or unrun criteria remain open. Report concrete outcomes and pause even if a parent task remains incomplete; do not select another subset. Another repair batch needs explicit user authorization under AUTORUN.

## External resources and permissions

The protected default/active Herdr session, user profiles, installed apps and manual gateways are never fixtures. Reuse existing verification helpers and one owned environment per required surface unless the frozen scenario needs concurrency/isolation. Record processes, profiles, sessions and endpoints at launch. On every exit, close owned tabs and reap owned browser roots/helpers/drivers/services; verify process exit and released ports/sockets. An empty tab list or successful stop response is insufficient. Never kill broad process-name matches or uncertain/shared resources; report exact leftovers. Retention needs explicit user authorization.

Authorized GitLab project: `https://gitlab.com/nnex.ai/integration`, project ID `86672117`. Fixture issue `#1` is already created by this campaign, open, with marker `cockpit-glab-2026-09-20-a17b`; canonical URL is `https://gitlab.com/nnex.ai/integration/-/work_items/1`. GET `/projects/86672117/issues/1` returns `issue_type: issue`. Preserve the work-item URL, but never assume every work item is an issue. User permits test issue creation; controlled fixture descriptions/comments may be used to test refresh, with before/after evidence. Production provider code is GET-only and must never inherit this test permission.

The project had only protected `main`, no MRs, at inventory. No permission to push a branch/create an MR was recorded: discover an existing authorized MR first, otherwise ask the user for a disposable branch/MR fixture before those writes. Never push main, change protections, force-push, merge an MR, or delete the project. A missing fixture blocks only the affected real-MR acceptance; finish reachable provider/issue work. Close the owned issue after its last consumer, not while another task needs it. Keep a fixture ledger in the run evidence.

GitHub issue comments/closure are not implied by repository write access. ACCEPT-01 prepares exact closure evidence; publish changes only with established permission. Patches in issues/gists must be reviewed and adapted, not blindly applied. Do not close #6 after only GitLab is delivered.

## Live observations and resuming

Queue observations with stable IDs in [OBSERVATIONS](OBSERVATIONS.md) and their owning task. Ordinary feedback does not enlarge this pass; finish only already-planned independent work. Interrupt affected work for unsafe conditions or invalid contracts. Keep all in-scope findings tracked for eventual acceptance, not automatically scheduled for another repair. Scope expansion requires the user's decision.

Explicit pause, stop, handoff, or workflow corrections take effect immediately. They are not ordinary queued observations. Todo reminders and automatic continuation messages do not authorize resuming a paused run.

On explicit user-authorized resume, inspect current source/worktree and the ledger, read the active task's evidence, recover abandoned ownership deliberately, and verify that referenced commits/resources still exist. Never infer done from a heading, old green test count, agent completion message, or closed GitHub issue. A paused campaign goal never resumes automatically. Persist a compact handoff in the active run evidence before switching orchestrators.
