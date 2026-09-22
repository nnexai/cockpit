# Cockpit stability and GitLab campaign

**Start the entire campaign with this one command in OMP, opened at the Cockpit repository root:**

```text
/goal Execute planning/stability-and-gitlab-2026-09-20/AUTORUN.md
```

No custom command installation, plugin reload, per-task restart, or manual agent-message relay is needed. [AUTORUN.md](AUTORUN.md) is the autonomous execution contract; [ORCHESTRATOR.md](ORCHESTRATOR.md) defines scheduling/safety; [tasks.json](tasks.json) remains the sole status ledger. Each dependency-ready task is strongly planned just in time, then assigned to the cheapest configured worker that can safely follow that bounded plan; integration verification remains independent. This package prepares execution; it does not claim product work has started.

Planning checks and review findings/resolutions are recorded in [PLAN_VALIDATION.md](PLAN_VALIDATION.md).

## Package layout

```text
stability-and-gitlab-2026-09-20/
  README.md             scope, task index, launch prompt
  ORCHESTRATOR.md       scheduling, ownership, safety, completion rules
  AUTORUN.md            one-trigger goal and bounded increment execution
  campaign.py           read-only readiness and completion bookkeeping gate
  tasks.json            single mutable task-status ledger
  INVENTORY.md          dated GitHub/source/GitLab evidence
  ACCEPTANCE.md         complete behavior/platform/issue coverage
  OBSERVATIONS.md       queued live feedback and scope decisions
  PLAN_VALIDATION.md    planning checks and review resolution
  tasks/                one runnable brief per task
  templates/evidence.md durable acceptance/handoff record
  runs/                 created at execution time for actual evidence
```

There are **24 required tasks and 3 explicitly deferred tasks**: GitHub PR expansion, Jira expansion, and macOS native daily-use acceptance. None is pre-completed from historical evidence. To mark individual work done, the orchestrator updates its `tasks.json` record with `status: "done"`, actual commit hashes, evidence paths and completion time only after the task's criteria pass. Briefs remain stable requirements, not a second status board.

## Scope

- Fix/reconcile every current open GitHub issue except the explicitly deferred GitHub PR/Jira expansion within #6.
- Implement GitLab issues (including verified issue-type work-item URLs) and independently addressable merge requests through `glab` and the existing source pipeline.
- Stabilize existing terminal, setup/teardown, Context, Review/comments, source, inline-browser and native-install workflows.
- Prove genuine-error behavior, scroll continuity, focus/ownership, durable unsent work and measured resource/performance bounds on the appropriate browser/native/platform surfaces.

No new provider framework, credential store, remote-access product, settings product, terminal graphics revival, legacy browser migration or cosmetic redesign. GitHub PR/Jira work and macOS native acceptance stay visible as deferred; #6 is not fully closed by GitLab alone. Linux-native annotation behavior remains in WEB-05.

## Task index

Priority 0 protects basic usability/data; priority 1 completes the selected functionality and proof; priority 2 closes the integrated campaign. Priority 3 is deferred. Dependencies are behavior prerequisites; write-lock conflicts are separately declared in the JSON ledger and orchestrator guide. The table below is a navigation index, not a live progress view.

