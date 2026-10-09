# WS-15 `projects.rs` split

Wave 2 · Size M · Depends on: WS-04 · Blocks: –

## Goal
Task setup reads as four phases (plan, execute, reconcile, teardown), each in its own module, with an explicit step table.

## Owns
`crates/cockpit-core/src/projects.rs` → `projects/{mod,plan,execute,reconcile,teardown}.rs`. Coordinate with `project_teardown.rs` if helpers move.

## Evidence
- `plan` is ~300 lines (`~286-589`): validation, templates, path containment, linked artifacts.
- `execute_inner` is 355 lines (`~1538-1893`).
- `reconcile` is 200 lines (`~1177-1376`).
- The teardown functions are at `~727-1099` and `recover_startup` at `~2044`.
- The step ordering is implicit in `step_at_least` (`~2102`) and `pending_unknown` (`~2038`).

## Change
1. Move the code into phase modules. `ProjectService` stays the public facade with unchanged signatures.
2. Split `plan` into validate / resolve-template / contain-paths / link-artifacts helpers.
3. Express the execute steps as an ordered table of `(step, action, receipt)`. Derive `step_at_least` and `pending_unknown` from it.
4. Move the tests next to their phase.

## Keep
- Operation IDs, generations and ownership receipts.
- Idempotency and unknown-outcome semantics.
- The public API.

## Acceptance
- No function over 150 lines in `projects/`.
- The existing tests pass, with only module-path edits.

## Verify
- `cargo test -p cockpit-core projects::`.
- Disposable fixture: Open, Create worktree, restart the host mid-setup (reconcile resumes), then tear down both.
