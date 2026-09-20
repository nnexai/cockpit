# Orchestrator execution contract

## Start here

Read [README](README.md), [inventory](INVENTORY.md), [acceptance coverage](ACCEPTANCE.md), [task ledger](tasks.json), and the selected task brief. This campaign is implementation-ready planning, not implementation proof. All required tasks start pending. Historical plans and issue-attached patches are evidence, not permission to replay old migrations or overwrite current code.

Read repository authorities (`CONTEXT.md`, `DECISIONS.md`, `CODE_GUIDE.md`) and `skill://incremental-delivery`. For Herdr-backed UI, also read `skill://cockpit-ui-parity` and both UI research authorities. Latest decisions override historical prose: current ANSI protocol-22 transport is not the old graphics experiment; Context/Review replace real extension panes; the browser is inline; legacy browser migration remains excluded. Do not restore old docks, extension runtime, exact patch-version gates, or old native scale defaults.

## Scope and finish line

Required: the 25 `required: true` tasks in `tasks.json`, including real macOS proof. Deferred: GitHub PR and Jira source expansion, explicitly tracked by `LATER-GHPR` and `LATER-JIRA`; neither is silently complete. GitHub #6 must remain open/partially satisfied after GitLab delivery unless its remaining provider scope is separately completed. No new credential store, settings product, remote-access platform, terminal graphics revival, or generic provider framework.

The campaign is complete only when every required task is `done`, each acceptance criterion has passing evidence on its required surface, all owned changes have commits, and ACCEPT-01 reconciles issues and resources. Missing macOS access, a needed MR fixture, or a failed verification leaves the affected task blocked, not done. Do all independently reachable work meanwhile. Scope reductions require the user's explicit decision, recorded in the observation ledger and task notes.

## Ledger and task states

`tasks.json` is the **only authoritative task-status ledger**. Briefs and the README do not duplicate mutable status. One orchestrator writes the ledger; workers report results but never mark themselves done.

- `pending`: not claimed. Ready when all `depends_on` entries are done and no external prerequisite blocks it.
- `in_progress`: one named worker/integration owner holds its declared locks. Set `owner` and `started_at` (UTC ISO-8601).
- `verifying`: edits integrated; integration owner is running review/static/runtime gates. Still holds locks.
- `blocked`: record precise `blockers`, attempted discovery, and the next unblock action. Explicitly retain locks in notes if partial edits exist; do not assume blocked work released shared files.
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

## Worker contract

Use the configured bounded implementation profiles (for example Luna for a file-owned UI slice, Terra for cross-module lifecycle/storage). Read-only exploratory research uses `scout`; risky integrated changes get a read-only reviewer. Keep advisory depth available as required by repository rules. A worker brief must include:

1. Exact task ID, outcome, issue links, source baseline and evidence distinctions.
2. Exclusive writable paths/symbols and non-goals; shared API/data identities fixed before siblings start.
3. Relevant task brief, authority files, prior dependency evidence, and acceptance criteria.
4. **Skip formatters, linters, builds, tests, services, and commits during a concurrent writing wave.** The integration owner runs each gate once after integration. No worker mutates user sessions, installs live binaries, or changes remote issues/MRs unless explicitly assigned a scoped fixture action.
5. Return changed paths, implemented behavior, unresolved risks, proposed focused checks, and consequential advisor recommendations. A successful worker exit is not acceptance.

Keep interpreting user intent at the orchestrator. Do not outsource top-level decomposition or let workers negotiate incompatible shared contracts. Never test while concurrent writers are changing the same integration surface.

## Verification and commit discipline

For every bug, capture a safe reproduction/negative control before repair, then show it no longer triggers. User-reported failures are ground truth: reproduction work establishes measurable acceptance, not whether to believe the report. Source-review candidates must be exercised before claiming a user-visible defect. Preserve passing behavior rather than rewrite a feature merely because the task exists.

After integration, run narrow relevant static checks, retained behavior regressions, and the real user action through the authoritative response/event to the rendered success/failure state. Browser proof is required for shared UI; real native proof is required for native commands/channels/startup/platform input/image decode. macOS-only behavior requires macOS, not Linux cross-compilation. Compare Herdr semantics in a uniquely named disposable TUI session. Run `skill://playwright-cli` when using that browser workflow, or the harness browser API for direct browser verification.

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
