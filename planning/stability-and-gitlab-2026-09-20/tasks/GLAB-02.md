# GLAB-02 — Import standalone GitLab merge requests

## Outcome

Extend the verified GitLab issue/provider identity to standalone Merge Requests (MRs) through the same read-only SourceProvider/cache/protocol flow. An MR URL is independently importable; it must not require a linked issue or Jira item. Metadata includes the exact source branch and review/discussion information that is available, while unsupported diff anchors remain explicitly unsupported. Freshness must detect title/body/comment/discussion changes even when the MR SHA is unchanged.

This brief follows GitHub issue [#6](https://github.com/nnexai/cockpit/issues/6) and depends on GLAB-01. Read [../ORCHESTRATOR.md](../ORCHESTRATOR.md) and [../tasks.json](../tasks.json); baseline is `6f6222b74e4f552ce697e61364cf653f4b6be29f`. The authorized project is [nnex.ai/integration](https://gitlab.com/nnex.ai/integration), but at inventory it had no MR and no recorded permission to push a branch/create one. Discover an authorized existing MR first; otherwise request a disposable fixture before any remote write.

## Evidence and starting points

- `crates/cockpit-providers/src/tea.rs` shows existing review metadata/review-comment normalization and explicit unsupported review data.
- `crates/cockpit-providers/src/lib.rs` and the GLAB-01 adapter define provider dispatch and CLI ownership; preserve the GET-only boundary.
- `crates/cockpit-core/src/repositories.rs` must resolve `/-/merge_requests/IID` with nested namespace and self-managed authority.
- `crates/cockpit-core/src/projects/defaults.rs` uses review `source_branch` for setup defaults; verify it does not infer an issue dependency.
- `crates/cockpit-core/src/sources.rs` defines SourceId, revision/hash freshness, bounded assets, refresh, and conflict preservation.
- `crates/cockpit-protocol/src/sources.rs` already provides Review and neutral source import/refresh contracts.
- `src/app/projects/SetupDialog.tsx` and `src/app/context/SourceImport.tsx` are the real setup/import surfaces.

## Changes

1. Resolve standalone URLs shaped as `https://host/group/subgroup/project/-/merge_requests/IID`, retaining exact host, configured base path, full project path, and numeric IID. Reject query/fragment/userinfo and ambiguous encoded path forms as GLAB-01 does.
2. Verify local checkout origin and provider response against the same full project identity. An MR URL may be imported as a source independently, but setup still requires the verified local repository when setup creates a workspace.
3. Normalize MRs to resource type `review` and canonical ID `<full-project-path>!<iid>`, distinct from issue IDs. Include normalized provider instance/base path in SourceId identity.
4. Fetch bounded title, description/body, state, author, labels, assignees/reviewers, timestamps, web URL, source/target project identity, target branch, source branch, head SHA, and available approvals/review/discussion metadata. Preserve discussion/note ID, author, created/updated times, URL, body, and file/line position as metadata text; do not claim local review anchors if the protocol cannot represent them.
5. Use the MR source branch exactly for setup defaults. Do not silently substitute target branch, a linked issue branch, local HEAD, or a generated name when source branch is present. Report a clear unsupported/invalid state when the source branch is unavailable.
6. Use explicit GET-only `glab api` requests and the shared bounded pagination/byte budget. Never perform remote clone, checkout, merge, push, branch creation, approval, comment, or other MR writes. Do not invoke a human-output command whose format could hide mutation or truncation.
7. Design freshness as a composite provider revision. SHA may be one component, but a changed title/body, updated timestamp, discussion/note set, or comment content must invalidate or refresh the source even if SHA is identical. Record enough bounded metadata to explain why a refresh changed.
8. Preserve user-edited generated source files on refresh through SourceService conflict behavior. A provider update must not overwrite local edits or report clean success while dropping a conflict.
9. Treat absent review capability, unavailable discussions, permission-limited notes, and truncated pages explicitly. Unsupported discussion anchors must remain visible as metadata or diagnostic, never as fabricated line comments.
10. Add the required review type: provider/source-identity review for revision construction and review metadata, plus a setup review proving standalone-MR behavior is not coupled to issue/Jira resolution. Do not perform a provider registry migration unless GLAB-01 evidence demonstrates it is required.

11. Keep source and target branch names as provider metadata with explicit escaping/length bounds; branch text is data and must not become a shell argument for checkout or a filesystem path without separate validation.
12. Verify the returned `web_url`, reference, project full path, and IID all agree with the requested host/base path before accepting the MR.
13. Preserve fork/source-project provenance. A valid MR remains importable if its source ref is missing locally or its source branch was deleted; setup must separately diagnose unavailable ref/permission without substituting a target branch or silently cloning/fetching a foreign project.
14. Build a composite freshness token from bounded MR revision, updated metadata, body/title hash, and bounded discussion/comment identity/content. If any component is unavailable, mark freshness conservative rather than pretending SHA is complete.
15. Preserve prior metadata long enough for the refresh result to explain which component changed, without retaining unbounded historical copies.
16. Normalize discussion ordering and duplicate IDs deterministically. A page-budget stop must be visible in the source entry and must not look like an empty discussion list.
17. Keep review comments and discussions separate from local saved annotations. A provider refresh cannot delete Cockpit comments or convert remote notes into local anchors.
18. Ensure a no-issue standalone MR can be selected in setup and Context Resources, while issue-only UI paths continue to reject incompatible resource kinds honestly.
19. Use fixture responses that include title-only, body-only, discussion-only, and timestamp-only changes with a stable SHA. Each must be observable as a freshness change or explicit uncertainty.
20. Keep all MR API calls GET-only and centralize the verb policy so future provider changes cannot accidentally inherit a mutation default.

## Non-goals

- No linked issue lookup requirement, Jira integration, issue creation, issue comments, MR approval, merge, checkout, clone, push, branch creation, or remote mutation.
- No diff renderer, local review-anchor model, full patch download, pipeline control, wiki support, or new source/cache model.
- No SHA-only freshness shortcut that misses title/body/discussion changes.
- No false success when a real MR fixture, discussion permission, or required provider capability is unavailable.
- GitHub PR and Jira work remain separate deferred scope; preserve gh/tea behavior.

## Acceptance

1. An authorized standalone GitLab MR URL imports without an issue or Jira URL and yields canonical `<path>!<iid>` identity.
2. Setup defaults use the exact provider-reported source branch; a missing/invalid source branch blocks or diagnoses setup rather than substituting another branch.
3. Provider output verifies project path, IID, URL/reference, and host/base-path authority before cache/materialization.
4. Metadata includes bounded MR fields and review/discussion records with IDs, authors, timestamps, URLs, bodies, and any available position text; unsupported anchors are explicit.
5. A refresh detects title/body/comment/discussion changes when SHA is unchanged, while unchanged content remains fresh within the documented bounded policy.
6. User edits survive refresh and produce a visible conflict/diagnostic instead of overwrite or false clean completion.
7. Captured CLI/API evidence proves GET-only behavior and no remote checkout/merge/write operation.
8. If no authorized MR fixture exists, real MR acceptance is recorded blocked with the exact permission/fixture prerequisite; issue work and deterministic provider negative cases remain verifiable.

## Verification

Start with local fixtures covering same-SHA metadata/discussion changes, pagination truncation, missing notes, malformed response, wrong project/IID, and permission denial. Exercise setup with an MR URL and no issue. Then, only with an authorized MR fixture, run installed glab read-only against the real project in a disposable session, import/list/refresh it, edit generated content, refresh again, and inspect conflict preservation. Never create a branch/MR without separately recorded permission, never merge, and never use a user's active session. Record fixture ownership and the absence of a fixture as a blocker rather than claiming completion.

The durable record must separate issue-independent MR proof from any linked-resource observation and include the requested/returned source branch, composite freshness components, discussion bounds, and conflict outcome.

Real MR acceptance is conditional on an authorized fixture. A fixture absence is useful evidence for the blocker but cannot be substituted with an issue, a local mock, or a fabricated MR URL.

## Handoff

Return changed paths, standalone-MR identity/branch/freshness rules, review metadata limits, negative evidence, and real-fixture status. Include durable run evidence and the implementation commit required by [../ORCHESTRATOR.md](../ORCHESTRATOR.md). Explicitly identify any blocked MR proof; a green issue-only run cannot close this brief.
