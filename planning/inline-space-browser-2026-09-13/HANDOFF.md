# Inline browser handoff

2026-09-13. The user-authorized no-migration inline cutover is implemented. One focused pre-repair verification pass and one post-repair verification pass completed; the broad A01–A25 matrix is not claimed.

## Scope

Implement the inline replacement using the existing CLI-managed Chromium/profile and raw JPEG screencast. **No legacy migration.** Remove the old extension/external-window runtime. Keep the selected Herdr layout authoritative and preserve saved feedback readability.

## Implemented

- Core browser ownership, stable target identity, inline lifecycle, revisioned drafts, tombstones, prepared captures, exact-PNG retry/recovery, feedback lookup/acknowledgement, and paste receipts.
- A supervised Node helper attached to the existing Playwright CLI Chromium/profile. It provides typed browsing/input/inspection/capture preparation, shared-view lifecycle, target rebinding, metadata barriers, bounded binary JPEG transport, and cleanup on hide/close.
- Web and native client adapters share the versioned browser-view contract. The web gateway and Tauri commands expose metadata/events separately from the bounded binary frame lane.
- `src/app/browser/` provides the inline Space-scoped pane, canvas presenter, browser controls, focus/input ownership, annotations, notes, capture composition, and recovery states.

- Observer forwarding uses bounded operation-aware response deadlines (75s browser actions, 45s view opens, 30s view commands) while preserving uncertain-outcome errors; observer event relays retain their owner socket until the browser stream closes.
- Annotation controls use compact icon-only SVG buttons aligned with the legacy bar; labels remain available through tooltips and ARIA, while Notes retains its visible count.
- Old extension assets, pairing/annotation runtime, extension-only server paths, and external-window presentation were removed. Historical planning/evidence remains; `extension_adapter.rs` remains for unrelated supported Herdr extension detection.

## Verification

Static and focused checks passed:

- TypeScript generation: `cargo run -q -p cockpit-protocol --bin export-typescript -- --write src/protocol/generated/v1.ts`.
- Host build: `cargo build -p cockpit-host --bin cockpit`.
- Native adapter compile: `cargo check -p cockpit-tauri`.
- Frontend/test checks: `bun run build`; `bun run test -- src/client/client.test.ts src/app/App.integration.test.tsx` (45 tests).
- Helper syntax: `node --check browser-runtime/browser-helper.mjs`.

The disposable web/gateway pass used `/tmp/cpol-3m_tp_na` and verified, in order: opening the Space browser, a real 800×600 JPEG canvas, region annotation persistence (`Notes 1` and one overlay), Hide without closing the pane resource, Show with a fresh live frame, and URL navigation to the fixture page. The final screenshot showed the fixture page in the inline canvas. The isolated Linux Tauri binary smoke used `/tmp/cpol-native-inline`; the process stayed alive for 17.5 seconds under isolated configuration and was then stopped by the harness.

The follow-up repair pass verified the reported slow-owner path and the observer stream lifecycle:

- A delayed-owner smoke used a disposable Playwright wrapper that delayed `cookie-get` by 12 seconds. Through the observer, browser action returned HTTP 200 in 14.49 seconds and inline view open returned HTTP 200 in 29.68 seconds instead of the former 10-second observer deadline.
- The repaired observer UI opened a live `800×600` canvas, remained **Live browser view** after metadata and frame WebSockets connected, and returned to the same state after Hide/Show.
- The toolbar showed nine icon-only action controls plus five color swatches. Tool selection and color selection changed state without visible text labels; the only visible toolbar text was the Notes count.

## Limits and follow-up

- This record does not claim A01–A25, the full security/performance matrix, native WebKit input/image-decode parity, or five-minute/reconnect/performance evidence.
- A23 and all legacy draft/pending-capture migration work are explicitly excluded by the user's cutover instruction. Existing saved feedback remains readable; no legacy profile was mutated.
- Browser and native runtime effects were limited to the run-owned disposable fixtures above. The protected default Herdr session, personal browser, unrelated POCs, and pre-existing work were not used.
