# SETUP-02 — Recover companion setup safely across platforms

Status, dependencies, owner and locks: [task ledger](../tasks.json). Follow [the orchestrator contract](../ORCHESTRATOR.md).

## Outcome

Companion publication works safely on Linux and macOS without overwriting an existing destination. A known unsupported platform fails before creating Herdr resources; failures that race or occur after creation leave an explicit recoverable operation with exact ownership. Real macOS proof is required.

This addresses the companion-publication and unexplained partial-resource portion of [GitHub #7](https://github.com/nnexai/cockpit/issues/7). It is not a new filesystem portability framework.

## Evidence and starting points

- `crates/cockpit-core/src/project_store.rs::publish_companion_no_replace` uses Linux GNU `renameat2` and explicitly returns Unsupported on other targets. Read its staging, directory-descriptor and journal ownership code together.
- `crates/cockpit-core/src/projects.rs` owns setup sequencing, partial state, retry/reconcile and owned-resource receipts; use the existing recovery model.
- `project_teardown.rs`, `context_assets.rs` and `sources.rs` establish cleanup, materialization and local-edit conflict boundaries to preserve.
- `src/app/projects/SetupDialog.tsx` renders progress/recovery; browser project routes and native commands share the project contract.
- Latest decisions preserve borrowed directories, exact create receipts and no-replace publication. Linux-only historical evidence does not establish Darwin behavior.

## Changes

1. Implement the supported Darwin atomic no-replace primitive using the existing descriptor-bound parent/staging model. Keep Linux semantics unchanged. Do not use destination-exists then ordinary rename, delete-then-rename, or copy-and-delete as an atomic publication substitute.
2. Preflight known platform/primitive and parent-path prerequisites before the first Herdr mutation, reusing existing validation. This is not a promise that permissions, disk capacity or paths cannot change later: those failures still need durable recovery.
3. Preserve one publication commit point: fully prepare operation-owned staging, perform required supported durability steps, then claim the final destination without replacement. Do not expose a half-populated final companion.
4. If the destination appears concurrently, preserve it unchanged. Distinguish a verified companion already published by this same operation from a foreign destination using existing receipt/manifest identity; retry may reuse only the former.
5. Reconcile interruption before/after publication and before journal acknowledgement. Preserve enough existing operation evidence to identify staged, published or uncertain outcomes; augment it only when a demonstrated crash window requires it.
6. Surface exact partial resources and valid recovery actions. Resume must not repeat a successful worktree/terminal creation, overwrite generated user edits, or automatically delete a potentially useful partial worktree.
7. Clean only verified operation-owned staging or explicitly confirmed owned resources. Cleanup is idempotent and refuses identity/path substitution, symlink redirection or borrowed resources.
8. Keep source fetch/materialization errors distinct from publication/platform/Herdr failures so the UI offers the correct recovery. Show user-relevant destination/effects and failure, not unnecessary filesystem implementation details.
9. Require filesystem/lifecycle review of atomicity, crash windows, ownership, symlink/race behavior and both target platforms before completion.

## Non-goals

- No Windows port, generic filesystem layer, new companion format or unrelated journal rewrite.
- No additional generic disk/path preflight subsystem; reuse existing limits and handle real races at the operation boundary.
- No recursive deletion of unowned or arbitrary abandoned directories and no delete-then-replace rollback.
- No product GitLab mutation, credential store, or changes to repository-action/trust policy.
- No claim of macOS correctness from Linux mocks/cross-compilation alone.

## Acceptance

1. Real Linux and macOS setup publish a complete companion without exposing a partial final directory or replacing an existing unrelated destination.
2. A known unsupported capability fails before Herdr creation with an actionable diagnostic. Later permission/disk/runtime failures retain the exact recoverable partial-resource record.
3. Existing and concurrently created foreign destinations remain unchanged, including empty directories; a same-operation published destination is reused only after ownership validation.
4. Interruption during staging, publication and receipt persistence can be reconciled without guessing, duplicating resources or deleting foreign data.
5. Retrying partial setup/source materialization does not duplicate worktrees/terminals, overwrite local edits or acquire ownership of borrowed paths.
6. Symlink/path replacement, cross-device and permission/full-disk failure injections fail safely. Unrelated sentinel files survive all cleanup.
7. Browser and native surfaces expose consistent partial/recovery results; actual macOS native setup exercises the production publication path.
8. Every created fixture path/process/resource is cleaned or retained with a named next consumer; absence of a macOS runner leaves verification blocked, not passed.

## Verification

Use run-owned repositories, companion/state roots and unique Herdr sessions on Linux and macOS. Inject interruption at the production publication/journal boundaries, inspect authoritative inventory and directory contents, then resume/reconcile. Include a foreign empty destination, concurrent destination creation, path substitution and a same-operation completed-publication retry. Run narrow existing store/project/teardown regressions after integration and actual browser/native setup proof; fixture-only mocks are insufficient for Darwin's primitive. Record which guarantees the filesystem actually supplies.

## Handoff

Commit the scoped publication/recovery changes, necessary tests/docs and `runs/<run-id>/SETUP-02.md`. Record platform/primitive, interruption point, operation/manifest identities, partial resources, native/browser observations, sentinel preservation and cleanup. The ledger receives the actual commit/evidence reference only after all required criteria—including macOS—pass.