| Task | Outcome | Priority | Depends on |
| --- | --- | --- | --- |
| [RUN-01](tasks/RUN-01.md) | Establish safe runtime and evidence baseline | 0 | — |
| [TERM-01](tasks/TERM-01.md) | Accept compatible Herdr patch releases | 0 | RUN-01 |
| [TERM-02](tasks/TERM-02.md) | Attach terminals only for visible panes | 0 | TERM-01 |
| [SYNC-01](tasks/SYNC-01.md) | Cancel obsolete work without false errors | 1 | TERM-02 |
| [TERM-03](tasks/TERM-03.md) | Stabilize terminal scrolling focus and input | 1 | SYNC-01 |
| [VIEW-01](tasks/VIEW-01.md) | Preserve Context and Review navigation continuity | 1 | TERM-01 |
| [FLOW-01](tasks/FLOW-01.md) | Verify durable local development workflows | 1 | VIEW-01, SETUP-02, SYNC-01 |
| [SETUP-01](tasks/SETUP-01.md) | Separate source inspection from explicit setup | 0 | TERM-01 |
| [SETUP-02](tasks/SETUP-02.md) | Recover companion setup safely across platforms | 0 | SETUP-01 |
| [GLAB-01](tasks/GLAB-01.md) | Import GitLab issues through existing source flow | 1 | SETUP-01 |
| [GLAB-02](tasks/GLAB-02.md) | Import standalone GitLab merge requests | 1 | GLAB-01 |
| [GLAB-03](tasks/GLAB-03.md) | Stabilize provider refresh conflicts and resource UI | 1 | GLAB-02 |
| [GLAB-04](tasks/GLAB-04.md) | Prove authenticated GitLab end to end | 1 | GLAB-03, SETUP-02 |
| [WEB-01](tasks/WEB-01.md) | Preserve browser page drafts and saved feedback | 0 | TERM-01 |
| [WEB-02](tasks/WEB-02.md) | Deliver first browser click and element pick | 0 | TERM-01 |
| [WEB-03](tasks/WEB-03.md) | Keep resized browser frames sharp and aligned | 1 | TERM-01 |
| [WEB-04](tasks/WEB-04.md) | Complete browser input navigation and ownership | 1 | WEB-02, WEB-03, SYNC-01 |
| [WEB-05](tasks/WEB-05.md) | Complete annotation capture and delivery reliability | 1 | WEB-01, WEB-02, WEB-03 |
| [WEB-06](tasks/WEB-06.md) | Bound browser frame input and lifecycle resources | 1 | WEB-01, WEB-04 |
| [WEB-07](tasks/WEB-07.md) | Make browser dependency failures actionable | 1 | RUN-01 |
| [WEB-08](tasks/WEB-08.md) | Verify inline browser security boundaries | 1 | WEB-04, WEB-05, WEB-06, WEB-07 |
| [NATIVE-01](tasks/NATIVE-01.md) | Deliver owned bundle installer and verify Linux-prefix install/update behavior; macOS execution is outside current scope | 1 | RUN-01 |
| [NATIVE-02](tasks/NATIVE-02.md) | Verify macOS native daily use and recovery (deferred) | 1 | NATIVE-01, SETUP-02, TERM-03, FLOW-01, WEB-05, WEB-06, WEB-07, GLAB-04 |
| [PERF-01](tasks/PERF-01.md) | Prove sustained whole application responsiveness | 1 | TERM-03, FLOW-01, GLAB-04, WEB-06, WEB-07, WEB-08 |
| [ACCEPT-01](tasks/ACCEPT-01.md) | Close campaign with evidence and issue reconciliation | 2 | PERF-01 |
| [LATER-GHPR](tasks/LATER-GHPR.md) | Add GitHub pull request source support | 3 | — |
| [LATER-JIRA](tasks/LATER-JIRA.md) | Add Jira work item source support | 3 | — |

## Operational starting point

1. Read the inventory and authority precedence; inspect the current worktree without absorbing pre-existing changes.
2. Select one dependency-ready task (including a no-code verification task), inspect its current source/authority/dependency evidence, and record a strong just-in-time plan in the existing per-task run record before dispatch. Do not speculatively plan every task.
3. Dispatch the cheapest configured worker capable of the accepted bounded slice, with explicit ownership, recipe, proof and stop/escalation conditions. Parallelize only genuinely disjoint slices; do not start product work before the plan gate passes.
4. Integrate, independently review and verify the actual surface once edits settle. Commit each accepted increment and record its evidence/SHA.
5. Resume from the ledger and durable run records; continue reachable tasks when one platform/fixture is blocked. Finish with PERF-01 and ACCEPT-01, not merely a green unit suite. NATIVE-02 remains deferred by the user's 2026-09-22 scope decision.

