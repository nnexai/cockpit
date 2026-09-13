# Delivery sequence and verification

Status: implementation complete for the user-authorized no-migration cutover. Focused static, browser/gateway, and Linux-native startup checks passed on 2026-09-13. The broad A01–A25, security, performance, and native input/decode matrices are not claimed; A23 is excluded.

## 1. Execution rules

Each increment has one observable outcome, owned files, a real-surface proof and its own commit. Research/source inspection is not runtime proof. No active/default Herdr sessions, personal Chrome profiles, or installed Cockpit processes are used for experiments or restarted without separate authorization.

Before implementation, capture the worktree baseline, current configuration/tool versions and latest browser/annotation behavior. Preserve unrelated work, including the neighboring MediaStream POC and rendering-results edits present during this planning task. Do not rerun historical terminal bootstrap or roll Herdr back to an older release.

Use disposable names such as `inline-browser-<run-id>`, dedicated Chromium profiles and an isolated Cockpit state/config root. Record ownership of every helper/browser/server/session/fixture before starting it. Clean up only those resources. Both the shared browser client and a real Linux Tauri window are required because the feature crosses IPC, native WebView decode, input and lifecycle boundaries. macOS claims require a macOS run; Linux proof is not portable proof.

## 2. Bounded implementation increments

### IB-00 — Prove one browser can serve pane and agent

**Outcome:** a disposable CLI-managed, persistent-profile Chromium remains usable by ordinary Playwright CLI while a separate helper receives its screencast, inputs into it and observes navigation/metadata.

- Verify installed CLI/Chromium/Node capabilities, launch flags, endpoint discovery and process ownership. Launch windowless unified Chromium; preserve the named session and normal working-directory invocation.
- Attach CDP without taking over CLI lifecycle. Use two same-URL targets, redirects, SPA history, reload and an agent DOM change to determine actual identity/event behavior.
- Exercise the binary POC path in native WebKitGTK and the shared browser host with scoped credential/Origin/CSP policy; establish decode and composition availability and packaging/runtime prerequisites.
- Specifically establish new-document/resize frame barriers and what capture metadata can be tied reliably to the pixels. Do not infer correctness from a screenshot that merely looks plausible.
- Prove IME/text operation feasibility and identify actual JS dialog/file chooser/download/permission event behavior. Enumerate browser facilities that the image surface does not provide, including audio.
- Record bounded static/typing/scroll/animation observations for later comparison; no performance target is considered measured in this plan.

**Ownership:** one temporary runtime probe owner; independent read-only review of launch/security and geometry/input evidence may run concurrently. Do not modify production `BrowserService` just to make a probe convenient. Remove throwaway runtime scaffolding after recording proof.

**Gate:** same tab/state seen by CLI and pane; scoped close leaves an unrelated sentinel browser intact; profile survives reopen; native and web frame/control paths work. Failed launch coexistence, epoch correctness or ordinary input support blocks dependent implementation. Resolve the specific seam—do not substitute a second browser, extension, screenshot polling or custom agent API.

### IB-01 — Display the selected Space browser inline

**Outcome:** existing browser Open creates/reuses the association and reveals real JPEG content in the proposed split on both clients; hiding it preserves the browser and feedback.

- Define identity/descriptor/error DTOs and generated types, `CockpitClient` view subscription and disposal.
- Add supervised helper attachment and view-ticket lifecycle to the existing owner. Implement async command/event reading and observer forwarding.
- Add typed frame envelope, bounded transport/decode, snapshot/event barrier and lifecycle status.
- Add the Space-scoped presentation split and visible loading/stale/closed/retry states; preserve Herdr layout IDs/order and Files/Review renderer behavior.
- Keep the production replacement opt-in to **disposable development associations** while incomplete. Do not change defaults or load both annotation producers into a user's live association.

**Gate:** GUI and CLI opens deduplicate; one real browser/profile; matching first frame; responsive hide/show/Space switch; delayed old frames cannot paint the new Space; owner versus observer close behavior holds. Browser and native capture proof, not a standalone POC-only result.

### IB-02 — Complete bidirectional browsing

**Outcome:** the inline browser can actually be used, and browser/page/agent changes are reflected without local requests.

