# GLAB-04 — Prove authenticated GitLab end to end

## Outcome

Prove the delivered read-only GitLab issue and standalone MR paths on the real browser and native surfaces: authenticated setup, import, refresh, conflict/availability handling, and recovery. Use only explicitly authorized disposable fixtures and run-owned Herdr sessions/configs. Authentication remains owned by the installed `glab` CLI/keyring; Cockpit must not receive, persist, print, or export secrets. A missing authorized MR fixture blocks MR acceptance and must never be represented as a false pass.

This brief follows GitHub issue [#6](https://github.com/nnexai/cockpit/issues/6), depends on GLAB-03 and SETUP-02, and follows [../ORCHESTRATOR.md](../ORCHESTRATOR.md) and [../tasks.json](../tasks.json). Authorized issue fixture: project [nnex.ai/integration](https://gitlab.com/nnex.ai/integration), ID `86672117`, issue marker `cockpit-glab-2026-09-20-a17b`, canonical work-item URL [/-/work_items/1](https://gitlab.com/nnex.ai/integration/-/work_items/1). At inventory the project had protected `main` only, no MRs, and no recorded branch/MR write permission.

## Evidence and starting points

- `planning/inline-space-browser-2026-09-13/03-delivery-and-verification.md` is the existing A01–A25 browser matrix; A23 is excluded, and this task must not silently omit the other applicable rows.
- `crates/cockpit-host/src/bin/cockpit.rs` composes the real provider, project, source, and browser services.
- `crates/cockpit-host/src/server/projects.rs` and `sources.rs` expose setup/import/list/refresh/recovery routes.
- `crates/cockpit-core/src/projects.rs` owns plan/start/partial/recovery and source materialization sequencing.
- `crates/cockpit-core/src/sources.rs` and `context_assets.rs` own cache, freshness, conflicts, bounded resources, and publication.
- `src/app/projects/SetupDialog.tsx`, `src/app/context/SourceImport.tsx`, and `src/app/context/ContextResources.tsx` are the browser acceptance surface.
- Native command/channel adapters and `src/client/projectProtocol.ts` / `sourceProtocol.ts` must remain contract-equivalent.
- `~/.config/glab-cli` and keyring metadata are auth authorities; no secret belongs in a Cockpit config, fixture, evidence capture, argv, or environment dump.

## Changes

1. Prepare a fixture ledger before execution: project ID/URL, issue URL/type, MR URL/IID if authorized, branch/worktree ownership, comments/discussion mutation permission, session/config/profile identifiers, and cleanup owner. Never use the protected active Herdr session or a user's browser profile.
2. Verify installed `glab` version/authentication with secret-safe output and target only the authorized host/project. Use GET-only provider operations. Do not enumerate unrelated projects or infer an MR from issue state.
3. Run the real browser setup flow with the fixture issue: enter the work-item URL, observe actual issue-type validation and canonical provider/source identity, inspect exact preview/plan/effects, start explicitly, and verify workspace, companion, terminal, and source materialization receipts.
4. Exercise source import/list/refresh from Context Resources, including bounded comment display, retained-list behavior on a controlled failure, and refresh after a controlled fixture comment/body change. Preserve ownership and restore the fixture after evidence where permitted.
5. Exercise setup/source failure and recovery using a disposable run-owned checkout/session. Verify no partial or failed source operation fabricates success, companion publication is no-replace, and retry/recovery cleans or completes only owned resources.
6. Run the native setup/import/list/refresh/recovery commands against the same protocol contract in a separate uniquely named disposable session. Record rendered/terminal output and authoritative response events rather than relying on process exit alone.
7. If an authorized standalone MR exists, repeat setup/import/refresh with no issue or Jira input; verify exact source branch, review/discussion metadata, same-SHA content freshness, and local conflict preservation. Do not checkout, merge, push, create, approve, or mutate the MR.
8. If no MR exists or permission is not recorded, stop the MR mutation path, record the precise prerequisite and blocker, and still complete all reachable issue, negative, browser/native, and recovery evidence. Do not mark standalone-MR acceptance complete.
9. Include a security/fixture-ownership review of auth handling and a runtime review of browser/native parity, receipts, cleanup, and no-active-session isolation. Any unexpected remote mutation is a release blocker.
10. Keep GitHub PR and Jira source expansion explicitly deferred. Passing GitLab issue/MR evidence does not close those capabilities or issue #6 beyond the proven GitLab scope.

11. Capture the exact host/base-path and full project identity used by each run, without logging token values or broad project listings.
12. Before changing a fixture issue, record its current title/body/comment count/type and verify the mutation is permitted by the fixture owner. Restore only owned changes after all consumers finish.
13. For controlled provider failures, use a fake executable/API fixture or reversible permission boundary; do not revoke access on a shared user account or damage the authorized project.
14. Assert that source preview/defaults and plan are read-only by inspecting Herdr inventory, checkout, companion, and terminal state before explicit Start.
15. Assert operation receipts against the authoritative server/native event and rendered UI state, including unknown/unsupported operation diagnostics where the campaign implementation exposes them.
16. Run import and refresh after browser reload/reconnect to verify the last confirmed list and durable cache survive a transient client disconnect.
17. Inspect generated source files for canonical identity, bounded metadata, revision/freshness, and conflict markers without treating screenshots as the only proof.
18. Keep all real-run commands scoped to the authorized project and fixture IDs. A successful `glab` login check alone is not issue or MR acceptance.
19. Record cleanup for every process, socket, profile, checkout, companion, worktree, and fixture edit. Leave no retained service without an explicit reason.
20. Reconcile the final evidence with every applicable browser matrix row and native counterpart; mark unavailable rows blocked with the missing prerequisite rather than omitting them.

## Non-goals

- No creation of GitLab branches/MRs/issues/comments unless a separately recorded user permission and disposable fixture explicitly authorizes that exact write; the production adapter remains read-only.
- No project protection changes, main pushes, force pushes, merges, remote deletion, checkout through Cockpit, or credential migration.
- No claim that a URL is an issue merely because it uses `/-/work_items`; validate actual `issue_type`.
- No browser-only or native-only completion, no mock-only authenticated proof, and no false completion when MR fixture/permission is absent.
- No GitHub PR or Jira implementation; both remain deferred separate work.

## Acceptance

1. Real authenticated browser issue setup succeeds through preview, explicit start, source materialization, Context Resources import/list/refresh, and durable receipts using the authorized fixture.
2. The work-item URL is accepted only after actual API issue-type validation; wrong type, wrong project, auth failure, and permission failure are visible genuine errors.
3. Browser refresh demonstrates bounded metadata/comments, controlled changed-content detection, retained-list failure behavior, and local conflict preservation where applicable.
4. Native setup/import/list/refresh/recovery exercises the same source/project protocol and reaches authoritative success/failure states in a separate disposable session.
5. Recovery after a controlled partial failure proves no-replace companion publication and owned cleanup; unrelated sentinels/resources remain untouched.
6. An authorized standalone MR, if available, imports independently of issue/Jira, uses exact source branch, exposes bounded review/discussion metadata, and detects same-SHA content changes. If unavailable, the evidence records the blocker and leaves this criterion incomplete.
7. No captured argv/environment/output contains credentials, no production-adapter request writes remotely, and no active-user-session or unauthorized fixture mutation occurs. Separately authorized test-fixture writes are ledger-recorded with before/after and cleanup evidence; they do not grant write capability to Cockpit.
8. Fixture ledger, evidence artifacts, cleanup, and real implementation commit are durable and reviewable; issue #6 remains open for deferred GitHub PR/Jira scope.

## Verification

The integration owner should use `skill://playwright-cli` for browser automation and the repository's native verification helpers. Assign unique session/config/profile/port identities, run resource guards, and capture authoritative responses plus rendered states. Perform controlled issue fixture updates only when permitted, with before/after records; use a fake provider for failure injection rather than damaging the shared project. Discover an authorized existing MR with narrow GET metadata only. If none exists, record the exact missing permission/fixture and do not create a branch or MR. Clean every recorded resource and retain durable evidence without secrets.

The durable record must distinguish issue acceptance, MR acceptance, browser proof, native proof, recovery proof, and blocked prerequisites. A single authenticated command transcript cannot stand in for those surfaces.

Before handoff, reconcile every created resource against the fixture ledger and include cleanup outcome, retained-resource reason if any, and the implementation/evidence commit.

## Handoff

Return the fixture ledger, browser/native run IDs, issue and MR URLs/types, receipts, failure/recovery observations, cleanup records, and any blockers. Include separate evidence for issue and MR criteria and explicitly state which GitHub PR/Jira work remains deferred. Completion requires a real commit for implementation plus durable evidence; an authenticated CLI probe, screenshot, or issue-only run cannot close the full brief.
