# OBS-020 — stabilize browser input and first frame across geometry changes

Date: 2026-09-23. Linux native run at 2× scale. Fractional scaling and macOS were not tested.

## Findings and repair

The DPR 2 native reproduction showed two state transitions behind the reported behavior:

- Chromium input coordinates use host-device scale on initial attachment and again after a main-document loader change; a real emulated viewport resize switches dispatch coordinates to CSS pixels. `browser-helper.mjs` now tracks that mode, scales pointer, wheel, and held-button release coordinates at the CDP boundary, switches to CSS coordinates after an accepted viewport-size change, and restores the DPR scale on a new document.
- A resize restarted the screencast but did not request a frame. With a static page, viewport metadata advanced while the last frame still described the old dimensions until another page event caused paint. Resize now captures a frame immediately after restarting the screencast.

## Native evidence

Run-owned session: `polish-1cbphncj`, root `/tmp/cpol-1cbphncj`. The protected/default Herdr session was untouched. The app reported DPR 2.

- Initial 706×202 CSS view: a helper-routed click at `(85.7, 198.1)` reached the fixture button at the same page coordinates and incremented its counter.
- Expanding the browser pane changed its surface and page viewport to 706×681 CSS. The new frame descriptor arrived at 706×681 without scrolling or reloading. A helper-routed click at the same point again reached the button at `(85.7, 198.1)`.
- After a full document navigation in the same target, the same helper-routed click again reached the fixture button at `(85.7, 198.1)`.
- A helper-routed wheel at `(85.7, 190)` with `delta_y_css: 400` changed page `scrollY` to 400.

Input was issued through Cockpit's native browser command bridge; the fixture URL was loaded with CDP. This run proves the helper's input and capture paths inside the native app, but does not cover a physical divider drag. The installed WebKit driver returned an error for W3C mouse actions and ignored `setWindowRect`.

## Checks and other findings

- `bun run test`: 220 tests passed.
- `bun run build`: TypeScript and production build passed; the existing large-chunk advisory remains.
- `cargo test -p cockpit-host`: 12 tests passed.
- `cargo build -p cockpit-host --bin cockpit -p cockpit-tauri --features tauri/custom-protocol`, `node --check browser-runtime/browser-helper.mjs`, and `git diff --check` passed.
- No separate unrelated product issue was confirmed during this focused browser run. Annotation delivery and retained annotation recovery were deliberately not exercised, per the user's instruction to discard that work.

## Remaining boundary

This closes the tested DPR 2 native pointer, wheel, resize/expand, and full-document navigation path. Fractional DPR and macOS remain untested. The broader stability campaign and its separately tracked findings remain open; this note does not mark campaign-wide acceptance complete.
