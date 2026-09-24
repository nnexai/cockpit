# Orchestrator execution contract

## Start here

Read [README](README.md), [inventory](INVENTORY.md), [acceptance coverage](ACCEPTANCE.md), [task ledger](tasks.json), and the selected task brief. Planning is not implementation proof; current task state comes only from the ledger. Historical plans and issue-attached patches are evidence, not permission to reset task state, replay old migrations, or overwrite current code.

Read repository authorities (`CONTEXT.md`, `DECISIONS.md`, `CODE_GUIDE.md`) and both UI research authorities for UI work. Latest decisions override historical prose: current ANSI protocol-22 transport is not the old graphics experiment; Context/Review replace real extension panes; the browser is inline; legacy browser migration remains excluded. Do not restore old docks, extension runtime, exact patch-version gates, or old native scale defaults.

## Scope and finish line

Required: the 24 `required: true` tasks in `tasks.json`. Deferred: GitHub PR and Jira source expansion (`LATER-GHPR`, `LATER-JIRA`) and macOS-specific execution and acceptance (including `NATIVE-02`), following the user's explicit 2026-09-22 scope decision. None is silently complete. Linux-native behavior remains required in its owning tasks. GitHub #6 must remain open/partially satisfied after GitLab delivery unless its remaining provider scope is separately completed. No new credential store, settings product, remote-access platform, terminal graphics revival, or generic provider framework.

