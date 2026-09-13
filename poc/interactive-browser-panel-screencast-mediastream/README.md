# Interactive Browser Panel MediaStream POC

This sibling POC proves a real browser capture path: a Playwright `addInitScript` bootstrap injects the same trusted `getDisplayMedia()` control into the local fixture and every top-level HTTP(S) document. A trusted Playwright click captures the current browser tab, encodes that stream as realtime VP8 with `VideoEncoder`, and sends bounded binary packets through an authenticated loopback WebSocket. The Tauri WebKit frontend configures `VideoDecoder`, draws each `VideoFrame` to a canvas, and closes it.

## Run

```sh
bun run setup
bun run dev
# or, in a Wayland desktop session:
bun run dev:wayland
bun run smoke
```

`BROWSER_BINARY=/path/to/chrome bun run dev` overrides the browser executable. Otherwise Playwright uses its deterministic bundled Chrome for Testing. The helper deliberately launches Playwright with `headless: false` plus explicit `--headless=new`: Playwright's `headless: true` boolean emits legacy `--headless`, which does not exercise this verified capture route. `--headless=new` is still headless; it does not open a visible browser window.

The bootstrap's fixed **Start MediaStream capture** control owns the display track. Its capture request uses `displaySurface: 'browser'`, `selfBrowserSurface: 'include'`, and `preferCurrentTab: true`, so it intentionally captures the current tab whether that document is the local fixture or an arbitrary HTTPS target. Chromium is launched with `--use-fake-ui-for-media-stream` and `--allow-http-screen-capture` for this test-only local route; no desktop-capture-source chooser override is needed. To let an HTTPS target connect to the authenticated loopback ingress, this POC also disables `LocalNetworkAccessChecks` and `LocalNetworkAccessChecksWebSockets`; that exception is strictly test-only and is not suitable for a general browsing profile. This does not use `chrome.tabCapture` or an extension: the MV3 `activeTab` invocation route is unavailable here.

After every top-level reload or HTTP(S) navigation, the helper waits for `DOMContentLoaded`, clicks the injected control, and waits for a fresh VP8 keyframe before returning the new snapshot. Page-owned producers restart packet numbering, so the native decoder treats a lower-sequence keyframe as a new stream and resets safely before rendering it.

VP8 is selected by `VideoEncoder.isConfigSupported()` from realtime candidates. The current probe rejected `prefer-hardware`, so the verified route falls back to `no-preference`; that is not a device-acceleration claim. VP8 has no decoder description in this flow, avoiding an H264 AVCC/decoder-config transport requirement.

Packets have a fixed 48-byte `IPWC` header and bounded VP8 payload. The helper accepts producer data only on token-authenticated `/ingress`, fans out to token-authenticated `/frames`, keeps one in-flight packet per native peer, and drops unsafe delta replacement until it requests a source keyframe. There is no JPEG, CDP screencast, WebRTC, or synthetic-frame fallback. If the native WebKit runtime lacks `VideoDecoder`, the frontend self-test reports that explicit failure; its navigation check requires a newly rendered frame rather than relying on packet sequence monotonicity.