- Implement pointer/drag/wheel/key/text/IME/clipboard, focus release, bounded ordered queues and explicit controller takeover.
- Implement viewport resizing, accurate transforms and presented-frame gating.
- Add target tabs, URL editing, history, reload/stop, page-driven cursor/title/URL/loading/history events, iframe/document lifecycle and explicit browser-blocker UI.
- Preserve real remote page context-menu events. Provide supported local actions separately; do not promise a native Chrome menu through JPEG.
- Implement JS alert/confirm/prompt handling. For file uploads, use explicit user-selected files through a narrow validated adapter—never page-supplied arbitrary filesystem paths. Downloads are written only to an owned configured destination and exposed with status; external-open is deliberate. Permission requests have visible allow/deny or unsupported status, never broad auto-grant. If a facility is outside selected support, publish that capability before interaction, surface the event, and allow cancellation so the browser cannot silently hang.
- Measure and fix slow decode/backpressure without adding a new transport architecture.

**Gate:** acceptance rows A01–A13 below. Validate actual native OS composition/clipboard behavior rather than just synthetic DOM events. URL changes, cursor changes under a stationary pointer, long text and navigation during queued input are release gates, not later polish.

### IB-03 — Author complete annotations on the image

**Outcome:** Browse/Select/Freehand/Region/Element and inline notes work on the live image, with real element evidence and honest alignment/recovery.

- Implement shared image/viewport/document transform and local overlay reducer, gesture ownership and note placement.
- Implement narrow target/frame-aware DOM picking; no on-page annotation host or data-attribute anchors.
- Add owner-persisted revisioned drafts, capacity/conflict handling and stale recovery.
- Add capture preparation, pinned-frame composition and PNG submission to the existing store. Separate public evidence from transient CDP handles.
- Preserve the current tool colors, stroke widths/simplification, optional text, notes overview, layout review and capture-as-shown behavior.

**Gate:** A14–A18 plus ordinary browsing regression. Actual exported PNGs must contain the real page, marks and comments and exclude editing chrome. Native and browser geometry must agree at zoom/resize/scroll; drawing over a page button must not activate it.

### IB-04 — Preserve feedback, ownership and old work

**Outcome:** preserve the complete extension-free feedback loop and readable saved captures. Legacy profile migration is explicitly excluded by the user's cutover instruction.

- Wire inline feedback/recovery to existing lookup/image/send/ack operations; preserve optional preview and recipient selection rules.
- Do not add legacy reader/import or migration-journal behavior; A23 is outside the authorized scope. Existing saved feedback remains readable.
- Exercise duplicate saves, lost responses, unknown paste receipts, capacity, retention, and interrupted delivery.
- Preserve named CLI status/open/close, `--current` after pane move and readable feedback with a closed browser.

**Gate:** A19–A22 and A24 remain the defined future matrix for feedback, delivery, ownership, and capacity; they were not broadly exercised by the focused pass. A23 is excluded.

### IB-05 — Cut over and remove the old runtime

**Outcome:** the inline implementation is the only production Space browser/annotation path, with documented limits and no obsolete extension/external-window dependency.

- Inventory every affected callsite before removing exported symbols; use LSP references and generated-type drift checks.
- Apply the documented no-migration cutover boundary, then remove extension runtime/HTTP/pairing/build embedding and external-window Show semantics. Keep historical evidence only, not a second supported runtime or import path.
- Update native permissions/CSP, packaging/install prerequisites, usage/keybinding/code-guide documentation, architecture decisions and original browser-plan status.
- Record focused integration evidence and keep the broader acceptance/security matrix explicitly unclaimed; do not expand scope through review-driven follow-up.
- Remove throwaway fixtures/scripts after proof; keep only behavior regressions that defend plausible failures and receipts needed to understand verification.

**Gate:** focused cutover proof covers the inline path, no extension install/submission runtime, no external Show dependency, one helper per owned browser, hide/show resource handling, and no unrelated work included. The broad A01–A25 gate is not claimed.

## 3. File boundaries and real parallel work

The names of new modules below are proposed placement, not instructions to scaffold every file before its behavior exists. Prefer a few clear browser-specific modules over a plugin/transport framework.