The campaign is complete only when every required task is `done`, all required criteria have passing evidence, owned changes have commits and ACCEPT-01 reconciles issues/resources. Follow [repository execution rules](../../.omp/RULES.md) and [AUTORUN.md](AUTORUN.md#execution); do not narrow acceptance or stop solely because verification found a repairable defect.

## Ledger and task states

`tasks.json` is the **only authoritative task-status ledger**. Briefs and the README do not duplicate mutable status. One orchestrator writes the ledger; workers report results but never mark themselves done.

- `pending`: never claimed. Ready when all `depends_on` entries are done and no external prerequisite blocks it.
- `queued`: previously started, safely checkpointed and unowned; retains its historical evidence, commits and `started_at` without holding locks. It has no external blockers. `ready` lists it as a candidate only when dependencies are done and locks are free; otherwise it is waiting.
- `in_progress`: one named worker/integration owner holds its declared locks. Set `owner` and `started_at` (UTC ISO-8601).
- `verifying`: edits integrated; integration owner is running the focused checks and actual-surface acceptance for the bounded contract. Still holds locks.
- `blocked`: an actual unavailable prerequisite or failed gate prevents the next affected action; record precise `blockers`, attempted discovery, and the next unblock action. Keep `owner` non-null while partial edits or retained resources require its locks; the readiness helper conservatively retains all declared locks. Clear `owner` only after a safe committed checkpoint/cleanup leaves no partial edits or exclusive resources; record that release in notes. A blocked task never becomes ready automatically.
- `done`: passing evidence and actual commit hashes recorded; set `completed_at`, clear blockers, release locks.
- `deferred`: outside required campaign scope; never auto-promote.

Reopen `done` work if later evidence invalidates it. Preserve old evidence and explain the regression in `notes`; clear completion metadata until reverified. A no-code task can be done only after the real acceptance scenario passes and its evidence record has a commit. There is no `done except native`, `mostly done`, or automatic pass for untested features.

Visible progress projects concrete implemented, verified and committed outcomes from the ledger and evidence. Subset success cannot mark its parent done. A failed test is repair work, not by itself an external blocker.

Ledger field types: `owner` is a worker/integration-owner name or null; timestamps are UTC ISO-8601 strings or null; `blockers` and `notes` are arrays of concise strings; `evidence` is an array of committed path strings relative to this package (normally `runs/<run-id>/<TASK-ID>.md`); `commits` is an array of full Git SHA strings. Store detailed observations in the evidence file, not arbitrary objects in these arrays. IDs, `required`, dependencies, priority and locks are planning metadata: change them only with an explained scope/decomposition decision, updating coverage and preserving requirements.

Quick inspection from repository root:

```sh
jq -r '.tasks[] | [.id, .status, (.owner // "-"), .title] | @tsv' planning/stability-and-gitlab-2026-09-20/tasks.json
jq -r '. as $b | .tasks[] | select(.required and (.status == "pending" or .status == "queued")) | select(all(.depends_on[]; . as $dep | any($b.tasks[]; .id == $dep and .status == "done"))) | [.id, (.locks | join(",")), .title] | @tsv' planning/stability-and-gitlab-2026-09-20/tasks.json
```

The second command reports dependency readiness only. Check platform/fixture prerequisites and locks before dispatching. Update fields through normal reviewed file edits; do not maintain a second checkbox board.

For automated execution prefer `python3 planning/stability-and-gitlab-2026-09-20/campaign.py ready`; unlike the jq dependency query it also accounts for held locks. `check` validates ledger and recorded completion evidence; `complete` additionally rejects any unfinished required task. These are bookkeeping checks, not behavioral proof. Ready candidates may conflict: claim/recheck or explicitly select disjoint ownership before dispatch.

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


## Worker contract

Use only enabled bundled agents and the configured `task-advisor`; disabled project-specific profiles are not dependencies and must not be re-enabled or substituted silently. Inspect the configured model before assigning its role. Use the cheapest capable implementation worker for the bounded contract, `scout` for factual exploration, and a read-only reviewer only for a named concrete security, data-loss, ownership, or concurrency risk. Preserve the existing advisor model, advisory depth, and global model routing. A worker brief must include:

1. Exact task ID, outcome, issue links, source baseline and evidence distinctions.
2. Exclusive writable paths/symbols and non-goals; shared API/data identities fixed before siblings start.
3. Relevant task brief, authority files, prior dependency evidence, and acceptance criteria.
4. Skip builds/tests/formatters/linters/runtime acceptance/commits while concurrent writers are active; required code generation remains in scope. Return changes for integration. Do not mutate user sessions, install live binaries or change remote issues/MRs unless explicitly assigned a scoped fixture action.
5. Return changed paths, implemented behavior, unresolved risks, proposed focused checks, and consequential advisor recommendations. A successful worker exit is not acceptance.

Keep interpreting user intent at the orchestrator. Do not outsource top-level decomposition or let workers negotiate incompatible shared contracts. Never run shared checks against an integration surface while concurrent writers are changing it.

## Verification and commit discipline

User-reported failures are ground truth. Diagnose only unknowns needed for the repair; reuse existing reproductions. Verify affected behavior after integration through actual user actions, authoritative responses and rendered results. Shared UI requires browser proof; native-specific behavior requires Linux-native proof. Never verify a stale binary after a failed build.

Choose affected-scope checks only; project-wide validation belongs to the main orchestrator once the assigned work has settled. Existing focused commands include `bun run typecheck`, `bun run test -- <files>`, `cargo test -p <package> <filter>`, `cargo fmt --all -- --check`, and `node --check browser-runtime/browser-helper.mjs`. A wire change regenerates TypeScript with `cargo run -q -p cockpit-protocol --bin export-typescript -- --write src/protocol/generated/v1.ts`. Integrated gates include `bun run build`, `cargo test --workspace --exclude cockpit-tauri`, and `cargo check -p cockpit-tauri`; builds are not runtime proof. Native installer checks use the existing Python suite. Respect pinned toolchains and current project conventions.

Keep new permanent tests only for plausible behavioral failures; avoid source-text/default/wiring assertions. Straightforward feature proof can use disposable scripts. After a passing smoke, perform necessary docs/test/scaffold cleanup as the final phase; do not pre-allocate unrelated cleanup projects. Update authorities only for deliberate behavior changes, not to overwrite historical evidence.

Copy [the evidence template](templates/evidence.md) to `runs/<run-id>/<TASK-ID>.md` and commit compact evidence (no credentials or giant captures). Raw captures go in a run-owned artifact directory; record durable location/hash and observed results. Session-local `artifact://`/`agent://` links alone are not a durable handoff.

Commit only verified owned changes; record full SHAs and evidence paths in `tasks.json` without a self-SHA loop or ceremony-only commits. Failed or unrun criteria remain open while in-scope repair continues.

## External resources and permissions

Follow repository fixture and cleanup rules. Reuse owned environments and record process/profile/session/endpoint ownership in run evidence. Installed apps, manual gateways, user profiles and active Herdr sessions are never fixtures.

Authorized GitLab project: `https://gitlab.com/nnex.ai/integration`, project ID `86672117`. Fixture issue `#1` is already created by this campaign, open, with marker `cockpit-glab-2026-09-20-a17b`; canonical URL is `https://gitlab.com/nnex.ai/integration/-/work_items/1`. GET `/projects/86672117/issues/1` returns `issue_type: issue`. Preserve the work-item URL, but never assume every work item is an issue. User permits test issue creation; controlled fixture descriptions/comments may be used to test refresh, with before/after evidence. Production provider code is GET-only and must never inherit this test permission.

The project had only protected `main`, no MRs, at inventory. No permission to push a branch/create an MR was recorded: discover an existing authorized MR first, otherwise ask the user for a disposable branch/MR fixture before those writes. Never push main, change protections, force-push, merge an MR, or delete the project. A missing fixture blocks only the affected real-MR acceptance; finish reachable provider/issue work. Close the owned issue after its last consumer, not while another task needs it. Keep a fixture ledger in the run evidence.

GitHub issue comments/closure are not implied by repository write access. ACCEPT-01 prepares exact closure evidence; publish changes only with established permission. Patches in issues/gists must be reviewed and adapted, not blindly applied. Do not close #6 after only GitLab is delivered.

## Live observations and resuming

Record observations with stable IDs in [OBSERVATIONS](OBSERVATIONS.md) and their owning task. Repair in-scope defects; scope expansion requires the user's decision. Interrupt unsafe operations or invalid contracts.

Explicit pause, stop, handoff, or workflow corrections take effect immediately. They are not ordinary queued observations. Todo reminders and automatic continuation messages do not authorize resuming a paused run.

On explicit user-authorized resume, inspect current source/worktree and the ledger, read the active task's evidence, recover abandoned ownership deliberately, and verify that referenced commits/resources still exist. Never infer done from a heading, old green test count, agent completion message, or closed GitHub issue. A paused campaign goal never resumes automatically. Persist a compact handoff in the active run evidence before switching orchestrators.
