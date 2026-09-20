# WEB-03 — Keep resized browser frames sharp and aligned

## Outcome

Ensure every controller-owned resize produces capture pixels at the accepted dimensions and a matching geometry barrier before location-sensitive input or annotation. A resized inline browser stays crisp instead of stretching an old screencast; zoom, device pixel ratio, page zoom, scroll, sticky/fixed content, and letterboxing use one frame-bound transform. A resize/control race cannot leave a pending request stranded when ownership changes.

This task re-verifies and hardens issue #8's prior resolution against the current baseline; the historical patch and handoff are not acceptance proof.

## Evidence and starting points

- Baseline: `6f6222b74e4f552ce697e61364cf653f4b6be29f`; issue #8 records a prior fix but broad A11/native geometry evidence remains unclaimed.
- Issue #8: https://github.com/nnexai/cockpit/issues/8.
- Read `planning/inline-space-browser-2026-09-13/01-architecture-and-transport.md` identity, JPEG/geometry, and resize sections plus delivery acceptance A08, A10, A11, A16, and A17.
- Source starts: `browser-runtime/browser-helper.mjs` `applyRequestedViewport`, `bindPage`, `resetFrameTransport`, `startScreencast`, `captureCurrentFrame`, and frame envelope generation (around 239, 447, 495, 654, 809, 824).
- Source starts: `src/app/browser/BrowserPane.tsx` viewport reconciliation, `frameSupportsViewportInput`, stream identity, and resize callbacks.
- Source starts: `src/app/browser/framePresenter.ts` `parseBrowserViewFrame`, `validateFrameDescriptor`, `FramePresenter`, and decode/publish barrier; `src/app/browser/transform.ts` is the shared coordinate transform.
- Use `planning/stability-and-gitlab-2026-09-20/tasks.json` for `browser-ui`, `browser-helper`, and `frame-presenter` serialization.

## Changes

1. Treat resize as a controller-owned, coalesced, revisioned transition. Restart/rebind screencast capture with accepted content dimensions; reset transport before publishing a new viewport descriptor.
2. Reconcile pending resize when control status changes, including observer-to-controller takeover and reconnect. Apply a new controller size once; observers scale and do not fight viewport authority.
3. Require a presented frame matching stream epoch, target/document generation, viewport revision, descriptor geometry, and dimensions before enabling pointer, wheel, inspect, annotation, or capture.
4. Keep the last frame visible with stale/input-blocked state while waiting; reject inconclusive geometry instead of guessing from receive order, wall clock, DPR, or CSS canvas size.
5. Make `transform.ts`, frame presenter, pointer mapping, element evidence, and PNG composition consume the same frame-bound coordinate spaces. Keep host zoom, page zoom, visual viewport offsets, scroll, and device pixel ratio distinct.
6. Add a required geometry/capture review covering resize races, matching-frame barriers, zoom/DPR, and native WebKit scaling semantics.

## Non-goals

- No new transport architecture, screenshot polling loop, or independent per-feature transform formulas.
- No claim that intrinsic JPEG dimensions alone prove alignment or crispness.
- No broad annotation/delivery reliability work; WEB-05 owns capture/delivery behavior.
- No server exception allowing stale geometry input or observer resize authority.

## Acceptance

1. Browser gateway resize fixture: resize from at least 800x600 to a materially different panel size; subsequent JPEG intrinsic dimensions and descriptor viewport revision match the accepted request without stretched old pixels.
2. Linux-native Tauri: the displayed frame intrinsic dimensions, CSS painted rectangle, and descriptor geometry remain aligned after panel resize and window/device-scale changes.
3. Rapid resize burst during takeover/reconnect eventually applies the latest accepted revision, never paints a stale frame as current, and does not strand a pending resize.
4. At controlled host zoom/DPR, page zoom, scroll, letterbox, fixed/sticky elements, and narrow presentation, a known hit target and exported annotation agree within two displayed CSS pixels; out-of-image presses are refused.
5. Two clients at different sizes observe the same target without observer resize fights; explicit takeover resizes once and waits for the matching frame before input.
6. Navigation, same-URL reload, target switch, and document change invalidate old geometry; delayed frames or metadata cannot enable input or move an annotation on the new document.
7. Capture-as-shown and annotation composition use the pinned presented frame and report stale/mismatch rather than silently producing misaligned evidence.

8. Record both requested CSS content size and delivered pixel dimensions; do not “fix” blur by multiplying a CSS size by DPR without confirming the helper’s viewport and screencast meanings.
9. Ensure a resize stop/restart cannot ACK a frame from the retired screencast into the new stream epoch. Retired frame bytes may be discarded, but their transport credits still need deterministic release.
10. Exercise a resize while a drag, wheel burst, Element hover, and capture preparation are each in flight. Each path must either finish against the old valid frame before transition or be visibly refused and released.
11. Keep geometry metadata monotonic within a stream and expose enough identity for evidence to correlate descriptor, canvas, and input response without relying on wall-clock ordering.
12. Preserve a usable stale image while waiting, but make the stale state obvious and keep location-sensitive controls disabled until the barrier passes.

The implementation should make one owner responsible for capture-bound dimensions and one shared transform responsible for every consumer. If native and gateway surfaces expose different scale semantics, record the unsupported case instead of silently applying platform-specific offsets.

## Verification

Use uniquely named gateway and Linux Tauri runs, isolated profiles, controlled fixture dimensions, known zoom/DPR values, and recorded frame descriptors. Exercise A08, A11, A16, and A17 plus resize/navigation race negatives. Record accepted viewport revisions, JPEG width/height, frame/metadata identities, painted rect, hit-target coordinate error, and cleanup. The integration owner must inspect real canvas output and native composition; a build, screenshot without descriptor evidence, or historical issue attachment is not sufficient.

The evidence record must include:

- requested content CSS size, accepted viewport revision, JPEG dimensions, frame epoch/sequence, and displayed painted rectangle;
- controlled host zoom/DPR and page zoom values, hit-target coordinates, and measured error;
- resize/takeover/navigation race outcomes, including stale-frame refusal and pending-resize reconciliation;
- separate browser-gateway and Linux-native observations, with profile/helper/frame cleanup.

Do not mark a frame sharp from visual appearance alone; retain the descriptor and geometry values that made input eligible.

Retain the raw descriptor values needed to reproduce the transform calculation, including scroll offsets and visual viewport scale/offset where supported.

The evidence must state which dimensions are CSS viewport, screencast pixels, and painted client pixels; do not collapse them into one “resolution” field.

## Handoff

Return changed paths, transform/barrier decisions, and unresolved platform geometry risks. The integration owner must commit a durable evidence record at `runs/<run-id>/WEB-03.md` and a real implementation commit. Keep all `browser-ui`, `browser-helper`, and `frame-presenter` writes serialized with WEB-02 and later consumers using `../tasks.json`; do not edit task status here.
