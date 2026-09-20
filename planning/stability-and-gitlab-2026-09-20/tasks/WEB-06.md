# WEB-06 — Bound browser frame input and lifecycle resources

## Outcome

Bound frame decode, input queues, helper/view lifecycle, and owner/observer reconnect resources under malformed input, slow consumers, crashes, and repeated hide/show. The latest eligible frame wins; stale content remains visibly marked with input blocked. Decoded images, sockets, buffers, held input, and helper processes are released by their actual owner without duplicating the browser or throttling other viewers.

This task supplies measured A01/A03/A13/A22/A25 resource proof; current source hotspots are candidates until exercised.

## Evidence and starting points

- Baseline: `6f6222b74e4f552ce697e61364cf653f4b6be29f`; inline handoff explicitly does not claim native decode, five-minute reconnect, or performance evidence.
- Umbrella issue: https://github.com/nnexai/cockpit/issues/9. Security-negative proof is assigned to WEB-08.
- Read architecture transport/backpressure/visibility sections in `planning/inline-space-browser-2026-09-13/01-architecture-and-transport.md` and matrix rows A01, A03, A08, A13, A22, A25.
- Source starts: `src/app/browser/framePresenter.ts` parse/validate/`FramePresenter` decode, ACK/discard and cleanup; inspect post-decode error paths and ImageBitmap/object URL ownership.
- Source starts: `src/app/browser/BrowserPane.tsx` `inputJobsRef`, `enqueueInput`, `runInputJobs`, frame stream cleanup, and held-pointer release.
- Source starts: `src/client/browser.ts` browser frame envelope/packet release and `openBrowserViewStream`; `src/client/native.ts` `nativeBrowserViewSubscription` and channel cancellation.
- Source starts: `browser-runtime/browser-helper.mjs` frame history/retain/release/write/queue/cleanup/reset transport, screencast ACK, disconnect handling, and `detach`; owner launch/observer forwarding lives in `crates/cockpit-host/src/browser_helper.rs` and `crates/cockpit-core/src/browser.rs`.
- Coordinate `browser-ui`, `frame-presenter`, `client-transports`, and `host-streams` serially per `planning/stability-and-gitlab-2026-09-20/tasks.json`.

## Changes

1. Close decoded ImageBitmap/object URL resources on successful replacement, deliberate discard, validation failure, decode rejection, presenter exception, detach, and stream epoch change.
2. Enforce one active plus one pending frame per viewer and bounded socket/frame histories; ACK only after presentation or deliberate discard. Keep metadata/input releases progressing when a viewer is slow.
3. Bound boundary input jobs as well as coalesced move/wheel jobs. On overload or disconnect, disable input, release held keys/buttons, and show a local resource error without replaying uncertain commands.
4. Classify owner versus observer lifecycle: hidden views dispose high-volume resources, observer close does not stop the owner, owner shutdown closes only verified owned helper/browser, and helper restart reuses the verified browser/profile with a new epoch.
5. Make browser/native stream-open and reconnect cancellation safe; stale callbacks cannot publish to replacement views. Preserve metadata/snapshot availability independently of frame heartbeat.
6. Instrument the smallest useful counters for queue high-water, frame age/drop, decode/object/buffer counts, ACK timeout, helper restarts, and cleanup; add required performance/resource review.

## Non-goals

- No transport rewrite, base64 fallback, unbounded retry, or screenshot polling heartbeat.
- No browser security-policy expansion; WEB-08 owns hostile authorization/origin cases.
- No broad input behavior or annotation feature expansion; WEB-04/05 own those contracts.
- No fabricated FPS/CPU/memory target; record measured results and explicit unsupported states.

## Acceptance

1. A01/A03: simultaneous opens deduplicate to one owned browser/helper/profile; owner shutdown and observer close are scoped, and an unrelated sentinel browser survives.
2. A13: malformed/oversized JPEG, delayed decode, missing metadata, slow/no-ACK observer, helper crash, and reconnect keep bounded memory/queues, retain stale last image with input blocked, and never replay old input.
3. A22: Space rename/close/endpoint replacement and pane moves preserve authoritative membership; identity mismatch cannot adopt or kill another browser; `--current` follows fresh authority.
4. A25: repeated hide/show and reconnect release frame sockets/decoders and leave one helper per owned browser with no extension/external-window runtime dependency.
5. Five-minute animated and reconnect runs plateau in decoder/object/buffer counts and process memory; static idle does not create continuous capture/polling; slow observer does not throttle controller input or peers.
6. Browser gateway and Linux-native Tauri both show cleanup after stream cancellation, target switch, window close, and owner crash; queue counters and resource ledger reconcile to zero/baseline.

7. Separate capture/encoding cost from presentation/decode cost in measurements. A binary frame path can reduce copies while Chromium still spends time encoding; report those observations independently.
8. Exercise a slow observer while the controller clicks, types, and releases. The observer may disconnect for ACK timeout, but it must not delay controller input, metadata, or held-state cleanup.
9. Exercise helper crash during an active drag, dialog, resize, and pending capture. The replacement epoch must block stale commands and release held input without launching another browser.
10. After repeated hide/show and Space switches, compare live decoder/object counts, stream sockets, helper children, and browser association identities with baseline. Record any explicitly retained low-volume metadata.
11. Verify no stale image ACK, frame credit, or pending queue entry survives detach/rebind. Cleanup must be idempotent when close and crash notifications race.
12. Keep static idle capture quiet while metadata remains available; absence of changed JPEG bytes is not treated as a dead browser heartbeat.

Measurements must name the fixture dimensions, Chromium/WebKit build, client type, warm-up period, and clock relationship. Do not turn proposed budgets into a pass without observed intervals.

## Verification

The integration owner must run isolated browser and real Linux Tauri sessions with named profiles, sockets, helpers, displays, and sentinel resources. Exercise A01, A03, A13, A22, A25 plus malformed/slow/reconnect scenarios. Record frame age, dropped frames, queue high-water, decoded image/object/buffer counts, ACK/discard timing, process-tree CPU/PSS, helper/browser identities, and cleanup. Use real native WebKit decode and channels where claimed; a typecheck/build or source audit is not lifecycle proof.

The evidence record must include:

- active/pending frame counts, decode/discard/ACK cleanup counts, queue high-water, frame age, and stale-input state;
- malformed/oversized, slow observer, helper crash, reconnect, hide/show, and owner/observer close outcomes;
- five-minute fixture dimensions/build identity, decoder/object/buffer counts, CPU/PSS observations, and clock basis;
- browser/native process, socket, helper, profile, and sentinel cleanup.

Do not mark a bounded run from a single frame or a build result; record plateau and failure isolation over the stated scenarios.

Include a baseline and post-run resource snapshot, not only event counts, and explain any intentionally retained metadata or process.

The evidence must separate normal cancellation from crash recovery and show that an old epoch cannot publish after reconnect.

## Handoff

Return changed paths, ownership/cancellation rules, measurements, and unresolved resource risks. The integration owner must commit durable `runs/<run-id>/WEB-06.md` evidence and a real implementation commit after all locks settle. Do not update `tasks.json` in this file; obey `../ORCHESTRATOR.md` and serialize all listed lock groups.
