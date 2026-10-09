# WS-04 Project-store migration removal

Wave 1 · Size M (~340–400 lines deleted) · Depends on: WS-02 · Blocks: WS-15

## Goal
Setup/teardown journals and receipts decode only the current typed format. No ownership inference for old journals remains.

Source of truth: `../migration-inventory.md`, section "Projects / project_store", and coupling note 4.

## Owns
- `crates/cockpit-core/src/project_store.rs`
- `crates/cockpit-protocol/src/projects.rs`: only the ownership `default()` hunks (`~174-179`, `~199`). WS-03 owns the `companion_root` hunk; the hunks are disjoint.
- The migration fixture and assertion portions of the `crates/cockpit-core/src/projects.rs` tests (`~2962-3125`).

## Change
1. Delete `migrate_legacy_operation_json`, `migrate_legacy_teardown_receipt` and `migrate_legacy_operation`.
2. Switch the two readers (`~466-473`, `~514-522`) from the raw-JSON intermediate plus read-time rewrite to the existing typed bounded reads.
3. Delete `omit_legacy_ownership`, `legacy_companion`, `legacy_operation` and the eight legacy tests listed in the inventory.
4. Remove `WorkspaceCheckoutOwnership::default()` and the plan ownership default after auditing their callers. Regenerate TS if anything changes.
5. Trim the old sources-cache and companion seeding in the `projects.rs` tests. Keep the scenarios around it.

## Keep
- `receipt_matches_operation`, journal locks, no-follow reads and bounds.
- The mutation generation for real writes, exact receipt matching and unknown-outcome handling.
- The linked-artifact request defaults (`~155`, `~207`): they are not ownership inference.

## Acceptance
- `project_store.rs` has no `legacy`/`migrate` symbols.
- Journals are read without rewriting the file.

## Verify
- `cargo test -p cockpit-core project_store:: projects::`.
- Disposable fixture via the browser:
  - Open an existing directory;
  - Create a task worktree;
  - restart the host (reconcile);
  - tear down both.
  Check that the ownership receipts are correct and nothing outside the fixture is touched.
