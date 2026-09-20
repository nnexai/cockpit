# Task evidence record template

Copy to `runs/<run-id>/<TASK-ID>.md` inside this package when executing a task. Replace these instructions with observed facts; an unfilled template is not evidence. Keep raw screenshots/traces outside Git unless small and intentionally owned, and record durable locations plus hashes. Never store credentials.

## Identity and ownership

Record task ID, run ID, UTC time, integration owner/reviewer, baseline and tested source commits, changed paths, OS/architecture, host/native/browser/Herdr/Node/CLI versions, protocol/schema and exact built artifact identity. Distinguish browser, Linux-native and macOS runs.

List the named Herdr session, endpoint, fixture repository/worktree/companion, config/state roots, process/profile/display/port identities and the resource-guard ledger. State which resources are protected and which this scenario created. For GitLab include only the authorized project/fixture IDs, marker, API method and mutation permissions—not tokens.


## Accepted just-in-time task plan

Write this section before dispatching implementation or running the planned verification scenario. It is the durable plan in this existing run record, not a second ledger, a new plan file, or a completion receipt. Record `ACCEPTED` only after the orchestrator has reviewed it and no design decision for the assigned slice remains unresolved.

- **Selection and baseline:** task/increment ID, dependency-ready evidence and why this slice is selected now; source/authority/dependency baseline and observed findings, clearly separated from assumptions or hypotheses.
- **Planner identity:** Sol/Astra planning owner and actual configured model; any bounded Astra advisory input and the orchestrator's decision. Record the implementation worker separately; worker effort or agent names are not proof of planning capability.
- **Coverage:** map every numbered task criterion to one or more bounded increments and name any outstanding criterion; state the observable outcome, required surface and platform for each increment.
- **Design and locations:** chosen design, APIs/interfaces, invariants and ownership rules; exact files, symbols, callsites and relevant existing patterns inspected. For a no-code task, describe only the scenario/state transitions and evidence path—do not invent code changes.
- **Recipe and ownership:** ordered bounded worker steps, exclusive writable paths/symbols, disjoint parallel ownership, shared contracts, non-goals and integration handoff. Include the expected authoritative result, not merely a command to run.
- **Proof, risks and resources:** positive, negative, edge, error, recovery and lifecycle checks; platform/browser/native requirements; resource identities, authorization and cleanup; known risks and the condition that makes each one a blocker or escalation.
- **Dispatch and acceptance:** cheapest capable worker and why it is sufficient; capability limits, stop/escalate conditions and independent integration/review owner; orchestrator acceptance owner/time. State how source/contracts and the plan will be revalidated after drift or resume and when the affected slice must return to strong planning.

An accepted plan authorizes only the named bounded slice. It does not prove behavior, satisfy an acceptance criterion, or permit weakening the original task scope.

## Increment contracts and receipts

Before each bounded increment, record:

- A local increment label (for example `WEB-02/01`) and one observable outcome.
- The original task acceptance criteria it covers and those still outstanding.
- Exclusive writable paths/symbols, shared interface contracts and non-goals.
- Exact positive and important negative scenarios: starting fixture/state, action, expected authoritative response and visible result, required platforms.
- The focused static/behavior gates and integration/review owner.

After verification, append actual results, evidence locations, changed paths and the resulting commit reference (record the SHA after that commit, not inside itself). Keep failed attempts and the reason for a changed approach concise. These are contracts and receipts, not a second task-status ledger. A partial increment does not satisfy the parent task; `tasks.json` stays `in_progress` until every original criterion has passed.

At a context checkpoint, state the next exact action, outstanding criteria, unresolved decision/blocker, current owned edits, and retained runtime/fixture identities. Resume from this compact state rather than replaying worker transcripts.

## Original behavior and decision

Describe the user's reported failure and safe measurable reproduction or negative control, including actual output/visible result. Source-review hypotheses are labeled as such until exercised. Describe the root cause established by evidence and why the repair preserves the current authority/ownership contract. For an already-correct feature, record the real passing scenario instead of inventing a code change.

## Acceptance results

Create one row for every numbered acceptance criterion in the task brief:

| Criterion | Surface/platform | Result (PASS / FAIL / BLOCKED) | Action, authoritative result and rendered observation | Evidence location/hash |
| --- | --- | --- | --- | --- |

Populate actual rows before requesting completion. Record initial and post-fix results separately where applicable. A successful HTTP request, mock, screenshot of startup or compiler result cannot stand in for the complete interaction. Explain any unavailable case; it remains blocked if required.

## Checks executed

Record exact commands/scenarios, cwd/environment identity where relevant, exit/results and pertinent output. Separate narrow tests/static gates from actual surface proof. Include negative/error/recovery cases, ordering/cancellation/unknown-outcome handling, and real native checks when required. Do not copy historical test totals.

## Performance evidence

When relevant, record fixed fixture/dimensions/build, warm-up duration, sample count and method, clock relationship, p50/p95, frame/request/queue/object counts, CPU/PSS units/range/slope and capture artifacts. State target and actual result. Never change thresholds after measurement without an explicit recorded decision.

## Review and advisory decisions

Record reviewer scope, concrete findings, resolutions and rechecks. If an advisor was consulted, record consequential advice and the integrating engineer's decision plus evidence; advisor agreement is not verification. State why review is unnecessary for a bounded no-risk documentation-only task if applicable.

## Cleanup and retained resources

List every created resource and removal receipt or named next consumer/final cleanup owner. Verify unrelated sentinel/user resources survive. Record fixture issue/comment/branch/MR changes with the applicable authorization; retain the GitLab issue while downstream validation needs it. Remove throwaway scripts after proof or retain only intentional behavior regressions. Keep compact evidence durable after runtime teardown.

## Delivery and remaining blockers

Record actual implementation/evidence commit hashes, task ledger path, any reopened downstream task, and exact remaining blocker/unblock action. The implementation SHA can be recorded in the ledger after committing this report; do not embed a commit's own hash in itself. No task is done while a required criterion is FAIL/BLOCKED or owned changes lack a commit.
