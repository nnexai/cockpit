# WEB-04 — Complete browser input navigation and ownership

## Outcome

Complete the supported bidirectional inline browser surface after WEB-02/03: pointer and drag release, wheel/trackpad scrolling, keyboard/text/IME, clipboard, navigation/history/targets, blockers, cursor updates, and explicit controller takeover. Browser input must not duplicate into Herdr terminals or local editors. Browser and native clients expose honest unsupported states for facilities they cannot safely implement.
Live browser image quality is best effort: keep animation, hover reactions, scrolling, and input active rather than freezing them for sharper frames; follow the canonical current contract and scenarios in [ACCEPTANCE.md](../ACCEPTANCE.md#live-browser-image-quality).

This is the broad A04–A12 increment for issue #9/#10 residual coverage, not permission to weaken Herdr authority or invent a second browser API.

## Evidence and starting points

- Baseline and evidence boundary: `6f6222b74e4f552ce697e61364cf653f4b6be29f`; #9/#10 are historical findings, while A04–A12 remain broadly unclaimed.
- Issues: https://github.com/nnexai/cockpit/issues/9 and https://github.com/nnexai/cockpit/issues/10.
- Read `planning/inline-space-browser-2026-09-13/01-architecture-and-transport.md` input, feedback, blockers, and ownership contracts and delivery matrix rows A04–A12.
- Source starts: `src/app/browser/BrowserPane.tsx` `remotePointer`, `onPointerDown/Move/Up/Cancel`, `onWheel`, `sendKey`, composition handlers, clipboard, navigation, target/tab commands, blocker rendering, and focus callbacks.
- Source starts: `browser-runtime/browser-helper.mjs` `command`, `requireControl`, `releaseHeldInput`, target transitions, navigation/history, cursor/focus observers, dialog/file/download/permission blockers.
- Source starts: `src/app/App.tsx` browser/terminal focus gating and workbench routing; `src/app/TerminalPane.tsx` attach/input/scroll cleanup; `src/client/browser.ts` and `src/client/native.ts` browser command/event adapters.
- Dependencies and locks live only in `planning/stability-and-gitlab-2026-09-20/tasks.json`; coordinate `app-shell`, `browser-ui`, `browser-helper`, and `client-transports` writes serially.

## Changes

1. Implement ordered pointer down/up/cancel and drag capture, including release on blur, detach, mode switch, lease loss, and out-of-pane continuation. Prevent chrome/letterbox presses.
2. Accumulate X/Y wheel and trackpad deltas with delta-mode conversion and nested-scroll targeting; prevent host-page scrolling only while browser surface owns input.
3. Route physical key transitions, modifiers, repeat, AltGr/dead keys, committed Unicode text, composition start/update/commit/cancel, and bounded user clipboard copy/paste exactly once. Keep address/note editors local.
4. Complete back/forward/reload/stop/address navigation, same-document events, target tabs, title/URL/loading/history/cursor metadata, and target/document barriers without local polling per gesture.
5. Surface JS dialogs, file chooser, download, permission, missing-helper, and other blockers as actionable browser-local state with cancel/deny/unsupported semantics; never expose arbitrary filesystem paths or auto-grant permissions.
6. Enforce one controller per target and viewport. Observer takeover is explicit; lease loss releases held input and leaves observation. On browser-to-terminal handoff, release browser input and require confirmed Herdr focus before terminal typing resumes.
7. Add a required Herdr parity/input ownership review, including a uniquely named disposable TUI comparison for terminal handoff and no uncontrolled focus reclaim.

## Non-goals

- No native Chrome context menu claim through JPEG; preserve remote page right-click events and provide separate supported local actions.
- No audio/media streaming, arbitrary CDP/evaluate API, page-injected annotation host, background clipboard synchronization, or auto-replayed uncertain mutations.
- No weakening of server visibility checks or Herdr session/layout authority.
- No performance/resource campaign beyond behavior needed to establish bounded input; WEB-06 owns measured queues/decoder lifecycle.

## Acceptance

1. A04: click/double/right/middle/drag-out/release/cancel matches fixture page events exactly once with no stuck buttons or annotation leakage.
2. A05: X/Y wheel and trackpad deltas scroll the intended nested page/container, preserve accumulated movement, and never scroll the host unexpectedly.
3. A06: typing, modifiers, repeats, navigation keys, AltGr/dead keys, Unicode/IME composition, copy, and paste arrive exactly once in a real editable fixture on browser and native surfaces; local address/note editors remain local.
4. A07: terminal→browser→terminal handoff has no input duplication/leak; old terminal is not writable while browser owns interaction; confirmed Herdr focus/takeover is required on return.
5. A08: two clients observe/take over one target at different sizes with one controller/viewport owner; lease loss releases held state and takeover waits for matching geometry.
6. A09/A10: redirects, hash, SPA push/replace, back/forward, reload, agent navigation, title/loading/history, and stationary-pointer cursor changes arrive from authoritative events; stale target/document replies cannot overwrite current state.
7. A12: dialog, file chooser, download, denied permission, and missing-capability cases provide visible response/cancel/unsupported state and cannot deadlock navigation/input or read arbitrary local files.
8. Browser and native gateway cleanup leaves no old key/button state after disconnect, target switch, helper crash, or focus transition.

9. For wheel, key, composition, and clipboard operations, preserve ordered boundaries even when motion or text jobs are coalesced. A queue policy may drop only superseded motion, never a release, key-up, composition-cancel, or acknowledged user boundary.
10. Navigation commands must not commit a requested address as confirmed URL until the authoritative event arrives; failed navigation retains the prior URL and reports the failure.
11. Keep two identical-URL targets distinct by target ID. Popup/background targets must not steal visible selection unless correlated with an explicit user gesture and policy.
12. For blockers, retain the browser-local pending state while awaiting response, release unrelated input safely, and allow cancel where the underlying facility supports cancellation. Never leave a hidden native dialog holding focus.
13. Cursor metadata is versioned by target/document/viewport/pointer sample; stationary style changes must update without a compensating pointer event.
14. The Herdr comparison must check workbench prefix handling, modal/editor focus, browser-to-terminal return, and the absence of duplicate terminal bytes, not merely visual focus styling.
15. In the canonical quiet, animation-without-input, hover, scroll, and return-to-quiet scenarios, verify that visible content and input reactions remain live; record delivered dimensions separately from CSS viewport/DPR and preserve geometry/input barriers. Do not require a freeze or reject a current frame on density alone. See [ACCEPTANCE.md](../ACCEPTANCE.md#live-browser-image-quality).

Keep this task behavior-focused. Queue high-water, decoder cleanup, and helper restart plateau measurements belong to WEB-06, while hostile credentials/origins belong to WEB-08.

## Verification

The integration owner must exercise A04–A12 on both browser gateway and Linux-native Tauri where applicable, with disposable Herdr session/TUI names, isolated browser profiles, real editable/IME/clipboard fixtures, nested-scroll and blocker fixtures, and a sentinel terminal. Record page event logs, input sequences, lease generations, target/document metadata, clipboard bytes, blocker responses, focus acknowledgements, and cleanup. Synthetic protocol checks may supplement but cannot replace native OS input and terminal handoff proof.

The evidence record must include:

- fixture page event logs for pointer, wheel, key, composition, clipboard, and navigation actions;
- browser and native input sequences, target/document metadata, controller lease changes, and blocker responses;
- terminal/TUI before-and-after focus acknowledgements and proof that no duplicate bytes reached the old terminal;
- explicit unsupported outcomes, cleanup of held input, and all disposable resources.

Do not substitute synthetic DOM dispatch for native OS keyboard, IME, clipboard, wheel, or terminal handoff behavior where the acceptance names those surfaces.

## Handoff

Provide changed paths, supported/unsupported facility table, and unresolved browser/native input risks. The integration owner must commit `runs/<run-id>/WEB-04.md` evidence plus a real task commit after all shared writers settle. Do not edit `tasks.json` here; use `../ORCHESTRATOR.md` for locks, resource guards, and commit discipline.
