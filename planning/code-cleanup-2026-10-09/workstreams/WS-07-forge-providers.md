# WS-07 Forge provider consolidation

Wave 1 · Size M · Depends on: – · Blocks: –

## Goal
The GitHub, GitLab and Gitea (Tea) adapters share one CLI runner, one byte budget and one fetch skeleton. Each adapter supplies only its argv, its JSON mapping and its stderr classification.

## Owns
- `crates/cockpit-providers/src/{gitlab,tea,github}.rs`
- a new `crates/cockpit-providers/src/forge/` (or `forge_cli.rs`)
- their tests

## Evidence
- `command()` is implemented three times (`gitlab.rs:~85`, `tea.rs:~84`, `github.rs:~77`).
- `COMMENTS_PER_PAGE`/`MAX_COMMENT_PAGES` are declared three times (`tea.rs` uses `u32`, the others `usize`).
- `ByteBudget` exists only in `gitlab.rs:~1079`.
- The fetch functions have the same shape: `gitlab.rs` `fetch_review_inner` (181 lines), `tea.rs` `fetch_review` (227), `github.rs` `fetch` (156).

## Change
1. Add a `CliRunner`: argv, env, timeout, output cap, and a stderr → error-code classification hook supplied per forge.
2. Share `ByteBudget` and the pagination constants.
3. Add a fetch skeleton: resolve identity → fetch item → page comments under the budget → verify returned identity → assemble the `SourceAsset`. The adapters implement a small trait for the per-forge steps.
4. Split the long fetch functions accordingly.

## Keep
- Error codes and messages.
- Output `SourceAsset` bytes and fields.
- Current cap values.
- Per-instance authority checks.
- Tea login selection.

## Acceptance
- One runner and one budget type.
- Each `fetch*` function is under 80 lines.
- Existing provider tests pass unchanged, apart from moved helpers.

## Verify
- `cargo test -p cockpit-providers`.
- Real Tea against a localhost fixture with a fake login (`CODE_GUIDE.md` provider table).
- If `gh`/`glab` providers are configured, run one live import each into a disposable Library through the browser Add dialog. Otherwise report them as unverified live.
