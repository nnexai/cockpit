# Browser surface POC comparison

Status: local evidence note from the current Wayland session. This records user-perceived behavior from the comparison run; it is not a cross-platform benchmark.

## Results

| Surface | Observed result | Current role |
| --- | --- | --- |
| Embedded WebView | Best-feeling interaction, but least compatible. This POC has no CDP surface and remote pages loaded through its iframe boundary were rejected by sites such as YouTube and Google. | Use for local or Cockpit/agent-generated HTML previews. |
| Screenshot-based CDP | Image quality is acceptable, but `Page.captureScreenshot` polling is very slow and the surface feels non-interactive. | Deprioritize for interactive browsing. |
| CDP `Page.startScreencast` | Performance is comparable to CEF. Video is lightly choppy, but input feels snappy. Fullscreen YouTube was somewhat zoomed/cut off. | Current broad-web candidate because installation and multi-platform support are simpler. |
| CEF OSR/CDP | Performance is comparable to interactive screencast. Video is lightly choppy, input feels snappy, fullscreen YouTube aligned with the pane, and this was the only surface that transmitted sound in the comparison. | Compatibility/media candidate when CEF's heavier runtime is justified. |

## Current direction

- Use embedded WebView for local, generated, or otherwise controlled HTML preview content.
- Prefer the CDP screencast path for the general interactive web surface. Its current behavior is good enough; frontend polish can remain optional rather than blocking.
- Keep CEF as the higher-compatibility option, especially for fullscreen alignment and audio, but do not make it the default solely from this run because its installation/runtime footprint is heavier.
- Treat screenshot polling as a feasibility fallback, not the interactive default.

## Open CEF observation

CEF opened several black windows during the comparison. Their source is unexplained. This remains a runtime investigation item if CEF becomes the selected default; no cause is inferred here.

## Evidence boundary

The compared implementations are:

- `poc/embedded-webview-panel`
- `poc/interactive-browser-panel` (screenshot-based CDP)
- `poc/interactive-browser-panel-screencast` (CDP screencast)
- `poc/cef-osr-panel` (CEF OSR with CDP)

The run was performed on Wayland with the Tauri windows forced to Wayland. Compatibility, audio, fullscreen behavior, and performance on other operating systems remain unverified.
