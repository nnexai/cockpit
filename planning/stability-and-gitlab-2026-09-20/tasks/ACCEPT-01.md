# ACCEPT-01 — Close campaign with evidence and issue reconciliation

Status, dependencies, owner, locks and completion proof: [task ledger](../tasks.json). Follow the [orchestrator contract](../ORCHESTRATOR.md).

## Outcome

The required campaign is demonstrably complete, with issue-level evidence, no lost acceptance items, and a resumable record of what was delivered versus explicitly deferred.

## Evidence and starting points

Read every required task's committed evidence and [ACCEPTANCE](../ACCEPTANCE.md). `tasks.json` is authoritative. Historical completion headings, attached patches, old test totals and agent outputs are not substitutes. #6 contains GitHub PR/Jira requirements beyond this GitLab campaign.

## Changes

1. Confirm every other required task is done with real commit/evidence references; verify current code still includes those changes and later edits did not invalidate acceptance. Audit open observations and blockers.
2. Run one integrated user journey: create an explicitly reviewed GitLab task Space, inspect imported context, use Files/Review, author and paste comments without Enter, use browser/annotations, switch/scroll/reconnect, refresh changed source, recover a real failure, and safely tear down owned resources. Include borrowed-directory refusal and preserve a sentinel unrelated resource.
3. Run the final affected shared build/type/protocol/test gates once on the settled tree, plus actual browser/Linux-native and recorded macOS evidence. Repair regressions through owned tasks; do not narrow the gate.
4. Map #3/#5/#7/#8/#9/#10/#11/#12/#13 to their exact accepted outcomes and commits. Mark #6 GitLab scope satisfied only after issue+MR proof; keep its GitHub PR/Jira scope outstanding. Carry closed #1/#2/#4 as regression proof.
5. Reconcile run-owned resources and the GitLab fixture issue after all consumers finish. Close only the marked test issue; retain compact fixture/evidence metadata. MR cleanup must follow separately recorded authorization.
6. Prepare concise GitHub closure/partial-completion notes; publish comments/closures only with established permission. Local campaign completion does not depend on permission to edit the tracker, but the truthful disposition must be recorded.

## Non-goals

No blanket closing of all GitHub issues, no undeclared feature expansion, no automatic production install/push/release, and no relabeling blocked macOS or MR evidence as deferred to get a green board.

## Acceptance

1. Every required task except this final gate has passing evidence and commits; all required cross-surface cases in the coverage map are satisfied.
2. The integrated real journey passes without spurious failures, lost drafts, incorrect source identity, unsafe deletion or duplicate mutations/paste.
3. Current shared validation gates pass and code generation is consistent; native acceptance is real on the required platforms.
4. Each observation has an accepted disposition; unresolved in-scope defects prevent completion.
5. Resource cleanup is ownership-checked, with explicit justified retained artifacts; the authorized issue fixture is closed only after its last use.
6. Issue dispositions accurately distinguish fixed, verified existing behavior, overlap, and the two deferred provider features.
7. User-facing handoff lists commits, exact behavior/platform proof and remaining out-of-scope work without claiming broader coverage.

## Verification

Check dependency coverage for all 24 other required tasks, evidence existence/actual observations, commit reachability and protocol/build provenance. Use the actual integrated surface for the final journey. A read-only integrated review should focus on data preservation, authority, cancellation, host isolation and negative cases. Do not rerun unrelated project-wide commands repeatedly while fixing one narrow regression.

## Handoff

Commit final acceptance/disposition evidence and checkpoint the ledger. Report the campaign commit range and per-task proof index to the user. Set this task done only after its own evidence commit exists; a later status checkpoint can record that SHA without self-referential commit rewriting.
