# WS-17 Core browser module split

Wave 2 · Size M · Depends on: WS-06 · Blocks: WS-22

## Goal
The core browser service is split into process supervision, CDP, receipts and the service facade. Comments no longer depend on the browser feature.

## Owns
- `crates/cockpit-core/src/browser.rs` → `browser/{mod,process,cdp,receipts,service}.rs`
- `browser/delivery.rs`
- `browser_feedback.rs` (split if over 1,500 lines)
- the import line in `comments/paste.rs`

## Evidence
- `impl BrowserService` is ~1,180 lines (`browser.rs:~174-1352`). It mixes the daemon, CDP, PID and `/proc` stat checks, and Playwright resolution.
- `send_feedback` is 191 lines (`delivery.rs:~54`).
- `comments/paste.rs:~1-6` imports `browser::BrowserHerdrAdapter`.

## Change
1. Move the code by concern and keep the `BrowserService` facade.
2. Split `send_feedback` into validate / render / deliver / record.
3. Move the Herdr adapter trait used by paste into a neutral module (e.g. a shared herdr-adapter module in core). Paste and browser then both depend on it.

## Keep
- Receipts and their formats.
- Owner-only startup reset.
- Discard-on-close.
- CDP identity checks.
- The feedback feature as-is.

## Acceptance
- No function over 150 lines in `browser/`.
- `comments/` has no `browser::` imports.

## Verify
- `cargo test -p cockpit-core browser:: comments::`.
- Browser smoke (`skill://cockpit-browser-smoke-on-disposable-fixture`): open, navigate, annotate, send feedback, close, restart the owner (cleanup).
- One native run.