The plan is a dispatch contract, not a second mutable ledger or a fake completion receipt. It records observed baseline/findings separately from assumptions; criterion-to-increment coverage; exact paths, symbols, callsites and existing patterns; chosen APIs/invariants; ordered bounded steps and disjoint ownership; edge/error/lifecycle checks, platforms, resources and authorization; worker capability and stop conditions; and orchestrator acceptance/revalidation rules. A no-code task gets a scenario-and-evidence plan, never invented code changes.

GitLab validation target: [nnex.ai/integration](https://gitlab.com/nnex.ai/integration). The authorized existing [fixture issue #1](https://gitlab.com/nnex.ai/integration/-/work_items/1) is open for this campaign. No MR exists at inventory; permission for fixture branch/MR writes must be established separately. Production adapters remain read-only.

## Autonomous execution

The single campaign goal stays active across small, independently verified increments. The orchestrator selects work, delegates disjoint slices, integrates, verifies, commits, updates the ledger, and immediately continues. It must not call the goal complete at a task boundary or ask the user to relay the next assignment.
**Model roles:** Sol or Astra plans and accepts each task; cheaper capable workers implement the accepted recipe. Use only enabled bundled agents plus the retained Astra advisor, not disabled project-specific profiles. Luna is not a planner. Model routing is not changed by this package.


Broad task briefs are decomposed just before execution; each dependency-ready task gets a strong planning round, including no-code scenario verification. The orchestrator owns top-level decomposition and plan acceptance; it may commission bounded read-only slice design only when genuine independent uncertainty warrants it. The accepted plan must inspect current authorities and dependency evidence, resolve consequential unknowns, and leave no unresolved design decision for the assigned slice before dispatching a cheaper capable worker. It is recorded in the existing `runs/<run-id>/<TASK-ID>.md` record, not a new plan file or mutable board.

The plan must give the worker a decision-complete recipe: baseline findings versus assumptions; full criterion-to-increment coverage; exact paths, symbols, callsites and current patterns; APIs/invariants; ordered steps and disjoint ownership; edge/error/lifecycle proof, platforms, resources and authorization; capability and stop/escalation boundaries. The worker implements only that slice. An independent integration owner then revalidates source/contracts and exercises the real surface; drift, resume changes, ambiguity or repeated failure sends the affected slice back through strong planning while independent work continues. An accepted plan is not behavior proof, a task receipt or permission to weaken original acceptance.

Multiple increment commits may satisfy one task, but partial work never marks that task done. The one `/goal` trigger remains the only user handoff; execution does not begin product work until its per-task plan gate passes.

The orchestrator runs `python3 planning/stability-and-gitlab-2026-09-20/campaign.py ready` itself. Its output is a candidate list, not a concurrency-safe batch or proof of runtime success. `check` validates ledger/evidence integrity; `complete` additionally requires all required tasks done. The helper never starts work or changes the ledger.

Unavailable external access/authorization blocks only affected work. Finish other reachable tasks first, then ask once for the precise remaining prerequisites; do not invent permission, loop on the same missing resource, or claim campaign completion. macOS-specific execution and acceptance are outside the current campaign scope; historical criteria remain in the handoff for reference. GitLab MR proof requirements remain mandatory.

See [AUTORUN.md](AUTORUN.md) for interruption recovery and bounded context checkpoints. Execution needs a running OMP session; this is not a daemon that survives closing OMP.

## Plan verification versus product verification

This planning delivery checks task-file/ledger agreement, dependency acyclicity and final-gate coverage, local links, required brief sections, issue/browser-matrix coverage, and scheduling/permission consistency. It does **not** rerun application tests or claim product fixes. Actual task evidence will be written under `runs/` during execution; none is fabricated here.
