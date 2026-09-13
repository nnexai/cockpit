# Architecture and bidirectional transport

Status: proposed implementation contract. [Overview](README.md) · [Annotations](02-annotations-and-migration.md) · [Delivery](03-delivery-and-verification.md).

## 1. Inline placement and authority

Open browser from the existing Commands or selected Space context menu. Reveal a resizable right-hand browser split within the work area, beside the selected Herdr tab's pane layout. Preserve the Space/sidebar and Herdr tab strip. On narrow windows, allow an explicitly selected browser-only presentation with a visible Back to terminals control; do not squeeze terminals beneath usable dimensions or overlay an invisible input catcher.

```text
selected Space / authoritative Herdr tab strip              Pane | Commands
+------------------------------------+----------------------------------+
|                                    | Browser tabs (Chromium targets)  |
| existing Herdr pane layout          | Back Forward Reload URL  Hide   |
| terminals / Files / Review          |----------------------------------|
|                                    | JPEG image + annotation overlay |
|                                    | local status / feedback count   |
+------------------------------------+----------------------------------+
```

This browser pane is a **Cockpit presentation split**, not a Herdr pane ID. Do not create a sleeping PTY, commandeer a terminal renderer, add a fake browser tab to Herdr snapshots, or patch Herdr. Browser tab chips represent Chromium targets and stay inside the browser pane. Herdr pane move/split/close shortcuts never mutate those tabs. General free docking, multiple browser splits per Space, and moving a browser into a different Space are outside this design.

Presentation visibility/width is per Cockpit window and Space association. Browser page/profile lifetime belongs to the owning runtime. Switching Herdr tabs keeps the same Space browser visible; switching Spaces binds to the destination association and detaches the old view. Show/Hide changes presentation only. Close browser is a separate explicit lifecycle action; close a browser tab only from browser-local chrome. No permanent browser-status strip is added to the sidebar.

Focus state is explicit: Herdr semantic focus, terminal writable intent, browser interaction lease, and local DOM focus are separate. Clicking the browser revokes this window's terminal control intent and releases writable terminal attachment while retaining visible observation. It does not fabricate a Herdr focus event. Clicking a terminal follows the existing confirmed Herdr focus/takeover path. A workbench navigation command may intentionally return to Herdr; never leak a browser click/key into a previously selected terminal. Compare that transition with the real Herdr TUI in disposable sessions during implementation.

Reuse current application tokens and chrome, not stale historical pixel specifications. Browser toolbar, annotation tools, notes, feedback overview, and inline recovery use the current compact controls. The overlay is clipped to the image; controls remain reachable at the minimum size and at zoom.

## 2. Runtime topology

```text
Cockpit pane / overlay
  -> CockpitClient: typed tasks + ordered metadata subscription
  -> native commands/channels OR browser HTTP/WebSocket
  -> BrowserRuntime owner (observer hosts forward to the owner)
  -> BrowserService policy + supervised helper control connection
  -> dedicated CDP sessions on the EXISTING Space Chromium

Chromium Page.screencastFrame (base64 at CDP boundary)
  -> helper: decode once, validate, attach frame identity/geometry
  -> private binary frame WebSocket -> client adapter -> image presenter

page/agent/CDP events -> helper -> owner -> ordered metadata -> pane
agent -> ordinary named Playwright CLI session -> the SAME Chromium
```

### Keep and extend

