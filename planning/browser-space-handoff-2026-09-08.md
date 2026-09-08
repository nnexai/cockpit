# Space browser integration handoff

## Continuation delivery

The continuation fixes extension sender authentication, stale worker loading, local draft recovery, bounded startup, durable saved-ID filtering, capture tab-switch detection, pending-capture capacity, destination revalidation, owner readiness, and explicit current-endpoint selection.

Extension bundles now use a unique worker script URL. Loading the new bundle preserves extension storage without uninstalling it. Pairing candidates become authoritative only after successful loading. The user's actual browser was repaired without closing its tabs: its popup reported Connected and Saved 1 annotation, and all 13 pre-existing annotations survived.

Geometry validation now compares bounded annotation anchors instead of rejecting every page mutation. The disposable heartbeat fixture failed before the repair and captured successfully afterward. A genuinely moved element exposed **Capture anyway (as shown)**, which saved two annotations. Wrong-tab, navigation, viewport, and offscreen protections remain.

Verification recorded during the continuation:

- Popup closure and browser reopen retained drafts; stale documents appeared in recovery.
- Eight concurrent draft writes survived; a ninth distinct draft failed explicitly.
- Injected pixel failure retained drafts; injected submission failure retained pixels and retried without recapture.
- Frontend build and 40 App/client integration tests passed.
- Six browser/feedback Rust tests passed, including pending capacity and unknown receipt retention; host/Herdr suites passed.
- The broad core run passed 101 tests and failed the unrelated execution-lease exclusivity test. That test passed in isolation.
- Both native and CLI binaries built; the installed native launcher was updated.

The detailed historical matrix below is not an assertion that every cross-platform and failure-injection scenario was exercised. Native image/send permutations, modal top-layer behavior, every ownership/crash combination, and the complete retention/failure matrix still require explicit evidence before claiming exhaustive plan acceptance. Preserve the user-edited integration plan.

## Checkpoint

Implementation checkpoint: `be68fc8` — `feat: checkpoint Space browser capture and inline feedback`.
Earlier tooling evidence: `d51d864`.

The user requested this handoff at a stable checkpoint. **The overall browser integration is not complete.** Resume against [the integration plan](browser-space-integration-2026-09-08.md), especially its acceptance section. That plan has pre-existing user edits deliberately left unstaged; preserve them.

The checkpoint contains the production lifecycle, extension, feedback store, CLI, web/native transports, and interaction polish. The working capture → fetch/read → exact acknowledgement and capture → direct paste paths have real evidence. Failure/recovery coverage and some lifecycle acceptance remain open below. This is not a claim that all 6,519 added lines satisfy the full plan.

## User direction to preserve

- The capture/send summary is useful; keep it.
- Annotation text belongs inline beside the mark. Keep the on-page palette for tools/actions, not a separate text form.
- Freehand has a thicker, visible in-progress stroke. It starts without text; `+ Text` adds an optional inline comment. Existing comment text is clickable for editing.
- Keep the redundant browser Refresh/status/Feedback strip out of the Cockpit sidebar. Browser actions are in Commands and the selected Space context menu. The feedback overview retains its own useful browser state.
- Acknowledge means handled without sending. Handled cards leave the pending overview; images survive their retention grace period.
- Avoid another implementation-worker correction loop. The parent integrated and repaired the checkpoint directly.
- Only integration-test agents launched through `omp`/`codex` must use `openai-codex/gpt-5.6-luna`; that restriction does not apply to implementation models. The existing disposable test agent is Luna. If changing its model, use `/switch`, not `/model`.
- Production remains independent of X11. Linux/X11 was the isolated harness; Wayland needs the user's machine and macOS remains untested.

## Verified behavior

### Browser and CLI

- Installed Playwright CLI 0.1.5 launches its default Chrome 152.0.7977.82 with a dedicated profile and named session. No explicit Chromium executable is required.
- Production loads the unpacked extension through the installed CLI's `run-code` and Chrome's `Extensions.loadUnpacked`, rather than deprecated Chrome load flags.
- The already-running Luna pane moved from `w2:p1` to `w3:p2`. Its subsequent `cockpit browser feedback --current` resolved w3 without restarting or changing its environment.
- The CLI parser was repaired: `--current` conflicts with `--space`, not with the explicit Herdr session flag.
- Earlier lifecycle work exercised GUI/agent open, preservation of existing page state, scoped close, multi-client ownership, and normal native owning-window closure. The current full acceptance matrix still needs reconciliation; do not infer the remaining rows passed.

