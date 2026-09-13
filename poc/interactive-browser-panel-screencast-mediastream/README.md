# Interactive Browser Panel MediaStream POC

This sibling POC proves a real browser capture path: the local fixture receives a trusted Playwright click, calls `getDisplayMedia()` to capture its current browser tab, encodes that stream as realtime VP8 with `VideoEncoder`, and sends bounded binary packets through an authenticated loopback WebSocket. The Tauri WebKit frontend configures `VideoDecoder`, draws each `VideoFrame` to a canvas, and closes it.

## Run

```sh
bun run setup
bun run dev
# or, in a Wayland desktop session:
bun run dev:wayland
bun run smoke
```

`BROWSER_BINARY=/path/to/chrome bun run dev` overrides the browser executable. Otherwise Playwright uses its deterministic bundled Chrome for Testing. The helper deliberately launches Playwright with `headless: false` plus explicit `--headless=new`: Playwright's `headless: true` boolean emits legacy `--headless`, which does not exercise this verified capture route. `--headless=new` is still headless; it does not open a visible browser window.

The local tab is named **Interactive MediaStream Fixture** and the fixture itself owns the display track. Its capture request uses `displaySurface: 'browser'`, `selfBrowserSurface: 'include'`, and `preferCurrentTab: true`, so it intentionally captures the current fixture tab. Chromium is launched with `--use-fake-ui-for-media-stream` and `--allow-http-screen-capture` for this test-only local route; no desktop-capture-source chooser override is needed. This does not use `chrome.tabCapture` or an extension: the MV3 `activeTab` invocation route is unavailable here.

Because the stream is owned by the captured page, top-level navigation to an arbitrary page would require a new trusted capture gesture and is not claimed by this local fixture POC. The helper does reacquire the stream after fixture reload.

VP8 is selected by `VideoEncoder.isConfigSupported()` from realtime candidates, trying `prefer-hardware` opportunistically and then `no-preference`; no hardware acceleration claim is made. Branded Chrome may expose proprietary codec support, but this POC intentionally needs none of it. VP8 has no decoder description in this flow, avoiding an H264 AVCC/decoder-config transport requirement.

Packets have a fixed 48-byte `IPWC` header and bounded VP8 payload. The helper accepts producer data only on token-authenticated `/ingress`, fans out to token-authenticated `/frames`, keeps one in-flight packet per native peer, and drops unsafe delta replacement until it requests a source keyframe. There is no JPEG, CDP screencast, WebRTC, or synthetic-frame fallback. If the native WebKit runtime lacks `VideoDecoder`, the frontend self-test reports that explicit failure.
