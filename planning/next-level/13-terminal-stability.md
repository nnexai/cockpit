# Terminal stability and the custom protocol-22 decision

Status: reopened decision after the user's report of severe flickering when Kitty TGP images appear. Planning/diagnostic evidence only. No running server, installed binary, dependency, or application code was changed.

The user values Kitty graphics, but the custom protocol-22 build and resulting flicker have made the primary terminal experience worse. The current implementation is not an acceptable baseline to preserve blindly during cleanup. Terminal stability is the first delivery gate, ahead of broader refactoring and feature work.

## What is established

- The user reports heavy flickering once Kitty TGP images are present. The user clarified that the whole view briefly turns dark before being repainted several times per second, especially during agent code output or Codex `/pets`. It may happen on any update, including without images. The user also reports very poor scrolling. Its exact mix of flicker, viewport jumps, and latency remains to be classified. Native/browser comparison remains to be confirmed. This review has not reproduced that visual failure in the user's running session and does not claim its root cause is established.
- Commit `7e8fe25` changed the required protocol from 20 to 22, replaced substantial terminal transport code, added the shared client-shell endpoint, and removed the WebGL addon while adding graphics presentation. It changed 31 files. These changes need separate evaluation; "Kitty support" was not an isolated addon toggle.
- The previous Cockpit commit `582792e` is a comparison candidate, not a proven fully correct rollback target. Its adapter required protocol 20. The historical stable-installation research is not proof of the currently available stable Herdr release or its full capability set.
- Current `terminal_wire.rs:999` calls its update `send_pane_patch` but emits a complete text frame with `full: true` and then graphics. `TerminalPane.tsx:405` resets xterm for every full frame. `encode_graphics` at `terminal_wire.rs:1177` begins by deleting all images, then transmits/places the scene's images again. Those are confirmed code behaviors to include in the reproduction trace, not a completed attribution of the reported flicker.
- `TerminalPane` also schedules repeated image-canvas invalidation after graphics writes. That workaround must be measured alongside frame and image lifecycle, not treated as proof of compositor stability.
- `scripts/kitty_image_smoke.py` emits one static image. It can show whether an image appears, but does not verify stability while surrounding terminal content updates.

The diagnostic sequence is: establish a temporal reproduction, minimize it, compare one variable at a time, then select and verify a repair. The concrete gate below is self-contained.

## TERM-01: establish the visible regression and compare baselines

Perform in isolated sessions/build outputs, without replacing the user's active Herdr server or primary Cockpit installation.

1. Record exact Cockpit commit, Herdr executable path/hash, advertised version, protocol/schema, client-shell generation, renderer/addon versions, client type, display scaling, and the minimal image-producing program. A version string alone does not distinguish these Herdr builds.
2. Extend a disposable copy of the image smoke fixture with repeated text updates while one image remains unchanged. Exercise idle image, text updates, image replacement/removal, scrolling, resize, focus changes, and two panes. Include a text-only control using the same update rate. Scroll up, pause away from the live tail while output continues, scroll down, and return to the tail. Exercise short wheel steps and a sustained gesture, with and without images. Record intended versus observed viewport movement, blank frames, delayed input, and unexpected tail-following.
3. Capture successive displayed frames or a video that can detect temporary blank/replaced image regions. Test native WebKit and browser Chromium separately. A canvas pixel buffer alone may miss compositor flicker; a single final screenshot is not an acceptance test.
4. Replay the recorded surface/graphics stream through the real renderer when possible. Count full resets, unchanged image retransmissions, image deletions, and visible blank frames. Keep trace data bounded and omit actual user content by using the fixture.
5. Compare the current build with the pre-graphics Cockpit/compatible Herdr pair and a freshly verified stable Herdr pair. Do not combine a protocol-20 client with a protocol-22 server and call the resulting failure a renderer comparison. Record any capability that each pair lacks.

One runnable fixture must fail on the reported flicker before a repair is selected. The gate is no unintended intermediate dark/blank view during continuous text-only or image/text updates, stable intended image content, and predictable scrolling that preserves the selected viewport while new output arrives. Reproduce the agent-output and `/pets` patterns; do not reduce acceptance to "image appeared". Establish performance baselines from measured runs; do not invent universal frame-time targets.

## TERM-02: select a reliable default, preserve Kitty as a goal

| Option | When it is justified | Cost / limitation |
|---|---|---|
| Repair current protocol-22 presentation | Replay proves a bounded Cockpit-side fix restores stability | Still depends on a locally built/non-stable Herdr target until a supported release is verified |
| Restore a stable Herdr-compatible terminal path | The current path cannot meet the visible stability gate in a bounded repair | Terminal graphics may need to be unavailable/experimental temporarily; verify actual stable capabilities first |
| Isolate protocol-22 graphics as an explicit experiment | Stable daily use and Kitty development both need to continue | Two temporary paths require separate fixtures and explicit selection; avoid making permanent dual-protocol support a new product project |

My recommendation is to prioritize the proven stable daily-use path. Keep Kitty support as a planned capability, but require it to pass the temporal gate before it dictates the default server build. Do not assume disabling the image addon alone restores stability: the new transport still emits full redraws and must be tested without graphics.

The source inspection makes patch/image presentation a concrete rewrite candidate. If reproduction implicates that path, give one presenter ownership of stable image IDs/assets, placement changes, text patches, and resize. Avoid resetting unrelated panes on a tab update. Update only changed content and reserve full resets for initialization or actual resynchronization. Verify ordering and native compositor behavior before deciding whether a frame transaction or a different renderer is required. These are candidate design directions pending TERM-01, not diagnosed fixes.

Do not modify Herdr-server to rescue Cockpit behavior during this work. If a required graphic contract is unavailable in a stable release, expose that capability honestly. The Context GUI can still render its own Markdown/Mermaid/images independently of terminal TGP support, subject to the chosen server's pane capabilities.

## TERM-03: revise affected plans after the decision

FND-01 and PANE-01 must derive capabilities from the selected proven Herdr target. Re-probe plugin launch/detection, worktree operations, env handoff, and paste support; do not carry protocol-22 feasibility claims across a stable fallback without verification. Keep the no-IPC GUI-replacement design and same-tab paste behavior. Missing process inspection retains the explicit per-pane override already planned.

Update current-implementation records and historical transport notes when a target is selected. Avoid maintaining contradictory "stable" and "experimental" rules across documents. Record the default build and experimental features in one place, with a reproducible rollback procedure for future graphics changes.

## Gate and sequence

TERM-01 is first. A targeted TERM-02 repair/fallback decision follows the evidence. Complete the existing-code ordering/input/request-lifetime repairs in bounded increments, then run CLEAN-05 and the behavior-preserving cleanup against the reliable baseline. Provider downloads and the Context/Review plans remain in scope; they follow stability rather than extending a flickering daily tool.

Completion requires the original user reproduction plus the temporal fixture to pass, including multiple panes and image/text activity, with exact build identities recorded. The user then verifies the daily-use result. No static screenshot, helper-only test, CRAP score, or mutation score can substitute for this gate.

## Scrolling acceptance

Scrolling is core terminal behavior, not a later visual polish story. The fixture must distinguish deliberate follow-tail behavior from a user who has scrolled back. Incoming frames, image updates, focus changes, and adjacent-pane activity must not move that viewport unexpectedly. Confirm which scroll state Herdr owns and what Cockpit merely projects; do not create a competing local scrollback model to hide a transport reset. Measure input-to-visible-scroll delay and redraw stability on the exact supported build pair. Test scroll after image deletion and renderer fallback as well as while images are present.
