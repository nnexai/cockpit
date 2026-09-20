# NATIVE-02 — Verify macOS native daily use and recovery

Status, dependencies, owner, locks and completion proof: [task ledger](../tasks.json). Follow the [orchestrator contract](../ORCHESTRATOR.md).

## Outcome

The completed daily-use workflow is proven in the real macOS Tauri app, including the macOS-specific setup, installation, plugin and terminal regressions. Linux/browser proof remains separate.

## Evidence and starting points

Regression anchors: closed [#1](https://github.com/nnexai/cockpit/issues/1), [#2](https://github.com/nnexai/cockpit/issues/2), [#4](https://github.com/nnexai/cockpit/issues/4), open [#5](https://github.com/nnexai/cockpit/issues/5) and [#7](https://github.com/nnexai/cockpit/issues/7). The latest inline handoff only claims Linux-native startup, not full platform input/decode parity. Read dependency evidence and `docs/native-install.md`; retain current window defaults/overrides, not old issue-patch defaults.

## Changes

1. Provision an explicitly authorized macOS runner and disposable session/config/install destination. Pin the exact integrated commit and native bundle identity.
2. Exercise compatibility, rapid tab/Space switching, split panes, scrollback under output, glyph alignment/Nerd Font fallback, modifier/clipboard input, resize and TUI/native ownership handoff.
3. Launch Files/Context and Review from real plugin panes; prove approved cwd/root and process-generation policy, borrowed-directory safety, owned worktree/companion setup, interrupted setup recovery and teardown.
4. Exercise inline browser first input, IME/clipboard, image decode, wheel/geometry, hide/show annotations, feedback capture and exact paste without submitting. Include native/browser observer takeover.
5. Exercise GitLab source resolve/import/refresh through native commands with the authorized fixture; preserve local edit conflicts. Confirm missing browser dependencies do not break terminal-only workflows.
6. Route any discovered defect to its owning task (reopen or add a bounded required repair) rather than applying an untracked omnibus patch.

## Non-goals

No inferred pass from Linux, mocked invoke, cross-compilation, or simply surviving startup. No broader macOS packaging/security product. Do not restart the user's app, Herdr session, or personal browser.

## Acceptance

1. A real macOS Tauri window demonstrates all listed user gestures and confirmed results with the integrated build.
2. #1 request-storm regression, #2 plugin-root/rendering, #4 cell alignment, #5 fresh bundle launch and #7 portable companion/partial state have individually identified results.
3. Native channels/image decode/input use the same observable contracts as browser, without hidden polling/reclaim loops.
4. Failure states preserve work and stay resource-local; no retained fixture process/profile/worktree is mistaken for user-owned state.
5. All failures are fixed and reverified before this gate is done; unavailable macOS remains an explicit blocker.

## Verification

Run the actual native scenarios at 1440×900 and 1024×640 where supported and record actual CSS/device scale. Capture focused screenshots, frame/input receipts and request counts as appropriate; compiler/test results complement, not replace, the native run. Use the existing runtime helpers only where supported on Darwin. Any implementation repairs need their own task gates/commits before rerunning the affected scenario.

## Handoff

Commit `runs/<run-id>/NATIVE-02.md` containing per-scenario PASS/FAIL/BLOCKED evidence, source/build identity, regression links, relevant repair hashes and cleanup receipts. No code change is required if behavior already passes. Mark done only after native acceptance is complete.
