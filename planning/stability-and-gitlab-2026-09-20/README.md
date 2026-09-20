# Cockpit stability and GitLab campaign

**Start with [ORCHESTRATOR.md](ORCHESTRATOR.md), then [tasks.json](tasks.json).** This package is the execution plan for the user's GitLab integration, bugfix and daily-use polish campaign. It is not a claim that the work has been implemented.

Planning checks and review findings/resolutions are recorded in [PLAN_VALIDATION.md](PLAN_VALIDATION.md).

## Package layout

```text
stability-and-gitlab-2026-09-20/
  README.md             scope, task index, launch prompt
  ORCHESTRATOR.md       scheduling, ownership, safety, completion rules
  tasks.json            single mutable task-status ledger
  INVENTORY.md          dated GitHub/source/GitLab evidence
  ACCEPTANCE.md         complete behavior/platform/issue coverage
  OBSERVATIONS.md       queued live feedback and scope decisions
  PLAN_VALIDATION.md    planning checks and review resolution
  tasks/                one runnable brief per task
  templates/evidence.md durable acceptance/handoff record
  runs/                 created at execution time for actual evidence
```

There are **25 required tasks and 2 explicitly deferred provider tasks**. All required tasks start `pending`; none is pre-completed from historical evidence. To mark individual work done, the orchestrator updates its `tasks.json` record with `status: "done"`, actual commit hashes, evidence paths and completion time only after the task's criteria pass. Briefs remain stable requirements, not a second status board.

## Scope

- Fix/reconcile every current open GitHub issue except the explicitly deferred GitHub PR/Jira expansion within #6.
- Implement GitLab issues (including verified issue-type work-item URLs) and independently addressable merge requests through `glab` and the existing source pipeline.
- Stabilize existing terminal, setup/teardown, Context, Review/comments, source, inline-browser and native-install workflows.
- Prove genuine-error behavior, scroll continuity, focus/ownership, durable unsent work and measured resource/performance bounds on the appropriate browser/native/platform surfaces.

No new provider framework, credential store, remote-access product, settings product, terminal graphics revival, legacy browser migration or cosmetic redesign. GitHub PR/Jira work stays visible as deferred; #6 is not fully closed by GitLab alone. A missing macOS runner or MR fixture is a blocker for its acceptance, not permission to reduce scope.

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
| [NATIVE-01](tasks/NATIVE-01.md) | Install and update the macOS application bundle | 1 | RUN-01 |
| [NATIVE-02](tasks/NATIVE-02.md) | Verify macOS native daily use and recovery | 1 | NATIVE-01, SETUP-02, TERM-03, FLOW-01, WEB-05, WEB-06, WEB-07, GLAB-04 |
| [PERF-01](tasks/PERF-01.md) | Prove sustained whole application responsiveness | 1 | TERM-03, FLOW-01, GLAB-04, WEB-06, WEB-07, WEB-08 |
| [ACCEPT-01](tasks/ACCEPT-01.md) | Close campaign with evidence and issue reconciliation | 2 | NATIVE-02, PERF-01 |
| [LATER-GHPR](tasks/LATER-GHPR.md) | Add GitHub pull request source support | 3 | — |
| [LATER-JIRA](tasks/LATER-JIRA.md) | Add Jira work item source support | 3 | — |

## Operational starting point

1. Read the inventory and authority precedence; inspect the current worktree without absorbing pre-existing changes.
2. Claim RUN-01 in the central ledger and establish safe runtime/evidence identities. Do not start by rerunning historical migration plans or creating another fixture issue unnecessarily.
3. Choose dependency-ready tasks with disjoint write ownership. Parallelize real independent slices, not multiple writers of BrowserPane/App/helper.
4. Integrate, review, validate and exercise the actual surface once edits settle. Commit each accepted increment and record its evidence/SHA.
5. Resume from the ledger and durable run records; continue reachable tasks when one platform/fixture is blocked. Finish with PERF-01/NATIVE-02 and ACCEPT-01, not merely a green unit suite.

GitLab validation target: [nnex.ai/integration](https://gitlab.com/nnex.ai/integration). The authorized existing [fixture issue #1](https://gitlab.com/nnex.ai/integration/-/work_items/1) is open for this campaign. No MR exists at inventory; permission for fixture branch/MR writes must be established separately. Production adapters remain read-only.

## Orchestrator launch prompt

```text
Execute planning/stability-and-gitlab-2026-09-20/README.md as the active
stability/GitLab campaign. Read ORCHESTRATOR.md, INVENTORY.md,
ACCEPTANCE.md, OBSERVATIONS.md and tasks.json first, then repository
skills/authorities and each selected task brief.

Use tasks.json as the sole status ledger. Start with RUN-01; select only
dependency-ready work, assign exclusive files/symbols and honor locks.
Delegate independent bounded slices to configured workers, retaining
integration, user intent and verification ownership. Workers skip shared
validation and commits while edits run. No user-session or live-install
mutations. Use the designated GitLab fixture and explicit permissions.

Deliver all required tasks in verified increments with real browser/native
and macOS proof where required, evidence and commit hashes. Record and
work around external blockers without silently dropping acceptance.
Keep GitHub PR/Jira expansion deferred and #6's remaining scope explicit.
Do not mark work done from an agent message, historical tests, or a build.
```

## Plan verification versus product verification

This planning delivery checks task-file/ledger agreement, dependency acyclicity and final-gate coverage, local links, required brief sections, issue/browser-matrix coverage, and scheduling/permission consistency. It does **not** rerun application tests or claim product fixes. Actual task evidence will be written under `runs/` during execution; none is fabricated here.
