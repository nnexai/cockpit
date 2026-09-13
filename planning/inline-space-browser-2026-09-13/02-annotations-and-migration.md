# Annotation parity, capture, and migration

Status: proposed implementation contract. [Overview](README.md) · [Transport](01-architecture-and-transport.md) · [Delivery](03-delivery-and-verification.md).

## 1. Surface ownership

The pane presents the latest accepted JPEG, an SVG mark layer, and HTML inline note editors/labels. Browser-local chrome owns tool selection, notes overview, capture status and saved-feedback entry. Nothing is installed into the remote page to render annotations. Native page dialogs/top-layer elements therefore cannot cover Cockpit's annotation controls.

Browse mode forwards input to Chromium; annotation modes intercept it locally. Selecting a tool first cancels/reconciles remote held input and composition. Drawing never clicks a remote payment/submit button. Switching back to Browse restores normal hover/cursor/input, not a stale annotation pointer capture. Local text editors retain their own caret, selection, multiline Enter and explicit save/cancel behavior.

Annotation authoring remains live, not a mandatory screenshot-first editor. A freehand stroke/region gesture pins its geometry for the gesture; if an agent or page changes geometry mid-gesture, cancel only that unfinished gesture visibly and preserve earlier work. Briefly pin the displayed image at Capture, not at entry to annotation mode. A separate stale-draft recovery view may show historical pinned pixels when actually available.

## 2. Required parity inventory

The [current content script](../../browser-extension/content.js), [worker](../../browser-extension/background.js), [popup](../../browser-extension/popup.js), and [latest annotation polish evidence](../product-atlas-2026-09-12/polish/execution.md) define the baseline. Preserve behavior, not white extension styling or MV3 storage mechanics.

| Existing function | Inline equivalent and acceptance |
| --- | --- |
| Browse/Annotate switch | Explicit modes in browser chrome. Browse reaches the real page; drawing/picking consumes pointer input without underlying actions. |
| Freehand | Immediate visible stroke, current 3px baseline, existing release-time simplification, continuous points and cancellation. No mandatory comment and no fabricated freehand label in the image. |
| Region | Drag a normalized rectangular region in any direction, current 2px baseline; optional inline comment, selection and removal. |
| Element | Hover highlight, select the actual browser DOM element, return bounded structured context and geometry, add/edit a note. Do not replace it with image/OCR guessing or a rectangle called an element. |
| Select/edit | Select a mark or click its inline text; edit/remove without changing other marks. Preserve the existing tool-key mappings where applicable and keep typing isolated from shortcuts. Do not add undo/redo as an unexamined prerequisite. |
| Colors | Retain the existing five-color choice and per-mark color in saved structured data and composition. |
| Optional text | Freehand starts without text; `+ Text` opens an inline editor beside the mark. Existing text is editable there. Region/element notes follow the same flow. |
| Notes overview | Optional, initially collapsed list, select/revisit an annotation, visible count and explicit clear. It shares state with marks/editors; it is not a second comments collection. |
| Tool visibility | Preserve subdued idle tools and full hover/keyboard-focus visibility without dimming annotations or changing hit geometry. Use current Cockpit tokens. |
| Geometry review | Distinguish aligned, review required, stale document, offscreen and unavailable evidence. Review evaluates relevant anchors, not every DOM mutation. Unrelated heartbeat/animation must not permanently block capture. |
| Capture anyway (as shown) | Explicitly permit changed layout when the displayed pixels/marks can still be honestly captured. Never bypass wrong Space/target/document, missing pixels, invalid viewport transform or out-of-bounds capture. Label stale element evidence instead of asserting a fresh DOM match. |
| Capture summary | Capturing/saving/saved/failed with counts, saved feedback entry and retry/discard. Preserve the short annotate/save/continue loop. |
| Recovery | Hide pane, switch tabs/Spaces, reconnect and restart without silent draft eviction. Recover stale document drafts separately; do not place old marks on a new page. |
| Saved feedback | Existing images, optional message edits, exact acknowledgement, direct send, no-agent and uncertain-delivery states remain available even with the browser closed. |

