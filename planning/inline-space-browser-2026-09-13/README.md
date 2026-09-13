# Inline Space browser replacement

Date: 2026-09-13. Status: implementation complete for the user-authorized no-migration cutover; focused verification passed. The broad A01–A25 matrix is not claimed.

Scope update from the user: implement the complete inline replacement and finish with one focused verification pass. Legacy migration is excluded; remove the old runtime directly. The historical migration tasks and acceptance row A23 below no longer apply. New inline draft persistence, pending-capture retry, and existing saved-feedback readability remain required.

## Outcome

Replace the current external Space browser and in-page annotation extension with an interactive Chromium image surface inside Cockpit. Use the binary JPEG `Page.startScreencast` approach in [the transport POC](../../poc/interactive-browser-panel-screencast-transport/), not screenshot polling, MJPEG, MediaStream/WebCodecs, CEF, or an iframe loading arbitrary sites.

The complete loop is: open the selected Space's browser inline → browse with real pointer/keyboard input → observe page-driven and agent-driven changes → draw/pick/comment over its image → save the image plus structured evidence → let an agent fetch/acknowledge it or explicitly paste feedback without submitting Enter.

This package proposes the implementation contract; it does not change the application's behavior, installed browsers, existing profiles, or repository architecture authorities.

## Read in order

1. [Architecture, pane placement, and bidirectional transport](01-architecture-and-transport.md).
2. [Annotation parity, capture, and durable migration](02-annotations-and-migration.md).
3. [Implementation increments and acceptance gates](03-delivery-and-verification.md).

## Decisions

| Concern | Proposed decision |
| --- | --- |
| Browser engine | Keep a dedicated Chromium profile and named Playwright CLI session per Space. Attach a supervised screencast helper to that same browser; do not create a second browser for the inline view. |
| Pane placement | A Cockpit-owned, Space-scoped resizable split beside the selected tab's **unchanged Herdr layout**. It is not a synthetic Herdr pane or tab. See the explicit authority departure in document 01. |
| Pixels | Raw JPEG binary WebSocket, independently decodable frames, bounded latest-only delivery and decode. No per-frame base64/JSON/Tauri-event image hop after CDP decoding. |
| Browser control | Typed, ordered, bounded input/navigation/resize requests through `CockpitClient` and the owning browser runtime. |
| Browser feedback | A separate ordered metadata stream: tab/document identity, URL, title, navigation/loading/history, viewport, cursor, focus/editability, dialogs, and failures. Metadata must update without local input and without a new JPEG. |
| Annotation surface | SVG/HTML overlays owned by Cockpit above the displayed image. No injected page annotation UI. A narrow CDP DOM inspection/probe is allowed for real element evidence and cursor state. |
| Durable feedback | Retain the current core feedback store, PNG artifacts, stable pending IDs, exact acknowledgement, retention, and paste receipts. Move draft ownership out of extension storage without discarding existing work. |
| Agent automation | Preserve ordinary Playwright CLI discovery and automation of the same browser. Local input arbitration does not claim to lock out direct agent CDP/Playwright operations. |
| Runtime ownership | Keep the existing single owner per browser state root; observers route to it. Hiding a pane never closes the browser. Normal owning-runtime shutdown still closes only proven owned browsers. |

## Intentional changes to existing decisions

