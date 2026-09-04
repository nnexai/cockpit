# Stable Herdr migration and terminal stability

Decision: the user chose to give up the custom protocol-22 path for now, provided the work remains committed, and explicitly wants mouse click handling retained. Stable Herdr is the default implementation target. Protocol-22/TGP work is parked in Git, not a competing default that the orchestrator may select silently.

This document plans the migration. No installed binary, running server, dependency, or application code is changed by this planning task. Execution order is [15](15-execution-timeline.md), verification is [16](16-verification-goals.md), and current-code repairs are [14](14-existing-code-repairs.md).

## Preserved work and useful comparison points

- `7e8fe25546ce5fa9364cab100522af6d63343e4a` preserves the protocol-22/Kitty implementation. It changed 31 files, including transport, schema, renderer/addons, and tests. Confirm that commit remains reachable before migration.
- `582792e` is the immediate pre-graphics comparison candidate. Its adapter required protocol 20. It is not a proven complete rollback target, and historical research is not proof of today's published stable Herdr capabilities.
- `34459ab` introduced terminal input parity and structured mouse/scroll handling before protocol 22. The `582792e` terminal wire still contains `Mouse`, `AttachScroll`, and mouse-capture handling, and core advertises terminal mouse input. This is concrete recovery material, not yet a runtime guarantee.
- Preserve the whole current planning/history chain. Port selected code or apply reviewed patches; do not reset the repository, blindly revert the large graphics commit, or undo unrelated input improvements.

## Failure to reproduce and measure

The user reports repeated whole-view dark frames during redraw, several times per second during active agent code output or Codex `/pets`. Kitty images make it prominent, but text-only updates may trigger it too. Scrolling is also very poor; its mix of flicker, viewport jumps, and latency remains to be classified.

Current code behavior is known: `send_pane_patch` emits a full text frame, `TerminalPane` resets xterm for full frames, and graphics encoding deletes and retransmits the image scene. The frontend also schedules canvas invalidation. These observations identify paths for tracing; this planning review has not reproduced the user's visual failure or established its sole cause. The existing one-image smoke script cannot prove redraw/scroll stability.

## BOOT-01: restore stable protocol compatibility before runtime smoke

This is the first implementation step after read-only inventory. The user will downgrade the Herdr `default` session to stable before the orchestrator starts. The orchestrator itself runs inside that session. Treat it as protected infrastructure: no restart, shutdown, upgrade/downgrade, session close, focus/layout mutation, or test input against `default`. Read-only version/schema inventory is permitted.

The current Cockpit frontend/host expects protocol 22 and will initially reject the stable server. Do not make a successful current-frontend smoke or a live protocol-22 reproduction a prerequisite for this bootstrap.

1. Record the running stable binary/server identity and schema using bounded read-only commands. Validate explicit executable and session selection; abort any test command that resolves to `default` or lacks a run-owned session ID. Do not assume the server's advertised version alone identifies its protocol.
2. Inspect preserved pre-22 adapter/renderer code and compare it to the installed stable schema. Freeze the minimum session snapshot/event, attach/input/resize/scroll/mouse contract needed to display a terminal. Keep the current repository and protocol-22 commit intact.
3. Adapt the Cockpit compatibility gate and actual transport/decoder together. Changing the expected protocol constant without fixing the wire path is insufficient. Reconcile generated DTO/parser changes and native/browser host integration where required. Preserve newer mouse/keyboard fixes deliberately.
4. Compile and run fixture checks before launching the candidate. Create a disposable named session using the already installed stable binary, with explicit isolated test configuration where supported. Never start another server for `default` or replace a binary it depends on.
5. Launch the patched Cockpit against that disposable session and verify both native and browser show a terminal, receive a snapshot, and deliver a harmless input-capture event to the correct pane. This is the bootstrap gate G00B, not final scrolling/mouse/stability acceptance.
6. Commit the bootstrap with exact target identity and protected-session guard tests. Then run TERM-01/02 temporal, mouse, and scrolling work through the compatible frontend.

If the installed default session has not actually been downgraded, record the mismatch and use only a separately launched stable test session if available. Never repair that environmental precondition by downgrading or restarting the orchestrator's own session. Ask only if no safe independent setup can proceed.

Old protocol-22 behavior may be replayed from fixtures or compared in an isolated process if safe and useful. Recreating the old live server is optional diagnostic evidence and cannot block fixing the known protocol mismatch.

## TERM-01: inventory and temporal baseline

After BOOT-01/G00B. Owner: Terra for actual runtime/replay, Luna for bounded fixture/report work. Astra owns stable-target capability decisions and evidence assessment.

1. Record original Cockpit commit and Herdr executable/hash, version/protocol/schema/client-shell identity, addon versions, native/browser runtime, scale, and test session. Use isolated sessions and explicit binaries; never replace the user's current server to obtain a comparison.
2. Build a fixture with constant text output, one unchanged image plus text output, image removal, and repeated screen updates resembling `/pets`. Include text-only runs. Capture successive displayed frames or video and enough trace data to correlate reset/write/graphics/scroll events.
3. Scroll up and pause while output continues, issue short and sustained wheel gestures, then return to the tail. Exercise focus changes, two panes, resize, and a graphical-pane/terminal transition when available. Record viewport jumps, intermediate dark frames, delayed response, and unintended tail-following.
4. Establish a red-capable signal for the old path. Use a negative control that deliberately blanks a fixture frame to prove the temporal detector notices the symptom. Canvas buffer contents alone may miss compositor flicker; use actual presented-frame evidence as well.
5. Probe the currently available official stable release through its official artifacts/source and installed schema. Use the user-installed stable binary when verified. Obtain any additional comparison tool only under a run-owned path; never replace the active binary. Do not assume stable means protocol 20 merely because the historical baseline used it.