- `BrowserService` remains the owner of Space association, lifecycle authorization, durable receipts and feedback policy.
- `BrowserRuntime` retains the state-root owner lock, private socket, observer forwarding and scoped shutdown. Extend owner forwarding with cancellable metadata subscriptions; a one-shot `WireResponse` is not a streaming implementation.
- Tauri and the browser gateway remain thin adapters. UI components import neither Tauri nor WebSocket/fetch. The `CockpitClient` browser-view adapter hides control, metadata and binary-frame mechanics.
- A small packaged Node helper contains the CDP transport/inspection code adapted from the POC. Supervise it from the owner, not from each native window. Keep its runtime/dependency location configurable and packaged/pinned; do not import from a hard-coded developer installation or use the POC fixture server in production.
- Preserve CLI-managed launch, dedicated profile and named-session invocation. Configure a private loopback CDP endpoint for the owned browser; verify its current process/profile/incarnation before helper attachment. `DevToolsActivePort` may supply the endpoint only after live ownership validation; a stale file is not authority.
- **Feasibility gate:** prove the installed CLI launch configuration supports windowless unified Chromium, persistent profile, loopback CDP attachment and concurrent ordinary CLI operation. Current production launch is external/headed; the POC launches a different browser itself. Neither establishes this combined path. Do not silently switch browser ownership or break agent CLI discovery if the probe fails: resolve the launch seam before implementation depends on it.

### Three channels, one public abstraction

1. **Commands:** finite typed control operations through the existing host adapters. Each carries request and view identity; responses distinguish accepted, rejected, stale, unsupported, and outcome unknown. Acknowledgement of input is not proof of resulting pixels.
2. **Metadata:** reliable ordered events and an atomic initial snapshot through native channels/browser WebSocket. Observers use the same owner-sourced ordering. No polling per mouse move or per video frame to keep URL/title current.
3. **Frames:** helper-owned loopback binary WebSocket, accessible only with an owner-issued view-scoped credential. Both adapters use this transport for JPEG bytes. This is an explicit exception to the native channel default, confined to replaceable high-volume frames; terminal transport is unchanged. No raw CDP forwarding is exposed to UI code.

Direct frame transport preserves the POC's avoided JSON hop. The owner grants/revokes access over its private helper control connection. It must also observe helper failure and close/revoke views. Register the actual Cockpit web origin and native origin; test WebKitGTK, CSP, browser local-network permission behavior, and credential negotiation rather than copying wildcard POC policy. If a same-origin frame relay is required by the supported web host, resolve it in the feasibility gate as a binary-only adapter concern, not a base64 fallback.

Use established bounded WebSocket libraries, not the POC's handwritten RFC framing. Helper stdio control uses one uninterrupted asynchronous reader, request-ID demultiplexing, a bounded unsolicited-event path, bounded stderr, explicit readiness and cancellation. Long navigation, dialogs, or stalled page evaluation must not block pointer release, metadata, or shutdown. Product/authorization rules do not migrate into helper JavaScript.

## 3. Identity and ordering contract

Proposed names below are semantic fields; define final Rust DTOs once and generate TypeScript before parallel consumers edit them.

| Identity | Lifetime and purpose |
| --- | --- |
| Space association | Existing key from fresh endpoint/session/Space authority. Rename preserves it; endpoint replacement/Space reuse requires revalidation. |
| Browser incarnation | New on actual browser replacement/relaunch. Never inferred from URL or profile pathname. |
| Target ID | Stable Chromium target identity while that tab exists. CLI tab indices are transient addressing hints only. |
| Document generation | New on full navigation/reload/frame-document replacement; same-URL reload changes it. Track child-frame generations separately for element evidence. BFCache restoration requires explicit revalidation of document-bound handles. |
| View/stream epoch | New when opening/rebinding a subscription, switching target, or invalidating transport. Late old-socket decode/input/metadata callbacks cannot cross it. |
| Viewport revision | New for resize/emulation/visual-viewport geometry changes affecting the transform. |
| Frame sequence | Monotonic within a stream epoch; gaps are allowed because frames are intentionally dropped. |
| Metadata sequence | Contiguous within an event subscription. Gaps invalidate control and require a fresh snapshot. |
| Input sequence / lease generation | Ordered commands from the one current Cockpit controller; prevents old queued input after loss/rebind. |

A view snapshot contains association/incarnation, target inventory, displayed target, document generation, viewport state, navigation/history/loading, browser status, cursor state, capabilities and control lease status. Subscribe with a snapshot-plus-buffered-events barrier; subscribing after an unrelated snapshot without a barrier loses events. A fresh event baseline is necessary after reconnect, not replaying old UI mutations.

