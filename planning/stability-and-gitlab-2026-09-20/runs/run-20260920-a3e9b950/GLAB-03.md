# GLAB-03 — retained resources and safe refresh publication

## Accepted implementation plan

Accepted by Main, 2026-09-21 under OBS-011 against returned GLAB-01/02 source interfaces. Original task criteria, gh/tea preservation, real MR fixture limitation and completion dependency on GLAB-02 remain unchanged. This is implementation, not acceptance.

Observed UI seams: SourceImport couples configuration/list loading with Promise.all, displays no-provider on load failure, globally disables every refresh, and moves updated entries to the end. ContextResources declares a modal dialog without a dedicated bounded panel layout. SourceService cache publication and concurrent provider fetch completion require scoped review before edits.

### Ownership and fixed contract

- Resource UI worker owns SourceImport.tsx, ContextResources.tsx and resource-scoped context.css plus meaningful related tests. Do not edit ContextViewer, App, browser UI or core. Reuse existing modal/focus and design-token conventions. Preserve entries, row order, selected resource and scroll during same-scope pending/failure; clear only on a genuinely different authorized context. Separate configuration/list failure from empty configuration. Render existing freshness/materialization/diagnostic codes honestly, including incomplete/unknown and local conflict. Bound the panel/lists with reachable import/close/snapshot controls at both required sizes. Use a bounded per-source request map (at most two active operations), deduplicate repeated source actions, and disable only the affected controls; report temporary capacity rather than silently dropping another source request. Abort/suppress superseded UI completions.
- Refresh core worker owns sources.rs and necessary existing source-cache tests only. Preserve the existing nonblocking cross-host import lease spanning bounded fetch/materialization and the separate short cache-publication lock; these already serialize source writes, so do not add another lock, CAS layer or retry queue. Preserve immutable records/current-pointer atomicity, descriptor safety, capacities and user-edit conflicts. Improve bounded previous/current revision diagnostics and distinguish successful provider fetch from local materialization failure using existing entry/status/diagnostic fields. Inspect failure paths before editing; no new cache model.
- Use existing SourceEntry, SourceFreshness, SourceMaterializationStatus and SourceImportResponse.diagnostics. Preserve bounded previous/current revision and content identities in refresh diagnostics where needed to explain changes without unbounded history or provider-specific parsing in core. No new source/cache model, transport, credential access or registry.
- Main owns provider revision-component handoff, any necessary shared integration, generated files, verification and commits. Existing gitlab.rs/gh/tea implementations are frozen unless Main explicitly assigns a demonstrated compatibility repair.

### Verification

Workers skip all formatters, linters, builds, tests and runtime during writing. Keep only meaningful list-retention/identity/concurrency/conflict regressions. Provider-compatibility and UI/accessibility review, deterministic negative cases, actual browser sizes/keyboard checks, native equivalence and real gh/tea/GitLab acceptance remain for final verification. No remote mutation or completion claim.

## Compact proof before consolidated batch

Backend diagnostics/materialization handling is committed in `7d06e6e`; the focused cache-preservation failure regression passed. Resource UI review repairs, typecheck and frontend build passed. One actual gateway scenario imported the owned issue #2 and standalone MR !1, inspected both rendered source documents and their discussion/comment markers, then refreshed the issue. Both rows stayed present and materialized; refresh visibly reported unchanged. Main inspected [the screenshot](GLAB-resource-ui.png) and corrected the Files pane identity to authoritative `w3:p4` in [the evidence](GLAB-resource-ui-evidence.json).

No changed-content, conflict, remote mutation or native result is claimed by this scenario. The split-pane screenshot exposed excessive row spacing; BatchWorkflows repaired that styling under OBS-015. Final integrated rendering and remaining focused checks occur after all consolidated writers settle, not through another standalone GLAB-03 delivery loop.
