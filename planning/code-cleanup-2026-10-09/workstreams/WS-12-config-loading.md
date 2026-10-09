# WS-12 Config loading

Wave 2 · Size M · Depends on: WS-03 · Blocks: WS-20

## Goal
The config file is parsed once. Roots and limits are resolved from tables instead of hand-repeated code. The unused `origins` provenance map is removed.

## Owns
- `crates/cockpit-core/src/config.rs` (split into `config/` modules if useful)
- the `origins` field in `crates/cockpit-protocol/src/projects.rs`
- `src/client/projectProtocol.ts` (+ test)
- the WS-02 test builder

## Evidence
- `load_project_configuration` is ~360 lines (`~190-553`). It repeats `choose_path`/`validate_paths`/`origins.insert` for ~8 roots.
- `load_browser_configuration`, `load_quota_configuration`, `load_library_sync_configuration`, `load_window_configuration` and `load_file_configuration` each re-read and re-parse the TOML.
- Each limit is written out three times (parse, default, origin) at `~370-440`.
- The `origins` map has ~50 keys (`~194-520`). It travels through the DTO and `projectProtocol.ts:~47`, and only test fixtures read it.

## Change
1. Read and parse the TOML once into one raw struct (`deny_unknown_fields` per section). Each `load_*` becomes a resolver over that struct.
2. Resolve the roots from a table: `(name, toml key, env var, default)`, with one validation and overlap pass.
3. Drive both the limits parsing and the struct from a `(name, default, min, max)` table.
4. Delete `origins` end to end, then regenerate TS.

## Coordinate
WS-14 runs in the same wave and must not edit `config.rs`. It takes the `BackgroundPolicy` defaults from the existing `LibrarySyncConfiguration` defaults. Keep that API stable, or message WS-14 if you rename it.

## Keep
- Every error code and message.
- Defaults.
- Env-over-TOML precedence.
- Overlap rules.
- The `[library_sync]` keys (out of scope).

## Acceptance
- One TOML read per load.
- `config.rs` production code is under 900 lines.
- No `origins` field anywhere.

## Verify
- `cargo test -p cockpit-core config::`.
- Start `cockpit serve` with the owner's config and with the fixture-helper config.
- Run one native startup (`skill://cockpit-rebuild-startup-config-cutover`).