The [2026-09-08 browser direction](../../DECISIONS.md#2026-09-08-space-associated-browser-direction) and [original browser plan](../browser-space-integration-2026-09-08.md) select an external browser and extension. This plan replaces those two choices and adds a narrow human-interaction transport; it does **not** introduce a custom agent automation API.

The older no-separate-dock rule for Files/Context/Review remains intact. Those renderers still occupy real Herdr panes. The new browser split is an explicit Cockpit-only presentation resource, with its own focus and hide controls, not a second authority for Herdr layout. This placement is a proposed design choice, not a claim that the user already selected exact docking semantics.

At implementation, update the browser sections of `CONTEXT.md`, `DECISIONS.md`, the original browser plan/handoff status, `CODE_GUIDE.md`, and relevant existing usage documentation to identify the cutover. Keep this planning task confined to its new folder. `NEXT_PHASE_PLAN.md` is completed history; the broader next-level timeline and the latest [polish execution record](../product-atlas-2026-09-12/polish/execution.md) are background, not instructions to restart historical bootstrap work.

## Source baseline and confidence

Source inspection and implementation verification were performed for this plan. Static checks, a focused disposable browser pass, and a Linux Tauri startup smoke passed on 2026-09-13; the broader security, performance, and acceptance matrices remain unrun.

| Source | Grounded baseline |
| --- | --- |
| [POC helper](../../poc/interactive-browser-panel-screencast-transport/helper/browser-helper.mjs), `startFrameStream`, `processScreencastFrame`, `dispatchInput` | Token-authenticated loopback JPEG WebSocket; 48-byte IPBF v1 header; CDP and viewer ACKs; one pending latest frame; mouse/wheel/key input; cursor returned in command responses. Single fixed-viewport page. |
| [POC frontend](../../poc/interactive-browser-panel-screencast-transport/dist/app.js), `scheduleFrameRender`, `applySnapshot` | Binary parsing and Blob image rendering, coalesced motion and serialized input. ACK precedes decode; no continuous metadata subscription or production navigation identity. |
| [POC native bridge](../../poc/interactive-browser-panel-screencast-transport/src-tauri/src/main.rs) | Tauri commands supervise a helper using synchronous JSON stdio. Useful process-boundary evidence, not the production async lifecycle design. |
| [POC results](../../research/browser-rendering-poc-results.md) | Recorded user interaction favors binary screencast; it removes the large JSON hop, not Chromium's JPEG capture cost. Fullscreen alignment has a known gap. Audio was observed only in CEF in the comparison. This file had pre-existing edits and was left untouched. |
| [Browser service](../../crates/cockpit-core/src/browser.rs), [owner runtime](../../crates/cockpit-host/src/browser_runtime.rs) | Fresh Space identity, named CLI session/profile, ownership receipts, observer routing, scoped close/reconcile, and the inline browser stream. |
| [Browser protocol](../../crates/cockpit-protocol/src/browser.rs), [feedback schema](../../crates/cockpit-protocol/src/browser_feedback.rs) | Existing lifecycle/feedback contracts; annotation kinds are Freehand, Element, Region. |
| [Historical browser plan](../browser-space-integration-2026-09-08.md) | Earlier external-window and extension behavior, retained as historical context only; it is not a supported runtime path. |
| [Feedback store](../../crates/cockpit-core/src/browser_feedback.rs), [delivery](../../crates/cockpit-core/src/browser/delivery.rs) | Durable PNG/evidence, pending-ID acknowledgement, bounded paste-only delivery, uncertain-outcome receipts. |
| [App](../../src/app/App.tsx), `FeedbackPanel` and browser actions | Commands/Space entrypoints, inline browser renderer, and feedback overview. |
| [Browser handoff](../browser-space-handoff-2026-09-08.md) | Recorded real capture/fetch/ack/paste evidence and explicitly incomplete acceptance. Later source and polish corrections override its historical open findings. |

## Required versus outside this replacement

**Required:** real bidirectional input and metadata, bounded JPEG transport, inline navigation/tab handling, resize/focus/recovery, complete existing annotation parity, lossless durable-work migration, preserved agent workflow, and browser plus Linux-native proof.

**Not silently promised by JPEG streaming:** audio transport, DRM/media compatibility, full browser chrome, OS-native menus/pickers, remote access, or complete accessibility-tree navigation. JavaScript dialogs, file upload/download handling, and clipboard need explicit bridges or visible supported/unsupported states as specified in the delivery gate; they cannot remain invisible browser blockers. Ordinary desktop pointer and keyboard interaction, Unicode/IME, cursor updates, URL changes, and all existing annotation tools are not optional.

Audio/video transport alternatives, full accessibility remoting, touch/pen pressure/pinch input, generalized docking, arbitrary browser extensions, and an agent locking framework are not selected here. Missing audio and any unsupported native-browser facility must be visible before launch/cutover; do not describe the replacement as complete desktop-browser feature parity.

## Finish line

Focused completion evidence: TypeScript generation, host build, Tauri check, frontend typecheck/build, helper syntax check, and a disposable gateway/browser pass all passed. The browser pass opened a real frame, persisted a region annotation, hid and restored the view, and navigated to a fixture page; a Linux Tauri startup smoke stayed alive under isolated configuration. This does not claim A01–A25, the full security/performance matrix, or native input/decode parity. User-directed legacy draft migration is excluded; existing saved feedback remains readable.
