# WS-14 Library refresh and related-traversal unification

Wave 2 · Size L · Depends on: WS-03 · Blocks: –
**Integration owner for `crates/cockpit-core/src/library*` in this wave.** Data-loss sensitive: one round of independent `reviewer` review (high-severity only).

## Goal
"Refresh a follow" and "traverse related items" each exist once. Manual refresh and background sync call the same pipeline with a different window.

## Owns
- `crates/cockpit-core/src/library/{follow,jira_follow,sync,related,refs}.rs`
- `library/store.rs` (test move only)
- `crates/cockpit-core/src/sources/{references,lane}.rs`
- new `library/follow_plan/` modules and sibling `tests.rs` files

## Evidence
- There are three refresh implementations:
  - `follow.rs` `refresh_follow` (`~266`; Confluence, then dispatches Jira);
  - `jira_follow.rs` `refresh_jira_follow` (488 lines, `~353`);
  - `sync.rs` `discover_follow` (266 lines, `~606`), which repeats absence confirmation (`~319-336`) and the accumulate key checks.
- There are three related-traversal drivers:
  - `references.rs` `collect_related` (233 lines);
  - `related.rs` `run_related`, `apply_reference_depth` and `refresh_related`;
  - `sync.rs` `sync_related_due` (191) and `sync_single_related_due` (162).
- There are two `change_reason` functions (`follow.rs:~73`, `jira_follow.rs:~66`).
- `lane.rs` duplicates the config defaults 10/1/300.
- Decision-ID comments (`S6`, `D17`, `D20`, `D4`) need the decision log to make sense.
- The inline tests make `store.rs` (3,041 lines), `jira_follow.rs` and `follow.rs` large.

## Change
1. Add a per-provider `FollowPlan`: list → classify (`ChangeReason` enum with `Display`) → confirm absence → fetch → related. It takes a window (full manual or delta) and is used by all three callers.
2. Add one traversal engine that takes a holder (item or follow) and seeds. All three callers use it, keeping the same caps (`TraversalBudget`) and completeness reporting.
3. Split `discover_standalone` (314 lines) and `drain_sync` (244) along their phases.
4. Give `BackgroundPolicy` defaults one source: delete the hard-coded 10/1/300 in `lane.rs` and derive them from the existing `LibrarySyncConfiguration` defaults. **Do not edit `config.rs`** (WS-12 owns it in this wave).
5. Replace the decision-ID comments with one-line plain rationale.
6. Move the inline tests of `store.rs`, `follow.rs` and `jira_follow.rs` into sibling `tests.rs` files.

## Keep
- **Never drop, untag or tombstone on an incomplete pass.**
- Pacing, budgets and request caps.
- Live/Accumulate semantics and the `[library_sync]` keys (out of scope).
- Journal publication and move behaviour.
- The persisted format.

## Acceptance
- One refresh pipeline and one traversal engine.
- No function in `library/` or `sources/` is over 300 lines.
- `sync_tests.rs`, `sync_safety_tests.rs` and the follow tests pass with only import changes.

## Verify
- `cargo test -p cockpit-core library:: sources::`.
- `cargo test -p cockpit-providers site_http::pacing::tests:: -- --test-threads=1`.
- Live run (`skill://cockpit-library-confluence-e2e`, `skill://cockpit-library-jira-e2e`, `skill://cockpit-scalable-atlassian-sync-research`):
  - Confluence space follow;
  - JQL follow in both modes;
  - manual refresh reporting unchanged items;
  - background delta sync with an accelerated policy;
  - reference depth 1–2;
  - an interrupted pass never removes members.