### Capture and consumption

- Drew on the real live fixture; drawing did not activate its payment button.
- Edited text inline beside a freehand mark; saved through the on-page palette.
- Exercised inline element and region comments, edited/removal gestures, then saved a freehand-only capture. The saved batch contained only the remaining freehand mark.
- Read actual saved PNGs. They include the page, thicker drawing and optional comment; palette, editor, `+ Text`, and browser chrome are excluded.
- A drawing-only PNG had no fabricated `freehand annotation` label.
- Luna fetched the first capture twice, opened its PNG with its real read tool, and correctly described “Confirm payment” and the user's comment.
- After a newer capture was saved, Luna acknowledged only the first ID. Repeating that acknowledgement changed nothing; the newer ID remained pending.
- Luna used ordinary Playwright CLI against the returned named session to change the live button's opacity to 0.65 and transform to scale(0.92), without navigation/reload.
- Closed-browser feedback remained readable. Acknowledged images remained on disk immediately after acknowledgement.
- In the real web UI, acknowledgement removed the pending card and displayed “No pending browser annotations.” The CLI then reported zero pending; the image still existed.
- A fresh two-capture batch was sent with the overview's `Send to agent` action. It selected the active tab's Luna pane without a picker. The real OMP input showed a 4,768-character pasted attachment, **not a submitted turn**. CLI pending count became zero.
- The sidebar strip is absent. Open overview refresh no longer clears/refetches already loaded image data or visibly toggles busy state on every background refresh.

### Native checkpoint

The final native binary launched in disposable Xvfb `:199`, rendered w3, opened Commands, and opened Browser feedback through the real Tauri transport. It showed the shared zero-pending state and the open browser. This proves native startup/placement/feedback lookup, not every native image/send/failure path.

### Build and targeted checks

- `bun run build`: passed; existing Vite large-chunk warnings only.
- `cargo build -p cockpit-host --bin cockpit`: passed.
- `cargo build -p cockpit-tauri`: passed.
- `bun run test src/app/App.integration.test.tsx src/client/client.test.ts`: 40 passed.
- `cargo test -p cockpit-core browser`: 5 passed.
- `cargo fmt --all`: completed.
- `git diff --check`: passed before the checkpoint commit.

`unknown_delivery_survives_acknowledgement_and_retention` first failed because pruning deleted OutcomeUnknown receipts. The repair limits timed receipt pruning to Accepted/Rejected; the regression now passes. It verifies receipt retention, not the complete unknown-paste transport scenario or image expiry.

## Resume here: remaining work

### 1. Finish draft recovery before broadening the UI

`browser-extension/background.js` retains drafts, but the popup does not expose an explicit stale-document recovery view. Implement that missing plan requirement without reattaching old marks to a new document or pretending lost pixels can be recaptured.

Concrete inspection findings still open:

- `saveDraft` uses `next.slice(0, MAX_DRAFTS)` (8), silently evicting older unfinished drafts. Replace silent loss with a bounded, explicit recovery/capacity policy.
- Draft persistence uses independent read/modify/write operations. Review races between rapid editing, capture completion, navigation invalidation and worker suspension.
- `tabs.onUpdated` marks drafts stale for status/URL changes; verify actual document identity, including same-document URL changes, rather than relying on update timing.
- Capture checks document identity before/after pixels, but still needs the active-tab-switch race exercised: the original document may remain alive while a different tab becomes the visible screenshot target.
- `loadPairing` caches credentials in worker memory. Verify same-source browser reopen and owner endpoint/token rotation, not just a source-version update.

Completion evidence: popup closure preserves edits; duplicate URLs remain document-bound; reload/navigation before capture exposes recoverable stale drafts; failed pixel capture preserves edits; failed submission can retry already captured pixels after navigation or tab closure. No silent retargeting.

### 2. Finish alignment and interaction acceptance

Run scroll, resize, browser zoom, and page-layout-change scenarios against the actual overlay. Current code requires review after movement and rejects off-viewport geometry, but those branches have not all been exercised. Verify explicit Browse mode passes real page clicks through, and verify modal/dialog interactions. A native top-layer dialog may cover the ordinary overlay; this is an unverified risk, not a completed repair.

