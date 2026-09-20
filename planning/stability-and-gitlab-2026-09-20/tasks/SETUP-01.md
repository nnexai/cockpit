# SETUP-01 — Separate source inspection from explicit setup

Status, dependencies, owner and locks: [task ledger](../tasks.json). Follow [the orchestrator contract](../ORCHESTRATOR.md).

## Outcome

Inspecting an issue/MR URL never creates a Herdr Space, checkout, companion or terminal. An explicit New Space action presents the exact authoritative setup effects before the user approves their execution. Preserve deliberate user edits and reconcile uncertain operations without duplicate setup.

This is [GitHub #7](https://github.com/nnexai/cockpit/issues/7), not a new generic workflow/receipt framework. Read-only here refers to user resources: bounded metadata caches and existing durable plan journals may be written under Cockpit-owned state roots. They cannot confer ownership of a checkout or companion.

## Evidence and starting points

- `src/app/projects/SetupDialog.tsx::submit` currently calls `planWorkspace` and immediately `startWorkspace`; trace the reported issue-opening gesture through this boundary before repairing it.
- `crates/cockpit-core/src/projects/defaults.rs` resolves source metadata and repository matches; `repositories.rs` validates artifact/origin identity.
- `crates/cockpit-core/src/projects.rs` already separates plan/start and performs provider validation; reuse its plan IDs, generations and operation receipts.
- `project_store.rs` journals plans and owned effects; `src/client/projectProtocol.ts` validates the existing contract.
- Browser project routes and native commands must expose the same behavior. UI components do not invent URL parsing or call provider CLIs directly.
- Latest `DECISIONS.md`: configured repository actions run automatically when explicit setup starts; no per-operation consent checkbox is required, and Cockpit never sets Herdr's `trust_repository` override. Existing-directory Open is path-only and may be non-Git.

## Changes

1. Trace all source entry/open/defaults handlers. Separate bounded source inspection from explicit setup intent; remove any path that starts setup merely because a source resolves or a link is opened.
2. Reuse the existing authoritative plan to show canonical source/provider, selected local repository (when needed), branch, label, checkout path, companion effects, source hydration and configured repository actions before mutation.
3. Start only the plan the user reviewed. Editing relevant inputs invalidates the preview. If fresh revalidation changes an effect, show the changed plan for confirmation rather than silently execute it. Keep the interface compact; a new general-purpose wizard is unnecessary.
4. Preserve explicit repository/branch/label/path/source edits against delayed defaults. Explicit values win; ambiguous repository matches require selection. A repository-only create and path-only Open remain usable without a provider.
5. Keep provider and origin validation before Herdr mutation for source-based setup. Authentication, malformed source or unsupported artifact failures remain visible and create no task resources; never silently fall back to another provider/repository.
6. Reuse existing operation ID/generation and durable receipt semantics after Start. Double-click, dialog close, lost response and client reconnect must inspect/reconcile the same operation, not dispatch another create.
7. Preserve partial-source recovery and owned-resource accounting. Do not redesign the journal or add fields unless a demonstrated contract gap requires the smallest shared protocol change.
8. Keep current repository-action/trust behavior explicit in the reviewed effects. This is approval of the setup mutation, not restoration of removed trust/consent controls.
9. Require lifecycle/ownership and UI review of the plan-to-start boundary, stale responses, operation identity and preservation of path-only Open.

## Non-goals

- No GitLab adapter implementation or remote fixture writes in this task.
- No prohibition on existing owned metadata caches/plan journals; do not equate their writes with worktree creation.
- No generic operation schema/version framework, new trust setting, automatic clone, forced checkout, or deletion of borrowed paths.
- No provider requirement for ordinary local directory opening.
- No weakening of existing Herdr authority, source origin checks, or uncertain-outcome semantics.

## Acceptance

1. Opening/previewing a valid source, changing URLs rapidly, cancelling and closing the dialog produce no new Herdr Space/worktree/terminal or companion/source materialization. Before/after inventory distinguishes allowed Cockpit state/cache writes.
2. Explicit setup displays the exact repository, branch, label, checkout/companion paths and effects before mutation; only approval of that still-current plan dispatches Start.
3. Delayed defaults never overwrite explicit edits. An invalidated/revalidated plan cannot execute effects the user did not review.
4. Provider/auth/identity failures appear before mutation, preserve entered values and leave no unexplained task resource.
5. Repository-only Create and plain/nested path-only Open still work without an issue/provider, preserve configured repository actions and leave `trust_repository` unset.
6. Repeated clicks, closing/reopening after dispatch and a lost Start response retain one operation identity and do not duplicate resources. Unknown outcome stays distinguishable from rejection and success.
7. A partial source failure preserves valid receipts and supports only owned recovery; it never retries already successful unrelated effects.
8. Actual browser and Linux-native setup surfaces demonstrate matching preview, explicit start, stale/defaults and recovery behavior.

## Verification

Use a uniquely named disposable Herdr session/config with a fixture repository, plain directory and nested borrowed path. Record authoritative inventories, paths and operation IDs before/after source preview and explicit Start. Inject delayed defaults, changed plan inputs, provider failure, duplicate click and lost response after dispatch. Observe the rendered recovery state and reconcile the exact receipt. Run focused existing setup/protocol tests after integration; add only plausible behavior regressions. No user-session or installed-application mutation.

## Handoff

Commit owned implementation, necessary authority/docs/test updates and `runs/<run-id>/SETUP-01.md` with exact action/result evidence, operation identity, reviewed effects and cleanup. Explain any intentionally changed UI semantics. The orchestrator records the real SHA and evidence path; a component test or screenshot without authoritative resource accounting is not completion.