| Lane | Existing seams / prospective ownership | Shared contract |
| --- | --- | --- |
| Runtime/CDP | `crates/cockpit-core/src/browser.rs`; browser-specific runtime service module; packaged helper under a dedicated production browser-runtime directory | Commands, identities, metadata, narrow inspection, owner lifecycle; no UI state or generic CDP endpoint |
| Owner/host adapters | `crates/cockpit-host/src/browser_runtime.rs`, `server.rs`, `src-tauri/src/lib.rs`, permissions/capabilities/config | Same command/event/error semantics in native/web/observer paths; delegated frame grants |
| Client/presenter | `src/client/CockpitClient.ts`, `browser.ts`, `native.ts`; proposed `src/app/browser/` presenter/input modules | Opaque view handle, typed metadata, disposable frame source, presented-frame descriptor |
| Annotation/drafts | Proposed `src/app/browser/` overlay/picking/capture modules; core browser-draft service | Shared transform, draft revision, immutable capture transaction; calls typed client only |
| Feedback/delivery | `browser_feedback.rs`, `browser/delivery.rs`, inline draft/capture methods | Existing pending-ID/paste receipts, durable images, and extension-free inline feedback; legacy migration is excluded |
| Integration owner | `src/app/App.tsx`, shared styles/key routing, protocol exports/generated TS, manifests/build embedding | Pane composition, focus boundary, one contract revision and final integration |

Freeze DTOs, event/stream identity, transform semantics, draft/capture revisions and ownership policy inline before assigning consumers. In IB-01/02, runtime/host work and client presenter work can run concurrently after that contract, but the integrator owns shared registrations and `App.tsx`. In IB-03, overlay authoring and core draft persistence/inspection can run concurrently against the fixed contract. Feedback migration work can begin alongside IB-03 once provenance/storage versions are decided; it does not need completed drawing UI.

No siblings edit the same files/symbols. Reserve shared `browser.rs` and protocol/generated files to one writer at a time; do not invent stub endpoints so another worker can claim completion. Workers skip builds, formatters, linters, runtime mutations, tests and commits while edits are in flight. Integrate once, run the narrow gate once, then exercise the real surface. Reviews of security/concurrency risks are independent read-only work, not delegated approval of scope.

## 4. Acceptance matrix

For every row record the actual user action, owner/CDP/Herdr response or event, presented result, failure result, client/platform/version and resource cleanup. Synthetic protocol tests complement these scenarios; they do not replace visual proof. Rows involving Linux-native input/IPC must pass on a real Tauri window.

