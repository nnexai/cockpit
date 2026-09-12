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
## Transport variant comparison

The transport variants keep the same Chromium CDP `Page.startScreencast` capture path. The difference is how JPEG bytes cross the helper/WebView boundary:

| Variant | Direct observation | Decision |
| --- | --- | --- |
| Baseline screencast | Base64 JPEG data URLs cross the helper/Tauri JSON event path. | Retain as the compatibility baseline only. |
| Binary screencast (`poc/interactive-browser-panel-screencast-transport`) | Raw JPEG bytes and geometry cross a loopback binary WebSocket with latest-only delivery. Chromium capture CPU remains unchanged, but the large JSON/data-URL hop is removed. | Best practical local transport. |
| MJPEG variant (`poc/interactive-browser-panel-screencast-mjpeg`) | Native Tauri/Wayland self-test passed. WebKitGTK rejected `fetch()` responses declared as `multipart/x-mixed-replace`, so the POC carries the same multipart bytes as bounded `application/octet-stream` with an exposed boundary header. | Compatibility fallback, not the performance default. |

One side-by-side native resource sample over approximately 50 seconds measured:

| Metric | Baseline | Binary |
| --- | ---: | ---: |
| Aggregate PSS | 1046.5 MiB | 916.1 MiB |
| Aggregate RSS | 1832.1 MiB | 1708.3 MiB |
| CPU, normalized to one core | 27.14% | 26.18% |

The memory difference favored binary in that run; the CPU difference is too small to treat as a capture improvement. A separate later idle sample of MJPEG versus binary measured 401.0 versus 396.7 MiB PSS and 1058.2 versus 1049.8 MiB RSS, with no sampled CPU ticks for either process tree. Those absolute samples used different process runs and are not comparable to the preceding totals. Static idle CPU is not a useful transport benchmark.

Direct CDP capture probes establish the workload boundary:

- Static, transparent-animation, and offscreen-animation pages produced one callback over five seconds.
- A visible moving square produced 300 callbacks over five seconds, with 300 unique JPEG hashes and approximately 16.6 ms mean spacing.
- Uniform synthetic scrolling produced approximately 120 callbacks over five seconds while decoded pixels stayed identical; geometry/scroll metadata still changed.
- A 60-second uniform-scroll run kept Chromium around 31.9–32.4% of one core. Per-frame pixel decoding added approximately 9.1% of one core in the decoder process without lowering Chromium cost.

Conclusion: binary transport is better than baseline downstream, not at reducing Chromium capture work. MJPEG is useful when multipart framing is preferable, but its WebKit compatibility priming and JavaScript parser make it less efficient locally. Direct embedded rendering or native OSR/shared textures remains the higher-efficiency architecture.

The transport POCs were committed as `3a37380` (binary) and `a9ae1fe` (MJPEG). Their helper, CDP, and native Wayland interaction checks passed in the current Fedora Wayland session.

### Native animated workload

For a direct comparison, all three real Tauri POCs were launched concurrently on Wayland with `POC_RESOURCE_TEST=1`. That opt-in mode loads the same fixture with a fixed 24×24 red square using a two-second alternating CSS `translate3d` animation. The default fixture remains unchanged. After a 15-second warmup, aggregate descendant process trees were sampled for 10 seconds from `/proc`; CPU is normalized to one core from process-tree CPU ticks. This is one workstation run, not a cross-platform benchmark.

| Variant | Processes | Aggregate PSS at sample end | Aggregate RSS at sample end | CPU, normalized to one core |
| --- | ---: | ---: | ---: | ---: |
| Baseline screencast | 10 | 743.7 MiB | 1423.0 MiB | 334.8% |
| Binary screencast | 11 | 426.2 MiB | 1112.0 MiB | 314.4% |
| MJPEG screencast | 11 | 457.3 MiB | 1144.7 MiB | 300.9% |

The animated run reinforces the transport conclusion: binary used approximately 317.5 MiB less PSS and 311.0 MiB less RSS than baseline at the sample endpoint, while MJPEG used approximately 286.4 MiB less PSS and 278.3 MiB less RSS. Baseline memory also grew by approximately 74.9 MiB PSS during the 10-second sample; binary and MJPEG stayed within normal sampling fluctuation. CPU remained dominated by Chromium capture/JPEG work; binary was not a capture-cost reduction, but had the lowest downstream memory of the two transport variants. The figures are directional because this is one concurrent run with no frame-count telemetry exposed by the native shells.

## Open CEF observation

CEF opened several black windows during the comparison. Their source is unexplained. This remains a runtime investigation item if CEF becomes the selected default; no cause is inferred here.

## Evidence boundary

The compared implementations are:

- `poc/embedded-webview-panel`
- `poc/interactive-browser-panel` (screenshot-based CDP)
- `poc/interactive-browser-panel-screencast` (CDP screencast)
- `poc/cef-osr-panel` (CEF OSR with CDP)

The run was performed on Wayland with the Tauri windows forced to Wayland. Compatibility, audio, fullscreen behavior, and performance on other operating systems remain unverified.