Keep the user's polished palette/inline text flow. Inspect placement at the normal and documented minimum sizes. Do not add another permanent Cockpit panel.

### 3. Finish delivery and retention failures

Happy-path direct delivery and shared pending state passed. Remaining:

- Standalone `Send browser context` with no selected annotation IDs.
- No eligible agent in the active tab; no fallback to another tab.
- Wrong active Space and destination changes during delivery.
- Explicit paste rejection and unknown dispatch outcome: retained feedback/artifacts, explicit duplicate-risk retry, no automatic second paste.
- Unknown receipt survives a later real CLI acknowledgement and subsequent pruning. The new regression covers only the store boundary.
- Acknowledged image expiry and automatic pruning after a short configured grace period; currently immediate survival was observed, not expiry.
- Bounded capture/store failures and recoverability.

Current defaults: one-hour retention, 256 MiB store, 4 MiB PNG, 6 MiB request, 64 annotations/capture, 8,192 points, 64 pending captures. Check implementation/config before changing these limits.

### 4. Reconcile remaining lifecycle acceptance

Use the plan's full lifecycle row, recording which earlier evidence remains applicable. Explicitly finish simultaneous-open deduplication, rename/label refresh, missing prerequisites/browser, crash recovery, Space closure, and refusal to close unrelated sessions. Repeat final ownership checks with current binaries where necessary.

Native final verification still needs image rendering and applicable send/failure paths. Earlier normal native WM_DELETE_WINDOW closure exited 0, closed its owned named browser and preserved its profile; a later native observer should not close the gateway-owned browser. Wayland/macOS claims remain bounded to available evidence.

### 5. Finish delivery documentation and cleanup

Update the existing tooling evidence JSON with default Chrome 152 and the verified CDP extension-load/source-version behavior. Update existing user-facing documentation after final acceptance. Preserve the integration plan's user edits. Commit bounded verified remaining increments; only mark the overall goal complete once every named acceptance criterion is accounted for.

## Implementation map and important repairs

- `crates/cockpit-core/src/browser.rs`: association, ownership, process evidence, lifecycle.
- `browser/extension.rs`: embedded extension assets, pairing, authenticated capture and feedback lookup.
- `browser/delivery.rs`: selected IDs, authoritative active-Space/tab recipient selection, bracketed paste and receipts.
- `browser_feedback.rs`: immutable artifacts, pending IDs, acknowledgement, expiry, limits, receipt storage.
- `crates/cockpit-herdr/src/cli.rs`: authoritative browser snapshot and paste adapter.
- `crates/cockpit-host/src/browser_runtime.rs`: owner discovery/routing; `browser_annotations.rs`: narrow authenticated annotation HTTP interface.
- `crates/cockpit-host/src/bin/cockpit.rs`: lifecycle/feedback CLI and authoritative `--current` resolution.
- `crates/cockpit-protocol/src/browser*.rs`, generated TypeScript, `src/client/*`, and `src-tauri/src/lib.rs`: protocol/transports.
- `browser-extension/*`: toolbar, closed-shadow live overlay, draft/capture persistence.
- `src/app/App.tsx`, `src/app/styles.css`: Commands/Space actions, feedback summary and inline outcomes.

Two easy-to-regress repairs:

1. Chromium ran an old background worker after a new unpacked path with the same manifest version. The live worker lacked `capture-page` even though the generated file contained it. Bundled assets now determine a numeric manifest version via SHA-256. Reusing the profile then loaded the current worker and the palette save succeeded. A manual `chrome.runtime.reload()` experiment was unreliable; it is **not** the production solution.
2. Content-message authentication no longer depends on the worker's in-memory `documentByTab` map surviving MV3 suspension. It checks Chrome-provided sender identity/document ID; palette capture also probes the currently active document.

The checkpoint restored existing native `cockpit_comments_paste_send` and `cockpit_comments_paste_mark_pasted` registrations accidentally lost during integration. File/Review recipient selection is intentionally unchanged.

## Disposable runtime and evidence

Resources were left available for continuation, not installed into the user's active session. Recheck live state before acting; hub process persistence is not guaranteed across harness shutdown.

