# TERM-02 — Attach terminals only for visible panes

## Outcome
Terminal renderers and terminal subscriptions follow selected-tab visibility while Herdr processes remain alive. Cached pane projections may support a flicker-free visual handoff, but a cached/non-selected pane must not run the terminal attach ladder or paint an error over the selected pane. Re-entry must attach the selected terminal normally.

## Evidence and starting points
- Baseline is `6f6222b74e4f552ce697e61364cf653f4b6be29f`; dependency and locks are in [../tasks.json](../tasks.json), including TERM-01 and `app-shell`/`terminal` ownership.
- Historical regression anchors: [GitHub #12](https://github.com/nnexai/cockpit/issues/12) and [GitHub #9](https://github.com/nnexai/cockpit/issues/9). Their closed/open state is not proof of current behavior.
- `src/app/App.tsx:819-891` computes `visiblePaneIds` but retains `projectionHistoryRef` projections; `renderPaneLayer` later mounts cached views.
- `src/app/TerminalPane.tsx:500-692` starts the attach ladder for each mounted terminal, retries `pane_not_visible`, and presents errors.
- `src/app/paneRenderers.ts` polls/inspects selected visible panes and is the visibility authority to preserve.
- `crates/cockpit-core/src/lib.rs` and `crates/cockpit-host/src/server.rs` enforce the server `pane_not_visible` gate. The server rejection is correct and must remain.
- `research/ui-design-direction.md`, `research/ui-implementation-constraints.md`, and `skill://cockpit-ui-parity` require visible-pane renderer/subscription lifetime and Herdr-owned hierarchy/focus.

## Changes
- Separate visual transition retention from live terminal attachment eligibility. A cached projection may remain available for handoff, but its PaneView must be non-attaching or detached until its tab is selected.
- Drive attach/detach from the selected visible pane identity and current session epoch; reattach after selecting a previously visited tab.
- Keep one active terminal stream per visible pane and retire its xterm/subscription deterministically when the pane leaves the selected tab.
- Preserve the sizing barrier and flicker-free incoming handoff; do not blank a healthy committed projection merely because a new pane is measuring.
- Keep the existing server visibility validation and `pane_not_visible` response unchanged. Do not turn repeated rejection into a success or suppress all terminal alerts with CSS.
- Retain focus/ownership state only when it belongs to the active visible pane; stale cached views must not reclaim focus or input.
- Add a focused UI/runtime review for lifecycle ownership because this change crosses App projection and TerminalPane attach effects.

## Non-goals
- No server exception for cached panes.
- No global alert suppression, CSS-only hiding, or retry-budget inflation.
- No rewrite of Herdr hierarchy, layout, or process lifetime.
- No requirement to eliminate every transition DOM node when it is needed for the measured handoff.

## Acceptance
1. In a disposable browser session with ten one-pane tabs, sixty rapid switches produce no terminal WebSocket attempt for a non-selected tab and no hidden-pane `role=alert` banner.
2. The selected tab has exactly its visible terminal stream(s), and the server still returns `pane_not_visible` if a deliberately issued hidden attach is attempted.
3. Switching to a previously visited tab reattaches its terminal, preserves the Herdr-selected hierarchy, and accepts input after the attach settles.
4. The same rapid-switch scenario through native Tauri channels has no hidden attach/error and leaves the selected terminal live.
5. DOM/renderer/stream counts remain bounded by visible panes plus any explicitly documented one-frame handoff; counts return to baseline after settling.
6. The existing flicker-free handoff remains observable while terminal output, focus, and ownership do not jump to a cached pane.
7. The TUI oracle shows the same selected Space/tab/pane and process continuity before and after browser/native switching.

## Verification
The integration owner should use uniquely named disposable browser and native sessions, capture WebSocket/channel requests, DOM terminal hosts, stream counts, and alerts during the 60-switch scenario, then repeat with a deliberate hidden-pane rejection to prove the server gate remains. Exercise scroll and output briefly while switching away and back. Compare hierarchy, focus, process identity, and pane order against a disposable Herdr TUI session. Historical #9/#12 captures are regression anchors, not fresh proof.

## Handoff
Provide durable run evidence for browser and native traces, resource counts, selected-pane screenshots, and the TUI comparison. Include the exact transition policy used for cached projections and cleanup of all disposable sessions. Link evidence and the real implementation commit in the ledger; source inspection alone cannot complete this task.
- Instrument or capture the terminal attach URL/channel request with its session, pane, tab, and visibility identity.
- Start with one tab, then add splits and additional tabs so both one-pane and multi-pane layouts are covered.
- During rapid switching, distinguish an intentionally retired active stream from an attach that was never allowed to start.
- Verify that a selected pane which is still measuring does not briefly attach a hidden projection as a workaround.
- Exercise a switch back while output is arriving; the newly visible terminal may receive a fresh full frame but must not receive a cached pane's input.
- Exercise ownership loss during the switch and confirm the cached pane cannot issue takeover or resize commands.
- Check browser DOM rects and mounted renderer identity after settling, not only the absence of a visible alert.
- Check native channel cancellation and host stream cancellation separately from the browser WebSocket trace.
- Deliberately force a `pane_not_visible` response through a controlled diagnostic path and preserve the error contract.
- If a transition retains a DOM host, record its documented lifetime and prove it has no live subscription.
- Keep frame sequence and stream cleanup evidence for at least one switch-away/switch-back cycle.

The evidence should distinguish:
- repaired projection/attachment behavior;
- server rejection observed as an expected negative control;
- any residual transition-only DOM retention that is not a live terminal;
- behavior not exercised because a native or browser fixture was unavailable.

Do not use `display:none`, opacity, or alert removal as evidence of detached terminal ownership.

- Include the selected tab and each attempted hidden pane in the capture, not just aggregate request totals.
- Record whether each cached host had an xterm instance, stream, subscription, focus listener, and visible geometry.
- Preserve a negative-control response proving that hidden attachment is rejected by the server.
- The handoff must identify any one-frame transition exception and its measured lifetime.
- A missing native fixture blocks the native criterion rather than reducing the task to browser proof.

- Preserve the request ordering around the final selected pane so a flicker-free handoff can be reviewed.