| ID | Scenario | Pass criterion |
| --- | --- | --- |
| A01 | Open/reuse from GUI and an already-running agent; simultaneous opens | One association/browser/profile; real inline pixels; agent CLI sees the same page state; no unnecessary navigation without URL; explicit URL opens preserve existing tabs. |
| A02 | Space/tab switch, Hide/Show, narrow presentation, browser-target close | Herdr order/layout IDs unchanged; browser tabs remain local; new Space never receives old pixels/input/feedback; hide preserves page memory; closed browser leaves saved feedback accessible. |
| A03 | Normal native/gateway owner shutdown and observer close | Only verified owned browser/helper close; observer close does not stop owner; normal owning-runtime exit follows existing policy; sentinel unrelated browser survives and profile remains. |
| A04 | Hover/click/double-click/right/middle, drag outside pane, release/cancel | Page event log and rendered selection/control changes match; no stuck buttons; letterbox/chrome presses do not trigger page input; annotation gestures do not leak. |
| A05 | Wheel/trackpad X/Y, nested scroll, page selection | Accumulated wheel deltas preserved under coalescing; correct target scrolls; no host-page scroll; ordinary text drag selection works. |
| A06 | Typing, modifiers, repeat, navigation keys, AltGr/dead keys, Unicode/IME, paste/copy | Real editable page receives text exactly once, including a long composition/paste; candidate/commit/cancel behavior works on native and web; local address/note editors stay local. |
| A07 | Terminal → browser → terminal, workbench prefix, modal/editor focus | No input duplication/leak; old terminal cannot type while browser owns focus; Herdr confirmed focus/takeover works on return; no uncontrolled ownership reclaim. Compare real TUI handoff. |
| A08 | Two Cockpit clients observe/take over one target at different sizes | One controller/viewport owner; observer does not resize browser; lease loss releases keys/buttons; observation continues; explicit takeover resizes once and waits for matching frame. |
| A09 | Link, redirect, same-URL reload, hash, SPA push/replaceState, back/forward; agent navigation | Confirmed URL/title/history/loading update without local polling/input; no stale target/document frames accepted; duplicate URL targets retain distinct identity; URL edit draft is not overwritten while typing. |
| A10 | Stationary pointer while hovered element/cursor/style changes; navigation and frame change | Pointer/text/resize/drag/etc. cursor updates from metadata even without mouse motion; stale replies cannot overwrite; custom unsupported cursor has a declared safe fallback; no page asset fetch from Cockpit. |
| A11 | Resize split/window, host zoom/DPR, page zoom, scroll, letterboxing, fixed/sticky elements | Fixture hit targets and exported mark positions agree within 2 displayed CSS pixels in controlled fixtures; no out-of-image clicks; old geometry disables input until matching pixels; controller and observer do not fight. |
| A12 | JS dialog, file chooser, download, denied permission; missing browser/helper | User sees actionable local state and can respond/cancel; no invisible native dialog deadlocks navigation or input release; unsupported facilities are explicit; no arbitrary file read or auto-grant. |
| A13 | Malformed/oversized JPEG, delayed decode, slow/no-ACK viewer, metadata gap, helper/owner crash | Bounded memory/queues; newest eligible image wins; stale last image retained with input blocked; no old input replay; other viewers/metadata/releases progress; restart reuses verified browser instead of duplicating profile. |
| A14 | Freehand/region in both directions, color choice, edit/remove, optional text, notes clear/revisit | Full parity table in document 02 passes; correct live stroke and release simplification; no invented drawing text; annotations survive hide/reopen; all controls reachable at desktop/minimum/narrow widths. |
| A15 | Element pick in ordinary DOM, same/cross-origin iframe, shadow root, transformed/nested scroller, canvas | Actual inspectable element context and geometry; late hover/pick replies discarded; honest boundary for inaccessible content; region/freehand remains usable. No selector-only retargeting. |
| A16 | Unrelated heartbeat versus moved/removed selected element; scroll/resize/reload/tab switch while drawing/capturing | Unrelated mutation does not block; real drift prompts review; obsolete gesture cancels without losing earlier marks; wrong-document capture refused; explicit capture-as-shown never bypasses identity/transform failure. |
| A17 | Capture at multiple DPR/zoom sizes, freehand-only and annotated element/note | Saved PNG visually matches pinned browser image plus marks/comments, excludes all editing chrome, carries correct page provenance and public element fields; agent read tool can open it. |
| A18 | Failed composition, full disk/store, lost save response, concurrent edits, later navigation, restart | Drafts/pending pixels retained; retry uses same ID/bytes; new edits are not cleared by old receipt; no silent eviction; composed historical capture retry succeeds without recapture. |
| A19 | Agent fetch twice, save newer capture, acknowledge exact older IDs twice, close browser | Fetch never consumes; ack is exact/idempotent; newer capture remains pending; closed-browser feedback/image access works. |
| A20 | Send browser context and selected feedback, no eligible agent, wrong Space/destination move | Fresh same-Space active-tab recipient, no picker/fallback elsewhere; paste bytes appear unsubmitted with no Enter; rejection preserves work; preview remains optional. |
| A21 | Unknown paste response, later CLI ack, retention pruning, explicit risky retry | Unknown receipt persists; no automatic duplicate paste; duplicate-risk acknowledgement required; handled images expire only by policy; unsent work is never retention-pruned. |
| A22 | Agent pane moves between Spaces; Space rename, close and endpoint replacement | `--current` follows fresh authoritative membership; rename preserves association; closure is scoped; endpoint mismatch/reused IDs cannot adopt/kill another browser. |
| A23 | Legacy saved capture/draft/pending PNG/consumed-ID migration | Excluded by user instruction; no migration is implemented or claimed. |
| A24 | Capacity and multi-client draft races | Ninth unfinished draft fails explicitly without eviction; revision conflicts/tombstones prevent lost/resurrected work; bounded capture/store limits and retention remain consumer-correct. |
| A25 | Clean-cutover security and long-running integrated surface | No extension install/pairing/submit runtime, no external Show dependency, old credentials rejected, one helper per owned browser, hidden capture stopped, resources released, no unrelated work included. |

