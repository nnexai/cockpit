# GLAB-03 — Stabilize provider refresh conflicts and resource UI

## Outcome

Make GitLab issue/MR refresh trustworthy and usable in the existing Context Resources panel while preserving established gh and tea behavior. Refresh must distinguish unchanged, changed, conflict, unavailable, truncated, and failed provider results; retain the last confirmed resource list during transient failure; and make bounded scrolling and status usable at the campaign browser sizes. Use the existing SourceProvider, SourceService, SourceCapability, cache, and protocol contracts. Do not create a second provider registry or silently migrate gh/tea.

This brief follows GitHub issue [#6](https://github.com/nnexai/cockpit/issues/6) and depends on GLAB-02. Read [../ORCHESTRATOR.md](../ORCHESTRATOR.md) and [../tasks.json](../tasks.json); baseline is `6f6222b74e4f552ce697e61364cf653f4b6be29f`. The existing browser matrix is `planning/inline-space-browser-2026-09-13/03-delivery-and-verification.md`; A23 is excluded and all other relevant behavior must remain accounted for across the campaign.

## Evidence and starting points

- `crates/cockpit-core/src/sources.rs` owns immutable hash-addressed cache, current pointers, freshness, refresh, hydration budgets, and user-edit conflict preservation.
- `crates/cockpit-core/src/context.rs` authorizes source operations and derives checkout authority; do not bypass it for GitLab.
- `crates/cockpit-host/src/server/sources.rs` exposes browser import/list/refresh routes.
- `crates/cockpit-protocol/src/sources.rs` defines neutral capabilities, entries, diagnostics, freshness, and requests.
- `src/client/sourceProtocol.ts` validates statuses, diagnostics, bounds, and duplicate source IDs.
- `src/app/context/SourceImport.tsx` selects configured providers, imports/refreshes, and renders source status/diagnostics.
- `src/app/context/ContextResources.tsx` composes source/snapshot resources and emits `context-resources`.
- `src/app/context/context.css` lacks a dedicated bounded resources-panel selector; existing source-entry rules do not prove usable scrolling.
- `crates/cockpit-providers/src/github.rs` and `tea.rs` are regression authorities; preserve their capabilities and failure behavior.

## Changes

1. Define refresh result semantics using existing protocol fields: fresh/unchanged, provider changed, local conflict, unavailable/auth failure, truncation, malformed response, and resource-limit failure must remain distinguishable and actionable.
2. Preserve the last confirmed list and selected source while import/refresh is pending or fails. Do not clear the list or render an empty success state during a transient network/auth/provider error.
3. Ensure a same-SHA GitLab title/body/comment/discussion change reaches the freshness comparison from GLAB-02. Do not weaken freshness to SHA-only to simplify UI or cache handling.
4. Keep user-edited generated files intact on refresh. Surface conflict identity and the provider revision/content difference without silently replacing local content.
5. Bound provider data and UI rendering: use the shared page/byte/asset limits, display truncation/availability diagnostics, and avoid unbounded comment/discussion expansion or layout growth.
6. Add a bounded scroll container and usable focus/keyboard behavior for Context Resources. Verify the source and snapshot lists remain reachable at 1440x900 and 1024x640 without clipping the import/refresh controls.
7. Show provider kind/capability and issue versus review status honestly. A provider that cannot supply a capability must display unavailable/unsupported state, not a fake success or generic silently empty panel.
8. Reuse existing provider configuration and selection. If URL-based selection or capability filtering is needed for GitLab, make the smallest compatibility change and prove gh/tea choices, login handling, and existing issue imports remain unchanged.
9. Keep browser/native source protocol responses equivalent. Do not add a GitLab-only transport or context cache, and do not migrate to an explicit provider registry unless implementation evidence demonstrates that the current dispatch cannot safely support the required behavior.
10. Add the required review type: provider-compatibility review for gh/tea preservation and UI/accessibility review for failure retention, bounded scroll, focus, and status semantics.

11. Keep source selection and scroll position stable across pending responses where the selected source still exists. If it disappears after a confirmed refresh, explain the change and select deterministically rather than jumping during every render.
12. Make status text and diagnostics accessible to keyboard and assistive technology users without relying on color or transient toasts alone.
13. Ensure long titles, namespace paths, comment bodies, and diagnostics wrap or truncate within the bounded panel; they must not force horizontal overflow.
14. Keep refresh controls disabled only for the relevant in-flight source and restore them after timeout/failure with a clear retry action.
15. Preserve source entries when an imported asset is unavailable locally, distinguishing provider availability from local hydration/materialization failure.
16. Bound refresh concurrency and deduplicate repeated requests so rapid clicks cannot create overlapping writes or race current pointers.
17. Verify current-pointer updates are atomic and a failed refresh leaves the previous immutable cache record usable for display and retry.
18. Retain diagnostics across browser/native transport boundaries with stable codes, not provider stderr copied into a user-facing unbounded blob.
19. Test both empty and crowded resource states: no sources should explain how to import, while many sources should remain navigable without hiding the import action.
20. Keep CSS changes scoped to the context resources surface and existing design tokens; do not restore excluded docks or invent a parallel layout system.

## Non-goals

- No new provider registry, generic provider framework, credential store, or URL coercion policy without demonstrated necessity.
- No issue/MR writes, checkout/merge operations, remote branch operations, or production fixture mutation.
- No redesign of Context/Review, no legacy browser-pane restoration, and no A23 behavior.
- No clearing cached resources on error, no fake success for unavailable discussions/capabilities, and no unbounded rendering.
- GitHub PR and Jira remain deferred separate work; gh/tea issue behavior is protected.

## Acceptance

1. A refresh with changed GitLab content reports changed/freshness state and preserves local edits with an explicit conflict rather than overwriting.
2. A same-SHA title/body/comment/discussion update is detected and displayed as changed under the bounded policy.
3. Auth, network, provider-unavailable, malformed, truncation, and resource-limit failures retain the last confirmed list and expose actionable diagnostics.
4. Unsupported capabilities remain visibly unavailable; no empty or fabricated success state is presented.
5. Resource lists scroll within a bounded panel, controls remain reachable, and keyboard/focus behavior works at both documented browser sizes.
6. Browser and native list/import/refresh contracts remain equivalent and source identity is stable across refreshes.
7. Existing gh and tea issue/import/refresh scenarios pass unchanged; no provider registry migration is claimed unless a concrete blocking seam and compatibility evidence are recorded.
8. Refresh evidence covers both GitLab issue and standalone MR forms where fixtures permit; missing MR fixture remains a recorded blocker, not an inferred pass.

## Verification

Use deterministic provider/cache fixtures for unchanged, metadata-only change, same-SHA change, local edit conflict, auth failure, unavailable capability, truncation, and malformed response. Exercise the actual browser resource panel with a long source list at 1440x900 and 1024x640, including keyboard focus and pending/error transitions. Run native source list/refresh in a disposable session and compare protocol outcomes. Re-run existing gh/tea real acceptance paths. Do not mutate production or protected Herdr sessions; record resource ownership, fixture permissions, and retained-list observations in durable evidence.

## Handoff

Return changed source/protocol/UI paths, status/freshness semantics, scrolling/accessibility observations, and gh/tea regression evidence. Include any provider-registry decision with concrete evidence, not preference. Completion requires a real implementation commit and durable evidence under the campaign contract; static CSS inspection or a passing mock alone is insufficient.