Metadata and pixels use independent delivery lanes. Correlate using epoch, target/document identity and viewport revision, not receive order or wall clocks. Every frame must either contain the needed geometry or reference immutable geometry already received; unknown geometry is bounded pending state, never guessed. A newer document/viewport event invalidates old actionable pixels immediately. Keep the last image with a stale marker, but block location-sensitive interaction until a matching presented frame exists.

CDP does not provide an atomic DOM+pixels snapshot with every screencast callback. Do not merely tag a delayed old frame with whatever document is current on receipt. On navigation/resize transitions invalidate the old capture session, establish a new document/layout baseline, restart/rebind capture and validate the new stream before enabling input. Prove delayed callbacks, redirect chains and same-URL reload behavior in the feasibility gate. If stable geometry cannot be established, report stale rather than claim exact alignment.

## 4. Input contract: Cockpit to browser

A browser view exposes operations equivalent to attach/detach, take/release interaction, send input, resize viewport, navigate, history back/forward, reload/stop, list/select/create/close browser tabs, inspect point, and respond to browser dialogs. These are human-surface operations, not a generic `evaluate`/CDP/shell API.

### Pointer and wheel

- Forward hover, move, press, drag, release, left/middle/right buttons, modifiers, double/triple-click count, and wheel X/Y. Retain pointer capture through a drag outside the image; release the pressed state on cancellation, lost capture, blur, mode switch, detach and lease loss.
- Coalesce idle motion to the newest point. Preserve ordered down/up/key boundaries; flush the final drag position before release. Coalesced wheel events accumulate deltas, not just the latest delta. Convert `deltaMode` to CSS-pixel deltas.
- Reject new presses in letterbox/padding/chrome. Out-of-bounds continuation of a captured drag uses an explicit tested boundary rule; it must still release remotely.
- Prevent Cockpit page scrolling/context menus only when the browser surface owns that input. Right-button page events and any implemented browser context menu are separate; do not claim Chrome's native context menu is streamed.
- Bound the input queue; overload disables input and releases held state with an inline error. Never quietly drop a key-up or replay an uncertain click after reconnect.

### Keyboard, text, composition, clipboard

- Workbench routing runs before browser forwarding. Preserve the current prefix policy and modal/editor ownership deliberately; an annotation textarea/address bar is a local editor, while the remote page input sink belongs to the browser surface. No duplicate DOM/xterm/remote delivery.
- Carry key/code/location/modifiers/repeat for physical key transitions. Treat committed text separately from physical keys so Unicode, AltGr, dead keys and shortcuts do not double-insert.
- Use a real focused text input sink with composition start/update/commit/cancel; map to supported CDP composition/text operations. Do not copy the POC's synthetic key pair on `compositionend` or its 16-character text limit. Test actual IME on native and web clients.
- Clipboard is explicit and permission-aware. Paste reads local clipboard only from a user action and sends bounded committed text once; copy reads a bounded selection through the browser adapter and writes locally on a user action. No background clipboard synchronization. Browser-reserved host shortcuts that cannot be intercepted get equivalent visible controls.
- Track held keys and buttons at the helper. A disconnected controller expires and releases them even if the frontend cannot send cleanup. Reconnect does not restore held input or auto-replay typing.

### Multiple clients and agents

Exactly one Cockpit view controls a target and its viewport at a time. Other views observe and scale the same pixels; their sizes must not fight over `Emulation` settings. Takeover is explicit; lease loss cancels input and leaves observation. A local browser lease is distinct from Herdr's terminal ownership and must not become a global agent lock.

Agents retain direct Playwright access. They can navigate, resize, open targets or mutate DOM during local work. Those changes invalidate/reconcile the view and annotation evidence. Do not promise exclusive page access or replay a human action to compensate for an agent mutation. Follow external navigation on the displayed target; newly created targets appear in the tab list. Automatically select a popup only when it is correlated to the local user gesture; background/agent-created tabs must not steal focus without explicit policy. Selection of a different target inside the CLI is not assumed to be a documented CDP active-tab event.

## 5. Feedback contract: browser to Cockpit