Widths refer to the annotation surface's CSS geometry. Image export scales them consistently with the shared transform; verify both live and exported readability at host zoom and high DPR.

## 3. Actual element inspection, without an annotation extension

Expose a narrow `inspectPoint` operation, not arbitrary page evaluation. Its request includes browser incarnation, target/document/frame identity, presented-frame/viewport revision, pointer sample sequence and viewport coordinates.

The helper uses CDP DOM hit testing (`DOM.getNodeForLocation` or a verified equivalent), frame/session routing and box/quads inspection. A bounded isolated-world query may supplement computed style, text and locator hints; Accessibility data may provide name/role when available. Neither the UI nor page receives privileged Cockpit APIs. Return:

- request identity and current document/frame generation;
- a transient node handle/backend-node ID valid only for that document;
- viewport/document geometry or transformed quads with explicit coordinate space;
- bounded tag/text/role/name/locator hints/excerpt;
- inspectability/freshness information, not an assertion that a locator is unique or permanent.

Persist only the public element evidence fields already supported by `BrowserElementEvidence` (`tag`, `text`, `role`, `name`, `locators`, `excerpt`) plus intentionally versioned new capture context. Keep live CDP node IDs, execution-context IDs and internal rectangles in the draft/inspection model, not accidentally spread into the strict capture DTO. The recent extension save repair demonstrates why explicit projection matters.

Hover requests coalesce. Late responses for an old pointer/document/frame cannot move the current highlight. Selecting requires a matching response; never silently reuse the previous hovered element. Bracket inspection with document/viewport/anchor checks. Fast page movement can still race CDP pixels; mark inconclusive evidence and ask for review/capture-as-shown rather than claiming atomic DOM/image alignment.

### Boundaries that must be exercised

- **Nested and cross-origin iframes:** route inspection through the right frame CDP session and transform coordinates through frame bounds, scroll and CSS transforms. Each node handle binds to the subframe document generation. Main-document `elementFromPoint` alone only identifies the iframe element.
- **Shadow DOM:** inspect deeper when supported; record an honest boundary when a closed root cannot be resolved. Do not fabricate selectors across inaccessible roots.
- **Canvas/WebGL/video:** the DOM element can be selected; inner rendered objects are not DOM nodes. Freehand and region annotation remain available for image content.
- **Sensitive content:** no password values, hidden full-page dumps, cookies or storage in element context. Source excerpts are bounded and rendered as text.
- **Layout changes:** compare selected nodes' geometry and membership. A removed/replaced node invalidates its live handle even if an identical selector or URL still exists.

Region/freehand fallback is a visible capability for genuinely non-DOM content, not permission to omit real element selection on ordinary pages.

## 4. Coordinates and capture semantics

Use the single presented-frame transform defined in document 01. Freehand points/regions remain document-CSS anchored as in the current extension, with their reference viewport and document identity. Element annotations retain original evidence and a transient live anchor for alignment checks. Do not silently move authored marks to a new element because its selector now matches elsewhere.

Scrolling in the same document may translate document-bound marks using confirmed geometry. Reflow/zoom/resize may require review. Nested scrolling, fixed/sticky elements and transformed frames must be checked against their real anchor geometry; adding top-level scroll offsets everywhere is wrong. Offscreen marks are not silently included in a visible-viewport capture. Let the user bring them into view or explicitly choose a visible subset, retaining omitted annotations unsent.

### Capture transaction

