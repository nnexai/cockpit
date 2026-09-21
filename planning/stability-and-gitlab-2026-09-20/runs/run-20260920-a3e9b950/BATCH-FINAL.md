# Consolidated stability delivery — 2026-09-21

Baseline: `163ee85fba8c883f80b5b24871de84bd210a7823`. Implementation/evidence commit is recorded in `tasks.json` by the subsequent checkpoint. This is the user's OBS-015 combined implementation/review/verification batch. The final user direction was to end within ten minutes; no additional broad matrix was launched. **This is not full campaign acceptance.** Known failures below remain open rather than being relabeled passes.

## Delivered implementation

- Browser focus/handoff, retained drafts and pending annotation intent, durable feedback operation identity, explicit rejected versus unknown outcomes, local editor event ownership, and bounded input queues.
- Capture-time geometry checks, bitmap/URL release, bounded decode/frame credits, independent observers, exact frame ACK/discard, controller/tab/dialog authority, grant expiry and scoped helper retirement.
- Browser/native cancellation, late-open cleanup, dependency diagnostics, private helper transport boundaries, and durable JSON publication.
- Context/Review scroll and source/diff continuity, compact resource presentation, terminal frame-grid handling, and conservative comment-paste acknowledgement.
- Final Main repairs: pointer down/up click count is at least one; note/editor/blocker/recovery controls do not bubble into remote pointer capture; alert acceptance does not pass null to Playwright; forced pointer release cannot synthesize a click; Retry awaits association reconnection and actually reopens the failed view stream.

## Executed checks

See `final-checks.md` for exact package commands and earlier integration failures. Frontend build and host/native binaries passed. The integrated frontend wave reported 158 tests; final affected App suites (`bun run test src/app/App.test.ts src/app/App.integration.test.tsx`) passed 58/58 after the Retry repair. The initially mistyped `.test.tsx` filter found no tests; it was corrected, not counted as a pass. Latest frontend bundle: `index-CH8RRJN1.js` (326 modules).

Rust: core 150, providers 28, host 12, Herdr 92, protocol 13: **295 passing tests**. These totals describe executed suites, not every original acceptance matrix. A temporary native-coordinate precision hypothesis test passed without a production change and was removed; no precision fix is claimed.

## Actual browser proof — Main

Authority: session `csg-a3e9b950`, gateway `127.0.0.1:37619`, fixture `127.0.0.1:37629/?case=web-final`, Space w3/tab w3:t2; no user-active Herdr session used.

1. Physical click regression: fixture pointer down/up previously produced no click; after click-count repair the hit counter advanced. A later 100-click run advanced it from 1 to 101.
2. Physical region draw -> note editor -> `MAIN-SAVED-REGION` -> physical Save. Before repair the surface captured the editor's pointer and Save never upserted; afterward the editor closed and the authoritative draft contained the exact comment (revision 3). `main-browser-note-fixed.png`.
3. Actual PNG capture `5e9b6fd0-8a45-48cd-864b-e8ae6c09d0c3`, SHA256 `248f43ac9f31f3915106616e0cfbf28c4b3e9357eb8743a6294eb1d024c88f49`: `final-web-capture.png`. Main visually inspected page pixels, region and exact note, without application chrome.
4. No eligible agent: operation `browser-feedback-5e9b6fd0-8a45-48cd-864b-e8ae6c09d0c3` was definitively rejected, target null, acknowledged IDs empty, pending count 1. UI reported rejection and retained note/marks/capture. This does not prove successful agent paste.
5. Animated fixture produced four different presented-canvas signatures and alternating header pixels. Animation then stopped.
6. Following owned gateway restart, old Retry reconnected only the association and left the stream failed. The repaired Retry returned `/browser/view/open` 200 and visibly restored `Live browser view`; older drafts remained listed. Fresh fixture navigation succeeded.
7. Physical page Alert -> visible `alert: Owned alert sentinel` -> physical Accept -> blocker disappeared and fixture reported `alert-dismissed: 1`. `main-browser-recovery-dialog.png` was visually inspected.
8. Actual page wheel moved remote scrollY 0 -> 280 -> 440 while host scrollY stayed 0. **Subsequent nested-wheel input stalled behind `Showing last confirmed frame`; no nested-scroll event was observed. This remains unresolved under WEB-04/WEB-06.**

