# WS-02 Shared `ProjectConfiguration` test builder

Wave 0 · Size S · Depends on: – · Blocks: WS-03, WS-04, WS-05, WS-06

## Goal
Adding or removing a configuration field touches one test helper, not ~22 test modules. WS-03 removes `companion_root` and WS-12 removes `origins`; both become one-line changes.

## Owns
- A new test-support helper in the crate that defines `ProjectConfiguration` (`crates/cockpit-protocol/src/projects.rs`). Expose it behind `cfg(any(test, feature = "test-support"))`, with the feature enabled through `[dev-dependencies]` in the consuming crates.
- Only the fixture literal at each construction site. Refresh the list with a search for `ProjectConfiguration {`:
  - `cockpit-core`: `context.rs`, `credentials.rs`, `library.rs`, `projects.rs`, `repositories.rs`, `repository_cache.rs`, `review.rs`, `sources.rs`, `sources/references.rs`, `orchestration/{dispatch.rs,retire_tests.rs,service_tests.rs}`;
  - `cockpit-host`: `cli_orchestration.rs`, `tests/server.rs`;
  - `cockpit-providers`: `github.rs`, `lib.rs`, `jira_attachments/download/tests.rs`, `site_http/tests.rs`, `tests/confluence_http.rs`, `tests/source_contracts.rs`;
  - plus any others the search finds.

## Change
- Add `ProjectConfiguration::for_tests(root: &Path) -> Self` (or the equivalent free function), which derives every root from one temp directory.
- Each site uses `for_tests(root)` with `..` struct update for only the fields it actually varies.

## Keep
- Every test's semantics. A test that deliberately sets an unusual root still sets it explicitly.
- No production code changes.

## Acceptance
- The full 17-field literal exists exactly once.
- `cargo test --workspace --exclude cockpit-tauri` passes, with the same test count as before.

## Verify
The command above. No runtime smoke is needed (test-only change).
