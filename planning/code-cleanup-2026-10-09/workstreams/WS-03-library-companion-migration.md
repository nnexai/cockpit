# WS-03 Library and companion-root migration removal

Wave 1 · Size L (~1,000–1,100 lines deleted) · Depends on: WS-02 · Blocks: WS-12, WS-13, WS-14
**Integration owner for `crates/cockpit-core/src/library*` in this wave.**

## Goal
The Library opens only current-format data. No code reads, upgrades or protects companion folders, old Space refs or old index/journal schemas. The retired `companion_root` setting is gone end to end.

Source of truth: `../migration-inventory.md`, sections "Library" and "Config / companion", and coupling notes 1–3. **Refresh the line numbers before editing.**

## Owns
- `crates/cockpit-core/src/library.rs`
- `library/{legacy,store,space,folder,jira_follow,sync}.rs`, `library/folder/tests.rs`, `library/sync_tests.rs` (fixture lines)
- `sources.rs` (SourceAsset defaults and old-shape tests)
- `config.rs` (companion root; retired Jira/Confluence `executable`/`login` rejections; the `profile` fixture)
- the companion env lines in `orchestration/dispatch.rs:~676` (only those lines; message WS-05)
- `crates/cockpit-herdr/src/cli/orchestration.rs:~242` (env allowlist)
- `crates/cockpit-protocol/src/projects.rs` (`companion_root` field only; WS-04 edits other hunks of this file)
- `src/client/projectProtocol.ts` + test (including the obsolete companion-teardown negative cases, `projectProtocol.test.ts:~127-151`)
- the WS-02 builder
- `scripts/verify/ui_polish_runtime.py` (stop writing `companion_root`)

## Change (in this order)
1. Remove `store.rs` `upgrade_v2`/`upgrade_v3`/`upgrade_v2_entry`/`upgrade_v2_follow`, their open-time calls, the schema-1 special error, and the five upgrade tests.
2. Remove the `space::migrate_target` call and its rationale. If the empty mutation in `space_listing` existed only to run that migration, remove it too. Listing then authorizes once and takes no write lock; keep fresh authorization.
3. Delete `mod legacy` and `legacy.rs`, plus `library_open_never_imports_or_mutates_legacy_or_companion_files`.
4. Remove the Jira "fetch once because old saves lack references" recheck branch (`jira_follow.rs:~62-77`) and its test. Do not replace it with an unconditional refetch.
5. Remove old-only serde defaults in sync state and `SourceAsset`, plus `old_source_assets_deserialize_with_empty_extensions`. In `fetch_only_neither_initializes_nor_reads_or_changes_legacy_cache` (`sources.rs:~1759-1811`), drop only the old `sources/source-current.json` fixture and its assertions, then rename the test. Keep its fetch-only vs prefetch and provider-instance assertions.
6. Retire `companion_root`, in this order:
   - config resolution and validation, and the overlap entry;
   - dispatch env injection and the Herdr env allowlist;
   - the DTO field, TS validator and test builder;
   - the fixture helper.
   Then regenerate TS.
7. Delete the helpful rejections of retired keys; generic unknown/invalid handling remains.

## Keep (do not delete)
- Current schema-4 index and schema-2 journal checks, `recover`, locks and leases.
- refs/inclusions and Space selections.
- The **serialized fields `legacy_migrated` and `relations_captured`**. Remove their upgrade consumers only.
- Overlap protection for the Library/Notes/state/cache/worktree/repository roots.
- The no-copy behaviour of setup.
- Any user folders on disk: never read, write or prune them.

## Acceptance
- No `legacy`, `upgrade_v` or `companion` symbols remain in the owned files, except CURRENT items in the inventory.
- `ProjectConfiguration` has no `companion_root`.
- A config file containing `companion_root` fails with the generic unknown-key error.
- The fixture helper starts a session.

## Verify
- `cargo test -p cockpit-core library:: sources:: config::`, `cargo test -p cockpit-herdr`, `bun run test -- src/client/projectProtocol`.
- Live run (`skill://cockpit-library-confluence-e2e`, `skill://cockpit-library-jira-e2e`) in a disposable Library:
  - import a Jira issue and a Confluence page;
  - follow and refresh;
  - select an item in a Space;
  - restart the host and re-open the same Library.
- Browser smoke of the Library view on a disposable fixture started by the updated helper.

## Doc notes for WS-25
Inventory "Docs" items for Library/companion: `DECISIONS.md` ~138/139/145, `CODE_GUIDE.md` ~35/192, `CONTEXT.md` ~285.
