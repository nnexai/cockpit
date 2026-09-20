# GLAB-01 — Import GitLab issues through existing source flow

## Outcome

Add a read-only GitLab issue path to the existing SourceProvider/cache/protocol flow. Resolve an issue URL to one verified project identity, fetch bounded metadata and comments through the installed `glab` CLI using explicit GET API requests, and expose it through setup/import/list/refresh without a second source model. The path must handle nested namespaces, self-managed authority, and the distinction between GitLab work-item URLs and actual issue resources.

This brief follows GitHub issue [#6](https://github.com/nnexai/cockpit/issues/6). The authorized disposable project is [nnex.ai/integration](https://gitlab.com/nnex.ai/integration), project ID `86672117`; fixture issue `#1` is open at [/-/work_items/1](https://gitlab.com/nnex.ai/integration/-/work_items/1), with marker `cockpit-glab-2026-09-20-a17b`. The API response for `/projects/86672117/issues/1` has `issue_type: issue`; preserve the work-item URL but never assume every work item is an issue. Read [../ORCHESTRATOR.md](../ORCHESTRATOR.md) and [../tasks.json](../tasks.json); baseline is `6f6222b74e4f552ce697e61364cf653f4b6be29f`.

## Evidence and starting points

- `crates/cockpit-providers/src/lib.rs` dispatches providers by executable basename; add only the smallest glab seam needed and keep gh/tea behavior unchanged.
- `crates/cockpit-providers/src/github.rs` demonstrates bounded GET metadata/comments and stable provider errors.
- `crates/cockpit-providers/src/tea.rs` demonstrates configurable host/base-path handling and provider-neutral source assets.
- `crates/cockpit-core/src/repositories.rs` resolves artifact URLs and is the authority for host/path/IID validation.
- `crates/cockpit-core/src/sources.rs` owns SourceAuthority, source IDs, bounded fetch/cache/materialization, freshness, and conflict preservation.
- `crates/cockpit-core/src/projects/defaults.rs` matches a local verified origin and consumes provider metadata.
- `crates/cockpit-protocol/src/sources.rs` already has Issue/IssueComments and neutral import/refresh/list contracts.
- `crates/cockpit-protocol/src/projects.rs` and `crates/cockpit-core/src/config.rs` define provider configuration; credentials remain CLI-owned.

## Changes

1. Resolve only absolute issue URLs shaped as `https://host/group/subgroup/project/-/issues/IID`, plus an equivalent configured self-managed base-path form. Keep host, effective port, configured base path, full namespace/project path, and numeric IID as separate validated fields.
2. Preserve `/-/work_items/IID` as a URL form that requires API validation. Accept it as an issue only when the fetched resource explicitly reports `issue_type: issue` and matches project/IID; reject epics, tasks, incidents, or unknown work-item types as unsupported rather than coercing every work item to an issue.
3. Reject userinfo, query/fragment where unsafe, ambiguous encoded slash/traversal, empty namespace/project components, and host/base-path mismatches before invoking the provider or writing cache state.
4. Parse and verify the local primary checkout origin against the complete GitLab project path and configured host/base path. Nested groups must match exactly; same project path on another host or subfolder must not match. An issue URL alone must not bypass local repository selection for setup.
5. Normalize an issue to resource type `issue` and canonical ID `<full-project-path>#<iid>`. Include normalized provider instance/self-managed authority in SourceService source identity so hosts and base paths cannot collide.
6. Implement the provider as read-only `glab api` calls. Every request with fields/pagination must pass explicit `--method GET`; select the configured host narrowly (`--hostname` or equivalent) and disable prompts. Never place a token in argv, environment, diagnostics, or generated assets.
7. Fetch bounded title, body, author, state, labels, assignees, available milestone, timestamps, web URL, revision, and comments. Preserve comment ID, author, created/updated time, URL, and body. Stop at the shared page/byte budget and return an explicit truncation diagnostic instead of silently dropping data.
8. Verify returned project path, IID, and canonical/web URL against the requested authority before accepting metadata. Map malformed JSON, auth denial, permission denial, missing issue, rate limit, and unsupported work-item type to stable actionable errors.
9. Reuse SourceService cache/freshness/materialization and existing browser/native source routes. Do not add a GitLab-specific cache, transport, or credential store. Keep production adapter GET-only even though the authorized fixture permits issue creation by a separate verification owner.
10. Add a provider/security review for argv and environment redaction, HTTP verb enforcement, host selection, path identity, and bounded pagination. Do not make provider-kind/config registry migration part of this slice unless implementation evidence proves it is required for safe dispatch.

11. Keep the supplied URL as original provenance and use the validated API-returned web URL as canonical provenance, including `/-/work_items/IID` when returned. Never rewrite an unverified work item into an issue or force the server's verified canonical URL into another spelling.
12. For self-managed installations, treat the configured `base_url` path and CLI host/subfolder authority as part of identity. A matching hostname with a different base path is a different provider instance.
13. Validate numeric IID boundaries and reject zero, signs, overflow, whitespace variants, and path components that decode into separators.
14. Keep local origin matching tied to the primary verified checkout; a secondary remote or arbitrary typed repository path cannot authorize setup by itself.
15. Normalize comment ordering deterministically and retain continuation/truncation metadata so refresh can distinguish a complete result from a bounded result.
16. Do not pass arbitrary user URL fragments as shell text. Build argv as discrete arguments and redact command diagnostics before returning them to protocol/UI layers.
17. Ensure provider timeout, nonzero exit, invalid JSON, and empty response preserve the distinction between authentication failure, not found, unsupported type, and transient availability.
18. Verify equivalent `/-/issues/IID` and `/-/work_items/IID` forms by returned resource type, project and IID rather than literal URL spelling. The inventory fixture returned the work-item form; a canonical URL on another authority is not equivalent.
19. Keep source assets deterministic and bounded so the same issue fetched twice yields stable source identity while changed content yields a freshness change.
20. Leave provider writes impossible from this adapter: reject mutation verbs in the command builder and keep fixture issue creation outside production code.

## Non-goals

- No issue creation, editing, commenting, closing, label mutation, project mutation, clone, checkout, branch push, or MR write.
- No GitLab MR, wiki, diff, pipeline, structured review-anchor, or generic provider framework; GLAB-02 owns standalone MRs.
- No blanket URL coercion of all GitLab work items and no fake success for unsupported issue types.
- No token transport through Cockpit and no replacement of gh/tea adapters.
- GitHub PR expansion and Jira source expansion remain separate deferred work; GitHub issue behavior must not regress.

## Acceptance

1. The fixture issue `#1` imports from its work-item URL only after API validation confirms `issue_type: issue`; canonical ID and source URL remain distinct and correct.
2. Nested namespace projects, self-managed host/base paths, ports, and local origin matching resolve to one identity; wrong host/path/IID and encoded traversal forms fail before cache/materialization.
3. Captured fake-glab execution proves every API call is explicit GET, no token is passed or printed, and no mutation verb occurs.
4. Metadata/comments are bounded, preserve required identity/timestamps/URLs, and report truncation explicitly at the shared budget.
5. Auth, permission, malformed-response, missing-resource, rate-limit, and unsupported-work-item errors render as genuine stable failures in setup/import/refresh.
6. Source ID, cache, refresh, and materialization use existing provider-neutral contracts and remain distinguishable across host/path/resource type.
7. Existing gh and tea issue/source flows continue to pass their real acceptance scenarios, and unsupported capabilities are reported rather than faked.

## Verification

Use a fake glab executable or local API fixture first to inspect exact argv, GET methods, host selection, redaction, paging, and negative mappings. Then run the real read-only `glab` 1.118.0 path against the authorized issue in a uniquely named disposable Herdr session and local checkout. Exercise setup defaults, plan, import, list, and refresh; inspect source identity and bounded comments. Do not enumerate unrelated projects or print auth state. Record fixture ownership, project ID, issue URL, before/after state, and cleanup in durable evidence.

The durable run record must include the exact source URL form supplied, canonical URL/ID after validation, provider host/base path, bounded page counts, truncation state, error code, and fixture ownership.

Real acceptance should exercise both `/-/issues/IID` and the authorized `/-/work_items/1` URL where the API can prove the same issue resource; a work-item URL that resolves to another type remains a negative case.

## Handoff

Return changed provider/core/protocol paths, canonical identity rules, exact CLI/API contract, error mapping, and fake/real evidence locations. Include any self-managed host limitation and whether it blocks acceptance. Commit implementation and durable evidence separately or in the owned task commit as required by [../ORCHESTRATOR.md](../ORCHESTRATOR.md); a source parser or mock-only result is not completion.
