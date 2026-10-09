# WS-16 `review.rs` split

Wave 2 · Size M · Depends on: WS-06 · Blocks: –

## Goal
Review is organized by concern: snapshot, git plumbing, source reading, cache and diff parsing. Worktree path safety lives in one module.

## Owns
`crates/cockpit-core/src/review.rs` → `review/{mod,snapshot,git,source,cache,parse,safe_fs}.rs`.

## Evidence
`review.rs` is 3,914 lines. Its sections:
- `snapshot`: `~137-260`;
- `collect`: `~823-893`;
- `diff`: `~956-1122`;
- `git_source_page`: `~1226-1378`;
- `read_worktree_source_page`: `~1974-2063`;
- `parse_unified`: `~2295-2367`;
- caches: `~1598-1720`;
- `cap_std` path-safety helpers: `~1887-2128`.

## Change
Move the code by concern. `ReviewService` stays the facade. Split `diff` and `git_source_page` into steps under 80 lines each. Move the tests next to their module.

## Keep
- Snapshot identity.
- Cache names and validation.
- Token streaming.
- Index/working-file non-mutation.

## Acceptance
- No file in `review/` over 900 lines.
- No function over 150 lines.
- The existing tests pass.

## Verify
- `cargo test -p cockpit-core review::`.
- Browser smoke: open the Review leaf on a disposable fixture repo with staged, unstaged and untracked changes plus a large file. Confirm the index and working files are unchanged afterwards.
