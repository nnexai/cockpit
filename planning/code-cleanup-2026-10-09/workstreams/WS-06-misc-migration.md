# WS-06 Comments, browser, review, quota and protocol migration removal

Wave 1 · Size M (~400–550 lines) · Depends on: WS-02 · Blocks: WS-16, WS-17, WS-20

## Goal
Remove the remaining old-format readers and old-shape test cases outside the Library, projects and orchestration.

Source of truth: `../migration-inventory.md`, sections "Comments", "Browser", "Review", "Quota" and the generic protocol items, plus coupling notes 5, 8 and 9.

## Owns
- **Comments:** `crates/cockpit-core/src/comments/{store,mod}.rs`; `crates/cockpit-protocol/src/comments.rs`; `src/client/commentProtocol.ts` + test; the `src/app/context/CommentDrafts.test.tsx` fixture lines (WS-11 does not edit this file).
- **Browser:** `crates/cockpit-core/src/browser.rs` receipt serde defaults only (`~130-148`); `browser/drafts.rs`; `browser/saved_work_tests.rs`; `ephemeral.rs` (old-name fixture only).
- **Review:** `review.rs` pre-viewer pruning (`~2722-2732`) and its test.
- **Quota:** `quota/tests.rs` (two old-only tests).
- **Protocol and client:** `crates/cockpit-protocol/src/{v1.rs:~50,viewer.rs:~71}`; `crates/cockpit-protocol/tests/typescript.rs`; the capabilities/sequence normalization in `src/client/CockpitClient.ts` (`~550-593`, `~1579`); `src/client/{client,libraryProtocol}.test.ts`. The obsolete companion-teardown cases in `projectProtocol.test.ts` belong to WS-03, not to this workstream.
- **CLI:** the `crates/cockpit-host/src/bin/cockpit.rs` `--legacy` rejection test.

## Change
1. **Comments:** delete the constructor migration, `migrate_legacy_batches`, the `LegacyPane` validation arm, the DTO variant and the frontend parse arm. Convert the shared fixtures to detached `Viewer` owners. Delete only the old-specific tests.
2. **Browser drafts:**
   - delete the old digest/preparation defaults, the old-capture fallback, `normalize_legacy_editor` and the `draft_annotation_matches_capture` helper;
   - require equal id/digest counts (two empty arrays stay valid);
   - update the cleanup fixtures to the current digest shape.
3. **Review:** delete only the old `snapshot-`/`file-` namespace cleanup.
4. **Protocol:** delete the missing-capabilities and missing-`state_change_seq` normalization in Rust and TS, the retired scope/retention negative cases, and the obsolete negative fixtures.
5. Regenerate TS.

## Keep
- Viewer detach/reattach and uniqueness/CAS safety.
- `opened_tab` and the browser lifecycle "retired" guards.
- Current browser draft format-1 compaction.
- Viewer-bound review cache validation.
- Quota schema-2 validation and lease omission.
- Genuinely optional transport fields.
- `run_legacy` (a live CLI dispatcher; rename at most).

## Acceptance
- `CommentOwner` has a single current variant.
- No `legacy`/`normalize_legacy` symbols remain in the owned files.
- The strict-validation tests still reject unknown fields generically.

## Verify
- `cargo test -p cockpit-core comments:: browser:: review:: quota::`, `cargo test -p cockpit-protocol`, `cargo test -p cockpit-host`.
- `bun run test -- src/client src/app/context/CommentDrafts`.
- Browser smoke on a disposable fixture:
  - Files viewer: write a comment draft, detach it, reattach it, send it;
  - Browser pane: draw an annotation, capture it, send feedback;
  - open the Review leaf.

## Doc notes for WS-25
`CONTEXT.md` ~272/274, `DECISIONS.md` ~81/84.