### Security negative cases

Exercise wrong/missing/expired credential, wrong association/target, observer attempting mutation, old lease after takeover, hostile Origin/Host, unsolicited pre-auth bytes, metadata/control floods, invalid frame envelope and decoded image area, hostile title/locator text, page attempts against loopback APIs, and lost-client stuck-key cleanup. Confirm remote page probes cannot read tickets or invoke Cockpit tasks. Loopback binding is not itself a passing security test.

### Performance proof, not marketing

Use the same fixture, dimensions and Chromium build for POC/production comparisons. Record presented frame age, input dispatch-to-observed-page response, metadata latency, dropped frames, live decoder/object/buffer counts, process-tree CPU and PSS. Clock comparisons need a measured clock relationship or in-process intervals, not raw timestamps from unrelated clock domains.

Suggested starting acceptance budgets, to validate at IB-00: controller click-to-visible-response p95 below 150ms on a local static fixture; title/URL/cursor feedback below 250ms under normal load; sustained visible animation without a growing frame backlog; exactly the bounded slots described in document 01. These are proposed budgets, not POC measurements. A platform unable to meet them requires an explicit evidence-backed decision, not a fabricated pass or a quiet threshold change.

Run static idle, fast typing, scroll, resize burst, animated page, video/fullscreen, a deliberately slow observer and repeated hide/show/Space switching. After warm-up, a five-minute animation/reconnect run should plateau in live buffers and memory rather than grow per frame. Static idle must not trigger repeated screenshots or continuous DOM polling. A slow viewer must not throttle local input or other viewers. JPEG stream audio is absent; record that explicitly. Validate fullscreen within the pane instead of repeating the recorded zoomed/cut-off POC result.

## 5. Static checks and retained regressions

Run the repository's formatter/type generation/typecheck/build gates only after the owned edits integrate. Use existing project commands; do not pin stale suite totals from historical handoffs. Native packaging is verified from a clean run-owned install prefix so the helper/runtime dependency is actually discoverable outside the source tree.

Keep regression tests only for plausible failures: epoch/late-decode ordering, snapshot/event barrier, frame-vs-metadata gap semantics, accumulated wheel/release ordering, coordinate transforms, document-bound picking, draft conflict/tombstone handling, retry identity, migration interruption, and uncertain paste receipt retention. Test public behavior, not source text, a particular helper filename, or DTO field forwarding. Use throwaway smoke fixtures for straightforward feature proof. Delete superseded wording/plumbing tests rather than repinning them.

A build does not establish input routing, native image decode, element alignment, profile safety or annotation parity. Each increment's completion record must name its commit and the exact acceptance rows exercised, distinguish browser/native results, and mark unavailable-platform evidence unverified.

## 6. Design risks to resolve, not defer invisibly

| Risk | Decision/gate |
| --- | --- |
| CLI-managed launch may not expose the required safe CDP/headless combination | IB-00 proves the single-browser path before production work. No dual-browser workaround. |
| Screencast metadata and asynchronous DOM inspection are not atomic | Explicit frame/document barriers and shared transform; inconclusive evidence refuses exact alignment. IB-00/A09/A11/A16. |
| Native WebKit decode/IME/loopback policy differs from desktop Chromium | Test the real native surface early, not only at final delivery. IB-00/A06/A13. |
| Direct agent operations bypass a local browser lease | Cooperative semantics are explicit; react to changes and preserve drafts. No exclusive-agent-control claim. |
| New split could be confused with Herdr layout/focus | Browser-local tab/chrome/hide semantics and explicit architecture departure; TUI ownership comparison. A02/A07. |
| Removing extension loses work only stored in its profile | Transactional one-shot export/import including pending PNG/consumed IDs, before shutdown/removal. A23. |
| Headless image surface loses native browser facilities | Explicit audio/media limit and browser-blocker capability table; no silent dialog/permission hang. A12. |
| Binary transport improves downstream memory but not capture encoding cost | Measure capture and presentation separately; no FPS/CPU claim inherited from the POC. A13/performance scenarios. |

This plan records the selected outcome, seams, and proof requirements. The implementation and focused evidence are recorded in the current [inline handoff](HANDOFF.md); no unlisted acceptance row is inferred as passed.