| Event group | Source and required behavior |
| --- | --- |
| Target created/changed/destroyed/crashed | CDP Target/page lifecycle. Maintain real IDs, title/URL and tab order under a declared owner policy; two identical URLs remain distinct. Close/detach selects a surviving target only by an explicit rule, never by URL matching. |
| Navigation/loading/history | Page navigation, same-document navigation, lifecycle and failure events, plus bounded history snapshots. Cover redirects, hash changes, `pushState`/`replaceState`, back/forward, reload and agent navigation. URL edits in local chrome remain separate from confirmed page URL; errors do not commit a requested URL as truth. |
| Title and page state | Target updates plus a narrow bounded document observer where necessary. Changes arrive without a local command and without a JPEG change. Do not repeatedly evaluate the full page on each input ACK. |
| Viewport/scroll/zoom | CDP layout metrics and bounded scroll/visualViewport probes; page scroll and page zoom are distinct from host UI scale and JPEG resolution. Expose geometry freshness. |
| Cursor | Compute the effective cursor at the latest pointer using bounded hit testing, including editable `auto` behavior. Re-probe after relevant page/layout/style changes even when the pointer is stationary. CDP screencast frames do not include a native cursor-change event. Version cursor results by document/viewport/pointer sample; stale replies cannot overwrite the current cursor. |
| Focus/editability/selection | Bounded DOM/Accessibility inspection for input sink/IME/clipboard support; no full DOM mirror and no password-value capture. |
| Dialogs and other blockers | Explicit JS dialog open/close and response operations. File chooser/download/permission events become browser-local UI or a visible capability refusal, not an invisible native window. |
| Runtime/control/error | Helper/browser disconnect/crash, viewport authority loss, lease revoked, metadata gap, frame/decode failure, and operation rejection/unknown outcome. Preserve last-known content and independent saved feedback. |

Use isolated-world probes where available and CDP frame sessions for cross-origin subframes. No DOM annotation host, style injection, page-controlled callback credentials or privileged host API inside remote content. Treat all probe results as untrusted, bounded data. Cursor values use a shared allowlist covering directional/text/drag states; an unsupported custom `url(...)` cursor reports a safe fallback without fetching page-supplied assets into Cockpit. DOM inspection limitations on canvas/closed shadow/internal pages must remain visible; they do not disable image-region/freehand annotations.

## 6. JPEG, decode, and geometry contract

### Binary envelope and backpressure

The POC IPBF v1 uses a 48-byte header, u32 sequence, JPEG pixel dimensions, visual viewport size/scale/offsets and JPEG length. It lacks production identity. Define one versioned successor envelope with length/version, stream epoch, target/document binding (directly or via the immutable stream descriptor), viewport revision, frame sequence, capture timestamp, image dimensions and geometry. Specify byte order and integer widths once. Use a u64 representation safe across Rust/JS or rotate the epoch before sequence overflow; never silently wrap u32 and discard future frames as old.

CDP supplies base64 JPEG; decode once in the helper. Do not pretend this removes Chromium encoding cost or guarantees GPU/zero-copy transport. Reuse immutable encoded frame buffers across viewers. Deduplicate only when **both pixels and geometry** match. A scroll can matter even when every JPEG byte is identical.

Keep CDP ACK pacing independent of viewer ACKs. Bound callback processing explicitly rather than assuming Chromium always has exactly one outstanding frame. Initial tuning follows the POC: JPEG quality 70, target cadence around 30 Hz, maximum capture 2560×1600 and 6 MiB encoded frames. These are starting configuration values, not measured service guarantees. Enforce encoded bytes, decoded pixel area and dimensions before allocation; the transport dimensions alone are not a sufficient decompression budget.

Per viewer: at most one transmitted/unacknowledged frame and one latest pending replacement, plus a bounded socket write buffer. ACK after a frame is presented or deliberately discarded, not merely assigned to `img.src`. An ACK timeout disconnects only that slow view. Metadata, input releases and other viewers must continue. A static page with no changed frames is healthy; do not use absent video frames as a heartbeat failure.

