# WS-22 Host transport consolidation (axum + Tauri)

Wave 3 · Size L · Depends on: WS-17 · Blocks: –
Native changes: one native run is mandatory. Work in the phases below and run the affected checks after each phase.

## Goal
The browser host (`cockpit serve`) and the native shell share one operation registry, one error mapping, one session-stream state machine and one browser-view relay. Tauri keeps only window/webview concerns.

## Owns
- `crates/cockpit-host/src/server.rs`, `server/*`, `browser_view.rs`
- `src-tauri/src/*`
- new shared modules in `cockpit-host`

## Evidence
- `src-tauri/src/lib.rs` (2,088 lines) has ~60 `#[tauri::command]` wrappers (`~180-1806`, `generate_handler` at `~1964-2063`).
- `server.rs` has ~30 routes plus 11 merges (`~183-249`).
- The error mapping differs: `inspection_error` vs `inspection_error_response`.
- The session stream is implemented twice: `relay_session` (`lib.rs:~1181-1336`) and `run_session_socket` (`server.rs:~717-922`, plus `advance_sequence`).
- The browser-view relay sits in Tauri (`run_browser_view_socket`, 231 lines, `~865-1096`), and so does `widget_request_size` (`~1709-1726`).
- `router()` is an alias of `build_router()` (`server.rs:~266-273`), and the browser-runtime guard is repeated in every handler (`~328-431`).
- Shutdown handling exists twice.

## Change (phases)
1. Remove the `router()` alias. Add one browser-runtime extractor/guard.
2. Add a pure session-stream state machine (snapshot, stale, disconnected, generation/sequence, reconnect) that emits frames. axum and Tauri only pump the frames.
3. Move the browser-view relay and request-size limit into `cockpit-host`. Tauri calls it.
4. Add one operation registry (a macro or table): `name, request type, response type, service call`. It generates the axum routes and Tauri commands, with one error mapping.
5. Share the shutdown handling.

## Keep
- Route paths, Tauri command names, DTOs and error codes: **no wire change**. WS-21 relies on these names staying unchanged, which is why the two workstreams can run in parallel.
- Loopback-only serving.
- Tauri capabilities.
- `dragDropEnabled: false` and the GTK app-paintable flag.

## Acceptance
- `src-tauri/src/lib.rs` is under 700 lines.
- No business logic in Tauri handlers.
- One error-mapping function.
- One session-stream implementation.

## Verify
- `cargo test -p cockpit-host`, `cargo test -p cockpit-tauri`, `cargo check -p cockpit-tauri`.
- Browser smoke and native smoke (`skill://cockpit-native-smoke-and-limited-worker-fallback`): startup, session switch, Herdr restart → stale/disconnected → recovery, terminal I/O, browser view, widget open/select.