Performance: `main-browser-latency.json`, 100 physical clicks, same monotonic automation clock until changed presented canvas pixels, no exclusions. p50 49.486 ms, p95 50.46987 ms, maximum 333.416 ms. Includes automation overhead. This is not the five-minute resource plateau, whole-app sustained performance, or full PERF-01 acceptance.

## Delegated real-surface and boundary proof

- `final-browser-surface.json`: terminal marker survives zoom/restore; Review selection/source-diff scroll continuity; resource issue/MR rendering; typing and resize observations. Its original click/Save/recovery failures are superseded only by the precise Main checks above. Originally claimed nonexistent screenshots were removed; do not infer screenshots from those historical names.
- `final-protocol-boundaries.json`: exact controller ACK/discard, invalid Host/Origin close 1008, unauthenticated timeout, observer mutation denial, alert acceptance, grant expiry/retirement and forced pointer release without click. Corrected continuously ACKing controller progressed through sequences 1,2,3,5,6,7 while observer withheld credit. Earlier slow-observer failures were harness ACK errors, not concealed product passes.
- `final-native-surface.json`: actual isolated Linux Tauri startup, existing-directory setup, Herdr terminal marker/zoom state, native inline frame/channel and hide/show observations. `native-replay-fresh-frame.png` contains real native canvas pixels. Native OS coordinate injection produced no fixture event; WebDriver actions were unsupported. Synthetic pointer capture requires instrumentation and is explicitly **not physical-input proof**.
- Native DOM-only region replay persisted an annotation, but UI reported `Annotation save was not acknowledged; retry it before closing`; note/capture completion remained unavailable. **Native acknowledgement is unresolved under WEB-05/NATIVE-02.** IPC instrumentation was unavailable because Tauri invoke/ipc descriptors are non-writable (`native-ipc-annotation-probe.json`). No speculative numeric or equality change was shipped.
- GitLab resource/import evidence already exists in `GLAB-resource-ui-evidence.json/png`. Final workflow evidence is recorded separately when returned. An isolated workflow gateway initially inherited the private compositor D-Bus address, preventing CLI keyring access; Main corrected only its D-Bus credential-service routing while retaining owned HOME/config/state. No token was read or copied.

## Coverage and issue disposition

Original 25 required task identities remain. OBS-014 permits targeted checks, not invented passes. SYNC/terminal/view/source/browser/security improvements are delivered; their task records point here. WEB-04/06 scroll recovery, WEB-05/native annotation acknowledgement, successful same-tab agent paste, the unexecuted broader workflow/security/input matrices and sustained PERF criteria are explicitly not all accepted. ACCEPT-01 therefore remains incomplete.

A01–A25 remain owned as mapped in ACCEPTANCE.md; A23 stays excluded. This batch is not a blanket A01–A25 pass. #6 remains partial: GitLab issue/MR implemented, GitHub PR/Jira deferred. #3 concerns the current inline implementation, not a resurrected extension. #9–#12 receive the browser/terminal/cancellation repairs but are not claimed fully closed while the failures above remain. No remote issue was automatically closed from partial evidence.

macOS remains **user verification—not executed**, per OBS-013; `MACOS-HANDOFF.md` retains the original platform checks. No emulation or substitute macOS claim.

## Safety and cleanup

All runtime work used named disposable authorities under `/tmp/csg-a3e9b950`. Final cleanup stops owned app/gateway/helper/fixture/Herdr/display resources, preserves evidence/profiles/owned worktrees for diagnosis, and performs no remote branch deletion/merge/approval. Owned GitLab issue 2/MR1 and their dedicated branch remain available for the macOS handoff. User explicitly authorized retaining the six OBS-008 HOME files unchanged; no speculative restoration or further HOME write.

The subsequent checkpoint records final service cleanup and commit identity. The umbrella goal is not marked complete while the named failures remain.