Frontend: one decode in flight and one latest pending packet. Bind each decode to its own immutable identity/geometry. Only an eligible newest decode may atomically publish pixels **and** presented-frame metadata. Close obsolete ImageBitmaps or revoke object URLs on success, replacement, error and detach. Use `createImageBitmap` + canvas if supported; an `img.decode`/offscreen-image path may supply equivalent semantics for WebKit. Do not use mutable shared image datasets as proof of which asynchronous decode completed.

### Coordinate spaces

Keep four explicit spaces: Cockpit client CSS, painted-image pixels, Chromium viewport CSS, and document CSS. The image's actual painted rectangle excludes toolbar and letterboxing. For the validated unzoomed baseline:

```text
u = (clientX - paintedLeft) / paintedWidth
v = (clientY - paintedTop)  / paintedHeight
imagePoint = (u * jpegWidth, v * jpegHeight)
viewportPoint = (u * viewportCssWidth, v * viewportCssHeight)
documentPoint = viewportPoint + confirmedDocumentScroll
```

This formula is not a license to guess under pinch/page zoom, visual viewport offsets, screencast `offsetTop`, iframe transforms, or host DPR. Record their meanings separately and derive/test the full transform from the captured viewport descriptor. POC `currentViewport` queries layout metrics after pixels and its fallback conflates offsets; do not port that as exact capture-time geometry. The presenter, input mapper, picker and annotation compositor consume **one shared transform** for the presented frame. No independent `devicePixelRatio` multiplication in each subsystem.

Resize requests are controller-owned, coalesced and revisioned. Set browser viewport from the image content area, capture at a bounded resolution, and display the last frame while waiting. Enable geometry-sensitive input only after a frame for the acknowledged viewport revision is presented. On takeover, use the new controller's size once; observers scale without sending resize. Page zoom and host UI/window zoom need separate scenarios.

### Visibility and recovery

Visible views subscribe; hidden views dispose decoder/buffers/frame sockets and release control. Stop screencast when there are no visible consumers, not the browser or page processes. Keep only the low-volume metadata needed by the owner. Resume from a new stream baseline. Snapshot freshness and page lifecycle are independent of JPEG activity.

On helper crash revoke view tokens/leases and preserve profiles/feedback. Restart only a verified owned helper, reconcile the existing browser, reattach and issue a new epoch. Do not launch a second browser/profile or close the existing browser just because capture failed. On owner shutdown, stop capture/helper and close only its verified CLI session using the existing lifecycle policy. Closing an observing web tab is not owner shutdown.

## 7. Security boundary

- Bind helper endpoints to loopback only. Validate Host/Origin, negotiate a version, and require short-lived unguessable view-scoped credentials before pixels/metadata/input. Origin checking alone is not authentication; loopback alone is not authorization.
- Prefer first-message credential negotiation so tokens do not enter URL logs; pre-auth sockets get a small deadline and byte limit and receive no image. Bind credentials to owner/incarnation/target/view and revoke on detach, switch, close and owner exit. Do not grant mutation permission from a frame-view token.
- Never expose raw CDP endpoints, runtime socket paths, frame tickets, profile secrets or pairing tokens to remote page JavaScript, image evidence, logs, agent feedback text or DOM attributes. Ordinary local Playwright access remains the existing trusted-workstation boundary, not a multi-user security claim.
- Remote HTTP(S) navigation is intentionally allowed for local development and the web. Do not mislabel all localhost browsing as SSRF; instead ensure pages cannot reach Cockpit's privileged control endpoints without authentication. Keep `file:`, `javascript:`, arbitrary DevTools targets and credentials in URLs out of user navigation commands; allow only explicitly handled internal blank/error states.
- Render metadata as text, never page HTML. Bound strings/locators, JSON/event queues, frame area, upload sizes and peer counts. Don't fetch favicon/custom cursor URLs from the privileged origin implicitly. Keep Chromium sandbox enabled.
- Native capability/CSP and browser gateway authorization must match. Do not copy the POC's global Tauri object or permissive loopback policy into application components.