Exit: exact available build identities, a reproducible workload, a working temporal failure detector, and stable-target capability inventory. A known injected-blank negative control proves detector sensitivity when a safe old live build is unavailable. A failed attempt to reproduce is recorded honestly; it does not establish the old renderer was sound. Continue the selected stable migration, but keep final visual verification incomplete until the relevant workload is demonstrably tested.

## TERM-02: migrate to the stable terminal path and preserve mouse input

Owner: one Terra implementation lane across the terminal adapter/presenter; Luna may supply fixtures in exclusive files. Serialize REPAIR-03 attachment/registry edits with this work.

1. Freeze a compatibility matrix for the selected stable server: session snapshots/events, focus/layout, terminal attach/input/resize/scroll, mouse capture/events, pane process inspection, plugin launch, worktree lifecycle, and byte-paste APIs. Classify each as verified, absent, or unknown. Keep required terminal/input capabilities blocking.
2. Recover the stable-compatible transport from reviewed pre-22 code where appropriate. Keep explicit protocol/schema validation and exact target identity. Reconcile newer error/validation/input fixes into the recovered code instead of copying whole files without review.
3. Remove the custom protocol-22 requirement from the active default. Park unsupported terminal graphics explicitly. Retain Kitty keyboard support if verified independently; graphics and keyboard are separate capabilities. Do not build permanent dual-protocol infrastructure solely to keep the parked experiment loaded.
4. Preserve mouse click-to-focus and input routing. Test a click on an inactive pane, the Herdr-confirmed focus result, application-mode pointer coordinates/button/modifiers, drag/release where supported by the existing contract, and wheel/page scrolling. No event may land in the previously focused pane or execute twice. Native/browser capability parity must be honest; disabling mouse and calling migration complete is not acceptable.
5. Preserve ordinary keyboard input and the established modified-Enter behavior, and fix workbench-prefix consumption through REPAIR-02. Terminal text updates must not require clearing unrelated panes or discarding the user-selected scroll position. Honor the stable server's scroll authority rather than creating a competing local buffer model.
6. Restore the appropriate verified renderer/addon configuration for this path. Do not assume turning off the image addon fixes the new transport's full redraw behavior. Remove unused graphics-reset/compositor workarounds from the default path only after confirming their responsibilities are no longer needed.
7. Ensure image-producing applications still leave a usable stable text terminal when TGP is unavailable. Report terminal graphics unavailable through capability/UI behavior where relevant; do not display raw binary payloads or silently submit terminal responses as user commands.
8. Build a launchable stable candidate with an explicit server executable/configuration path. Keep the user's current installation/session untouched. Document how to select the candidate and how to recover the archived protocol-22 work from Git later.

Exit: G01 passes for the selected stable build in native and browser, including mouse/focus and scrolling. Protocol-22/TGP is explicitly parked, and the migration diff preserves unrelated application/planning work. A stable API limitation is a blocker to the affected promised behavior, not permission to fake it through undocumented server changes.

## TERM-03: revalidate downstream feature contracts

Owner: Astra integrates one capability/interface update; Luna runs bounded probes after Terra establishes the stable runtime.

Re-probe plugin launch/detection, worktree create/open/remove, env handoff, and same-tab paste against the selected stable target. Protocol-22 research remains historical evidence for that build only. Where process inspection is unavailable, retain explicit per-pane renderer selection and known-launch evidence without extension IPC. If plugin launch or paste semantics differ, adapt the Cockpit interface and fixtures before dependent features begin.

The Context GUI's Markdown/Mermaid/images remain required and are independent of terminal TGP support. Full local graphical review remains required. Keep Herdr authoritative for real panes and layout; no Herdr-server modifications are authorized by this plan.

Update current implementation records, schema fixtures, generated DTOs where necessary, startup capability messaging, and the real-runtime scripts. A `version` string alone is insufficient to distinguish the previously custom build from the selected stable binary.

Exit: G04 probes use the selected stable target and all affected future stories reference the corrected capability contract. Later work does not silently reintroduce the protocol-22 requirement.

## Stability and scrolling acceptance

No unintended intermediate dark/blank view during continuous text-only updates. Maintain stable text when an application attempts unsupported image output. When graphics are explicitly parked, the gate does not require TGP image display, but does require usable text/input/scroll behavior.

Scrolling is core terminal behavior. Deliberate follow-tail and a user who scrolled back must be distinct. Incoming frames, adjacent-pane activity, focus changes, and image-producing output must not move the scrolled-back viewport unexpectedly. Measure input-to-visible-scroll delay and verify final viewport position, with both short wheel steps and sustained gestures. Pass criteria and artifact contracts are specified in G01.

The earlier request for a human visual checkpoint is replaced by automated temporal/native/browser evidence for this autonomous run. If a human-only condition remains, record it as incomplete instead of inventing approval. The required selected scope can finish only when its actual gates pass.