1. **Prepare:** snapshot selected annotation IDs and draft revision; flush local editor state. Validate association, current document, presented frame and transform. Obtain bounded anchor freshness results. Do not use a newly requested screenshot from another target.
2. **Pin:** retain the exact presented JPEG bytes/decoded bitmap, frame descriptor and annotation revision. Pause local presentation/input briefly, not the browser process or the agent. If required identity/geometry changes during preparation, abort before save and retain the draft.
3. **Compose:** render that pinned browser image, marks and the same positioned visible comment labels into a bounded export canvas. Exclude address/tab bars, tools, notes sidebar, hover outline, selection handles, editors and `+ Text`. Saved evidence is a PNG so existing readers/artifact delivery continue working; encoding JPEG pixels to PNG does not recover detail lost in streaming.
4. **Submit:** send the typed capture envelope with a stable capture ID and immutable pixels to the existing core feedback store through the ordinary client/owner boundary, not the extension-only HTTP endpoint. Initially the existing bounded `png_base64` submission is acceptable for this one-off artifact; it is **not** the live frame transport. Validate image magic/dimensions/area/size against declared evidence.
5. **Confirm:** only a durable store receipt clears the selected saved annotations. New edits made after preparation remain unsaved. Resume the live view on success or failure with explicit status.
6. **Retry:** if persistence fails after composition, retain the exact PNG, descriptor, capture ID and selected annotation IDs. Retry those bytes even after navigation/tab closure; never recapture a different page under the same ID. Duplicate ID with different content is rejected. A lost response is reconciled by capture ID.

Record URL/title, document/target identity, viewport, image dimensions, frame sequence/epoch, pixel capture time when available, and annotation/save time separately where the schema is extended. Do not claim the save time is the time Chromium produced the pixels. A page/agent navigation after pinning does not invalidate already frozen historical evidence, but it must not rewrite that evidence's metadata.

Capture-as-shown saves exactly what was pinned, with an explicit reviewed/stale-geometry flag if needed. It cannot launder uncertain element evidence into a verified current locator. If the frame-to-geometry relationship is unknown, capture preparation fails; a misleading image/coordinate bundle is not a fallback.

## 5. Durable draft and feedback model

Saved feedback continues to use `BrowserFeedbackStore` and its current limits/retention/receipt semantics. Extend only where target/frame provenance requires it. Keep old records readable as historical captures; Chrome numeric `tab_id` is not a Chromium CDP target ID.

Choose a versioned capture context with optional new target/stream provenance for old-record reads and mandatory provenance on new inline captures. This is durable-format migration, not two live browser APIs. Do not repurpose numeric tab IDs, invent `0`, hash string target IDs into an integer, or rewrite old capture evidence to look inline-native. Define compatibility decoding and migrated serialization before changing generated frontend types.

New unsaved drafts are owner-persisted, not stored only in React/localStorage. Key them by association, browser incarnation, target and document generation. Carry draft ID/revision and a small bounded editor view state; serialize writes per draft, reject stale revisions, and keep tombstones/consumed IDs so delayed autosaves cannot resurrect saved/deleted marks. Multiple clients must either acquire a draft-edit lease or get explicit revision conflict; browser input ownership alone is not sufficient draft synchronization.

Initial limits should preserve the existing bounded behavior: eight unfinished document drafts, one pending composed capture per association, 64 annotations per capture, 8,192 points, 4 MiB PNG, 6 MiB submission envelope, 64 pending saved captures, 256 MiB feedback store and one-hour handled-artifact retention defaults. Recheck source/config before implementation; do not silently raise limits to fit a high-resolution export. Expose capacity before losing work and offer explicit discard/export. Unsaved drafts and pending unsent pixels never expire under the handled-image retention timer.

The recoverable-state inventory includes live drafts, stale drafts, a composed-but-unsaved capture, saved pending IDs, handled captures inside retention, and delivery receipts. A hard browser crash cannot recreate unsaved page state or pixels that were never retained. State that limitation; preserve whatever structured draft/pinned evidence actually exists.

## 6. Preserve feedback and agent workflows

Keep the existing public lifecycle and feedback CLI discovery usable by already-running agents. `--current` continues to resolve the caller's actual Herdr pane/Space from fresh authority, never GUI browser focus. Status includes ordinary named-session/working-directory instructions and current connection state, not a frame token.

Fetch remains read-only; acknowledgement clears exactly the fetched IDs and is idempotent. New feedback stays pending. Saved images are readable when the browser is closed. No auto-send on save, no background agent interruption, no auto-ack on image read.

Direct Send browser context/Send to agent continues to resolve the first eligible agent in the authoritative active tab of the associated active Space, without a new recipient picker or fallback to another Space/tab. Browser DOM focus is not the recipient. Preserve focus confirmation and immediate destination revalidation, bracketed paste without Enter, accepted/rejected/unknown receipts and explicit duplicate-risk retry. Unknown paste receipts survive a later acknowledgement and artifact retention pruning.

