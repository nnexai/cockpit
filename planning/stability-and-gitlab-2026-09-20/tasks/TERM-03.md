# TERM-03 — Stabilize terminal scrolling focus and input

## Outcome
Cockpit terminal interaction matches Herdr for scrollback, tail-follow, output continuity, mouse modes, modifiers, prefix handling, clipboard, focus ownership, and pane hierarchy. Switching between two clients must not create duplicate input or reorder Herdr-owned Spaces, tabs, panes, or agents.

## Evidence and starting points
- Baseline/dependency/locks live in [../tasks.json](../tasks.json); this task follows SYNC-01 and owns App, TerminalPane, and Herdr adapter boundaries.
- Regression anchors are [GitHub #1](https://github.com/nnexai/cockpit/issues/1), [GitHub #4](https://github.com/nnexai/cockpit/issues/4), [GitHub #9](https://github.com/nnexai/cockpit/issues/9), and [GitHub #12](https://github.com/nnexai/cockpit/issues/12). Closed issues are historical anchors, not current-failure claims.
- `src/app/TerminalPane.tsx` owns xterm setup, frame writes, scroll/mouse forwarding, modified Enter, prefix/clipboard input, focus, and ownership.
- `src/app/App.tsx` owns selected hierarchy, pane projection, focus requests, and `orderAgentsByHerdrPriority`.
- `src/app/session/focusCoordinator.ts` and `sessionStore.ts` define focus epochs/tokens and stale-state transitions.
- `src/app/App.test.ts` and `src/app/TerminalPane.test.tsx` show existing modifier, agent-order, and clipboard contracts; preserve behavior rather than testing implementation trivia.
- `research/ui-design-direction.md`, `research/ui-implementation-constraints.md`, and `skill://cockpit-ui-parity` make Herdr authoritative for scroll position, focus, layout, and ownership.

## Changes
- Trace and repair only the terminal interaction paths that diverge from Herdr: scrollback/tail state, continuing output while scrolled, short and sustained wheel gestures, and returning to the tail without synthetic local authority.
- Preserve Herdr's terminal mouse mode negotiation and correctly forward text/application mouse modes, buttons, wheel, modifiers, and prefix keys.
- Preserve modified Enter, Unicode/IME-safe keyboard input, clipboard copy/paste, and the existing hold-until-control ownership behavior.
- Ensure a two-client focus handoff has one owner and one input route; stale client streams cannot reclaim control or duplicate commands.
- Exercise hierarchy/agent-order regressions across Space/tab/pane switches, splits, zoom/restore, and focus changes; fix projection logic only where observed parity fails.
- Do not invent a browser-side scroll authority or a second full-height scrollbar; keep Herdr as the source of truth.
- Add a parity review against a disposable Herdr TUI session and a terminal input/ownership review before integration.

## Non-goals
- No terminal graphics revival, protocol redesign, or alternate scroll model.
- No CSS-only hiding of scroll state or failure banners.
- No broad xterm replacement or unrelated pane renderer rewrite.
- No automatic replay of uncertain input after ownership loss.

## Acceptance
1. With continuing output, scroll up in a selected terminal, switch tabs/Spaces, return, and observe the Herdr-confirmed scrollback/tail state; no local reset or duplicate scrollbar appears.
2. Short wheel steps and sustained wheel gestures each arrive once, including terminal text/application mouse modes, button/modifier combinations, and release/cancel paths.
3. Keyboard modifiers, prefix commands, modified Enter, Unicode/IME input, copy, and paste match the Herdr TUI oracle; paste waits for the requested pane to own control and does not submit an extra Enter.
4. Two disposable clients alternately focus the same session: exactly one client owns input at a time, the losing client reports ownership honestly, and no duplicate bytes reach the terminal.
5. Space/tab/pane hierarchy, split/zoom state, focused pane, and agent ordering match Herdr after switching and recovery; no cached pane changes authoritative order.
6. Native Tauri and browser clients both preserve terminal output continuity and focus/error behavior after a detach and reattach.
7. The TUI oracle confirms scroll, focus, ownership, hierarchy, and agent order for each scenario; no invented authority is accepted as parity.

## Verification
The integration owner should use a uniquely named disposable Herdr session and one additional client/session identity for two-client tests. Capture terminal output sequences, scroll positions/tail status, input bytes and mouse reports, focus/ownership events, and hierarchy snapshots from browser and native surfaces. Repeat key actions directly in Herdr TUI and compare authoritative results. Historical issue captures and existing unit tests are regression anchors only.

## Handoff
Record the TUI comparison, browser/native evidence, input traces, scroll/tail observations, and cleanup of both clients. Link the durable evidence and real commit in the ledger. Any remaining divergence must name the authoritative Herdr behavior and remain unresolved rather than being papered over with a local scroll rule.
- Establish a baseline in Herdr before changing a terminal: scroll position while output is idle and while output continues, mouse mode, focus owner, and process identity.
- Repeat the baseline in the browser client and native client, recording the exact input bytes or mouse reports where instrumentation permits.
- Cover text mouse mode and application mouse mode, including wheel at the top/bottom boundary and a drag interrupted by tab or Space switching.
- Cover Shift/Ctrl/Alt/Meta combinations, prefix activation/cancellation, modified Enter, and key-up after an ownership loss.
- Include clipboard text with newlines, Unicode, and a paste attempt while control is pending; verify no implicit submit.
- Exercise the terminal after a full-frame reconnect and after a selected-tab reattach; output sequence must remain valid.
- Run the two-client case with both clients attempting focus and with one client disconnecting during an ownership handoff.
- Check the focused pane when a split is zoomed, restored, reordered, or selected from the agent list.
- Compare agent ordering after status changes and equal timestamps; the Herdr order is the oracle, not DOM insertion order.
- Record whether any discrepancy is caused by the client projection, adapter translation, or Herdr itself before editing.

The evidence should distinguish:
- repaired forwarding or focus code;
- Herdr-authoritative observations that were already correct;
- unresolved parity candidates awaiting a different platform or input device.

Avoid asserting that a local xterm scrollTop is authoritative. The task is complete only when the corresponding Herdr state and the rendered state agree.

- Establish a baseline in Herdr before changing a terminal: scroll position while output is idle and while output continues, mouse mode, focus owner, and process identity.
- Repeat the baseline in the browser client and native client, recording the exact input bytes or mouse reports where instrumentation permits.
- Cover text mouse mode and application mouse mode, including wheel at the top/bottom boundary and a drag interrupted by tab or Space switching.
- Cover Shift/Ctrl/Alt/Meta combinations, prefix activation/cancellation, modified Enter, and key-up after an ownership loss.
- Include clipboard text with newlines, Unicode, and a paste attempt while control is pending; verify no implicit submit.
- Exercise the terminal after a full-frame reconnect and after a selected-tab reattach; output sequence must remain valid.
- Run the two-client case with both clients attempting focus and with one client disconnecting during an ownership handoff.
- Check the focused pane when a split is zoomed, restored, reordered, or selected from the agent list.
- Compare agent ordering after status changes and equal timestamps; the Herdr order is the oracle, not DOM insertion order.
- Record whether any discrepancy is caused by the client projection, adapter translation, or Herdr itself before editing.

The evidence should distinguish:
- repaired forwarding or focus code;
- Herdr-authoritative observations that were already correct;
- unresolved parity candidates awaiting a different platform or input device.

Avoid asserting that a local xterm scrollTop is authoritative. The task is complete only when the corresponding Herdr state and the rendered state agree.

- Include both clients' endpoint/session identities and prove they were not the user's active session.
- Record exact modifiers and terminal mode for every disputed input sequence.
- Preserve a negative control showing Herdr rejects or transfers ownership as expected.
- The handoff must name any platform-specific input that remains unverified.
- A missing macOS/TUI fixture blocks only that acceptance criterion and must remain visible in the ledger.