| Resource | Address |
| --- | --- |
| Root | `/tmp/cb0908-y9mvnus6` |
| Herdr session | `browser0908-y9mvnus6` |
| Herdr socket | `/tmp/cb0908-y9mvnus6/config/herdr/sessions/browser0908-y9mvnus6/herdr.sock` |
| Cockpit config | `/tmp/cb0908-y9mvnus6/cockpit.toml` |
| Cockpit state | `/tmp/cb0908-y9mvnus6/cockpit-state` |
| Fixture | `http://127.0.0.1:48571/lifecycle.html` |
| Gateway owner | `http://127.0.0.1:48572` |
| Old observer | `http://127.0.0.1:48573` — restart before relying on its protocol |
| Native assets | `http://127.0.0.1:5173`, serving repository `dist` |
| Isolated display | Xvfb `:199`, 1440×1000 |
| Current Space | w3, “Browser parallel proof”, tab w3:t1 |
| Luna pane | w3:p2; w3:p1 is an ordinary shell |
| Browser association | `cf585867f9b1b29182f459e6` |
| Playwright session | `cockpit-cf585867f9b1b29182f459e6` |
| Playwright working directory | `/tmp/cb0908-y9mvnus6/cockpit-state/browser/workspaces/cf585867f9b1b29182f459e6` |
| Retained profile | `/tmp/cb0908-y9mvnus6/cockpit-state/browser/profiles/cf585867f9b1b29182f459e6` |

Hub names: `browser-herdr-proof`, `browser-fixture`, `browser-proof-display`, `browser-lifecycle-gateway`, `browser-lifecycle-observer`, `browser-lifecycle-native`, `browser-native-assets`. Native PID at checkpoint was 3375422; gateway PID was 3308611. These are evidence, not authority to kill processes later. The gateway predates the final receipt-pruning repair; restart it before testing that repair.

The Luna pane currently contains the direct-send batch in its **unsubmitted input**. Preserve that evidence or clear the input deliberately before sending another test prompt. Do not accidentally submit it with `pane run`.

An independently launched sentinel named `cockpit-000000000000000000000001` used `/tmp/cb0908-y9mvnus6/launch-probe`; verify its current status before cleanup. Close only explicitly owned named sessions; never use Playwright global close-all/kill-all.

Useful external CLI lookup (inside the agent, `--current` uses inherited Herdr evidence):

```bash
target/debug/cockpit browser feedback \
  --config /tmp/cb0908-y9mvnus6/cockpit.toml \
  --herdr-session browser0908-y9mvnus6 \
  --herdr-socket /tmp/cb0908-y9mvnus6/config/herdr/sessions/browser0908-y9mvnus6/herdr.sock \
  --space w3
```

Installed executables: `/home/linuxbrew/.linuxbrew/bin/playwright-cli` and `/home/linuxbrew/.linuxbrew/Cellar/herdr/0.9.0/bin/herdr`. Herdr itself rejects `--socket`; use `HERDR_SOCKET_PATH` and the disposable `XDG_CONFIG_HOME`, plus `--session`. Cockpit uses `--herdr-socket`.

For real toolbar activation through the installed CLI, `Extensions.triggerAction` requires a CDP **tab** target, not a page target. Discover it fresh with `Target.getTargets` filtered to `type: 'tab'`. Extension ID is `fblkilbfbmpndnfjaacljmcljhepakok`. Wait for the popup to render before physical input; immediate coordinate clicks hit the underlying page. X11 input was test-only, guarded to `:199` and the known disposable process/profile. Do not copy X11 assumptions into production.

Evidence files under the disposable root:

- `live-stroke-polish.png`: stroke visible before pointer release.
- `inline-comment-polish.png`: text editor beside the mark.
- `native-polish-commands-ready.png`, `native-polish-feedback.png`: actual final native surface.
- `cockpit-state/browser/artifacts/capture-d49b963b-aab4-43de-9aa6-f6ae68716d7e.png`: polished freehand with optional inline text.
- `cockpit-state/browser/artifacts/capture-79ab5ba4-73c0-4803-9d8e-041b20a99fea.png`: drawing-only capture, no default label.
- Earlier agent-read image: `capture-05917819-857c-4db5-b295-9bf9c59e97a0.png`; its first annotation ID was `49a2d00f-ba90-42ae-9acb-848e957121ba`.

All currently saved captures were acknowledged or directly sent; last authoritative pending count was zero. Artifact files may expire under the running retention policy. Process-local Eval variables and browser handles are not a handoff API; reopen/reconstruct tools from the addresses above.