Move the existing feedback presentation into a reusable browser-feature component if needed, but keep its lookup available independently of a live view. Message preview remains optional by the latest decision; preparation/revalidation remains required. Do not change Files/Review recipient policy as part of this browser replacement.

## 7. Migration and clean cutover

### Inventory before changing an association

Read owned receipt/profile status, durable feedback JSON/PNGs, delivery receipts and extension storage in the proven owned profile. Relevant existing keys include `cockpit.feedback.drafts`, `cockpit.feedback.pending`, `cockpit.feedback.consumed`, and pairing state. An extension may have pending pixels that were never submitted to Cockpit. Stopping/relaunching a profile before inspecting this state can lose the only accessible recovery path.

### One-shot migration sequence

1. Acquire the existing owner/association operation lock and a migration journal. Freeze new extension saves for that association during export; do not run two concurrent producers against the same unfinished draft.
2. Using the existing trusted owned-browser extension access, export drafts, pending capture bytes/metadata and consumed markers. Do not read arbitrary user profiles, raw browser databases, or cookies. If the profile is unavailable, mark migration blocked and preserve it; do not start an empty replacement and call it migrated.
3. Validate and transactionally import into core draft/pending storage. Retain original annotation/capture IDs and evidence. Deduplicate against existing saved receipts and consumed IDs. Legacy drafts whose Chrome document identity cannot map to a live CDP document become **stale recovery entries**, not live marks. Pending composed PNGs remain retryable with their original identity.
4. Record imported counts/hashes and a durable migration receipt before removing any legacy state. A crash at each boundary must resume idempotently without duplicate pending feedback or discarded captures.
5. Drain or explicitly recover in-flight capture/paste outcomes. Keep existing saved images and unknown-delivery receipts untouched. New-browser incarnation does not invalidate historical pending feedback.
6. At a deliberate cutover boundary, close/reopen only the proven owned browser if the headless launch change requires it, retaining the profile. Surface the loss of unsaved page JavaScript/forms; never restart the user's active browser silently as a deployment side effect.
7. Load the inline runtime without the annotation extension, revoke extension pairing credentials/HTTP access, and use only the new draft/capture producer. Profile data and migration receipt remain; deletion of unrelated browser data is not part of the cutover.

Provide a bounded one-shot import command/tool for remaining legacy profiles, not a permanently supported extension path. Profiles absent from the current migration run can be explicitly inventoried and imported later; block their inline open until import/disposition is resolved. The production extension runtime must not be retained as a hidden fallback. Package legacy export support only as an isolated migration utility while old data exists; it cannot accept live annotation submissions.

### Remove after migration and acceptance

- `browser-extension/` runtime UI/worker/popup and their embedded asset/install/pairing machinery in `browser/extension.rs` (extract feedback service methods first; that module currently also serves non-extension feedback operations).
- Extension-only `AnnotationServer`, `/status`/`/capture` authorization and startup plumbing in `browser_annotations.rs`/`browser_runtime.rs`.
- Extension asset dependencies in host/native builds, manifest loading/source-version logic and extension-specific protocol/capability entries that become obsolete.
- External-window `Show`/`bringToFront` behavior and UI wording. Migrate GUI actions to reveal the inline pane. Keep browser Open/Status/Close lifecycle semantics; remove obsolete Show from callers/protocol/CLI where exposed rather than retaining an alias that pretends to focus an external window.
- Superseded extension-only verification scripts after equivalent consumer-behavior checks exist. Keep useful fixtures and recorded historical evidence; do not delete the selected POC or unrelated experiments.

### Rollback

Before runtime cutover, keep implementation changes isolated so a code revert restores the old launch path without data changes. After durable migration, rollback uses the recorded pre-migration data backup/format boundary, never overwrites new feedback with an old profile export. Preserve all captures created since cutover. Do not implement automatic downgrade/dual-write synchronization. If an old binary cannot read new provenance, refuse that rollback until a lossless export/read-compatible path is available. No automatic rollback may restart the user's active browser.
