# Contract and implementation evidence

The first sections record the original planning probe. The final section records the implemented product's test gates and disposable runtime smoke; unexercised acceptance/performance scenarios remain explicitly unclaimed.

## Disposable Herdr split probe

Used `scripts/verify/ui_polish_runtime.py start`, which created owned fixture `/tmp/cpol-23ecgv3j`; all RPCs below targeted that fixture only. Its snapshot reported Herdr 0.9.2, protocol 22. Stopped the fixture with the script after the probe; only recorded matching fixture processes were stopped.

Commands:

```sh
python3 scripts/verify/ui_polish_runtime.py rpc /tmp/cpol-23ecgv3j pane.split '{"target_pane_id":"w1:p1","direction":"right","focus":true}'
python3 scripts/verify/ui_polish_runtime.py rpc /tmp/cpol-23ecgv3j pane.list '{"tab_id":"w1:t1"}'
python3 scripts/verify/ui_polish_runtime.py rpc /tmp/cpol-23ecgv3j session.snapshot
python3 scripts/verify/ui_polish_runtime.py stop /tmp/cpol-23ecgv3j
```

Observed split result (identity-bearing fields shown; cwd/status/scroll/revision also returned):

```json
{
  "id": "polish-proof",
  "result": {
    "type": "pane_info",
    "pane": {
      "pane_id": "w1:p2",
      "terminal_id": "term_65ca641775fa62",
      "workspace_id": "w1",
      "tab_id": "w1:t1",
      "focused": true
    }
  }
}
```

`pane.list` returned `type: "pane_list"` with two members, `w1:p1` and `w1:p2`. The subsequent snapshot independently confirmed `w1:p2`, terminal `term_65ca641775fa62`, workspace `w1`, tab `w1:t1`. Thus the current raw split result provides a creation identity which the Cockpit adapter can validate and retain instead of discarding. Implementers must validate identity/membership against current ordered state and retain unknown-outcome handling; focus is not creation proof.

The observed array order does not establish a documented semantic member-order guarantee. The initial Cockpit grid will use deterministic ordering of confirmed stable pane IDs, not Herdr geometry or layout-order hints. No concurrent creation race was exercised in this probe.

## Approved interactive reference

[`mocks/tab-layout/demo.html`](mocks/tab-layout/demo.html), commit `6aa0331`, is the approved interaction/appearance reference, not a backend implementation.

Before user approval, browser smoke exercised actual pointer-driven centre swap, edge reposition, divider resizing; right/down splitting, viewer reuse, zoom/restore, external equal-share column insertion, last-terminal closure of viewers, and tab isolation. Browser errors were empty at the end of that smoke. User then explicitly approved drag/drop, resizing, appearance and control icons; approval is recorded in [`00-contract.md`](00-contract.md).

## Original planning-phase limits

- This planning probe verifies raw split-result identity and membership, not the new Cockpit layout or browser integration.
- No observer/control arbitration probe was repeated: the user's stated control-only sizing behavior is an accepted requirement.
- Files/Review authorization, tab-scoped browser isolation, durable draft migration, focus reconciliation and native parity remain implementation acceptance scenarios in the plan.
- No product build, tests, formatter or new product runtime scenario was run during this planning phase.

## Implementation verification

The implementation followed the approved interaction reference and ownership cutover. The user requested reasonable main-path verification rather than exhaustive edge-case or performance loops.

### Gates

- Protocol TypeScript regenerated from Rust, including retained `SavedTab` work scopes.
- `bun run typecheck` passed.
- `bun run test` passed: **561 tests, 62 files**.
- `bun run build` passed.
- `cargo test --workspace --exclude cockpit-tauri` passed: **606 tests**.
- `cargo test -p cockpit-tauri` passed: **1 test, 1 ignored**.
- Explicit host and native builds passed: `cargo build -p cockpit-host --bin cockpit`, then `cargo build -p cockpit-tauri`. A combined command containing `--bin cockpit` filters out the native executable; do not use it as native build evidence.
- Review's standard-worker-stack regression and exact Git paging continuation tests passed. Actual native Review then displayed the affected diff without its earlier stack overflow.
- Formatting check remains non-clean. No repository-wide formatting changes were applied. Test/build output also contains the existing ts-rs serde-attribute, jsdom canvas/localStorage and bundle-size diagnostics.

### Browser runtime

Owned fixtures `/tmp/cpol-poahatlm` and `/tmp/cpol-xc2e658y` were started and stopped with `scripts/verify/ui_polish_runtime.py`, without plugins.

- Files opened, listed `README.md` and `sample.txt`, and rendered the README; Herdr still had only its original real terminal.
- Review displayed the actual fixture Git changes, including `Changed line five` and `Added review line`.
- Splitting from Files placed the validated new terminal below Files, not beside the runtime source terminal.
- Actual header centre-swap and edge-drop worked; all surviving leaf DOM nodes stayed identical. Divider resizing changed a terminal from 584px to 659px live and after release. Zoom/restore worked.
- An external terminal appeared as a full-height right-edge column; existing viewer selection stayed intact. External `pane.focus` then selected that terminal.
- Two tabs had independent managed browser profiles. Closing the second tab's Browser removed only its profile; returning to the first tab preserved its mixed layout and Browser.
- Actual terminal input produced `SMOKE_TERMINAL_OK`, independently confirmed by Herdr `pane.read`.
- On the final rebuilt fixture, closing the last terminal retired Browser and deleted its profile. Saved browser work remained reachable under its original tab identity after page reload, even with no Spaces or terminal panes remaining.

Visual evidence: [`evidence/browser/mixed-workbench.png`](evidence/browser/mixed-workbench.png), [`evidence/browser/retired-tab-saved-work.png`](evidence/browser/retired-tab-saved-work.png).

The browser automation wrapper advertises DPR 1.25 while Chromium's device-pixel ResizeObserver reports CSS-pixel sizes, causing an xterm WebGL test-rendering mismatch. Raw same-cell Chromium viewport sizing at DPR 1 with a real size change rendered both terminal prompts and command output correctly; native terminal rendering independently worked. No product workaround was added for this tooling mismatch.

### Native runtime

Actual Linux Tauri/WebKit smoke ran in a private compositor against disposable, pluginless fixtures. [`evidence/native/native-smoke-result.json`](evidence/native/native-smoke-result.json) records outcomes and screenshots.

- Terminal prompt and keyboard command output rendered.
- Files listed and rendered the fixture source.
- The final rebuilt native app and matching gateway owner displayed the real Review diff; the observed native stack overflow no longer occurred in the affected scenario.
- Files selection survived unchanged Herdr state.
- Managed Browser rendered the owned fixture page. Close removed its profile and live association receipt without repeating the action.

One native Browser close remained pending for over two minutes before completing. Its latency cause is unproven; no speculative timeout or producer change was added. This observation is not a responsiveness pass.

### Safety and limits

One combined high-severity review found three browser provenance/retirement risks. Corrections pin retirement to the outgoing association, retain discoverable offline saved-tab provenance, and sync legacy archive publication before deleting the original receipt. Focused authorization, cleanup-exclusion, migration, source-replacement, creation-correlation and durable-recovery regressions passed in the integrated gates.

No claim is made for the full A01–A73 matrix, live legacy-agent delivery, macOS, exhaustive control arbitration, large-pane performance, or timing thresholds. All disposable fixtures were stopped; the user's plugin registry mtime remained `1790518335`.

## Ephemeral pane-state follow-up

The user superseded the retained-work policy: remove saved/archive cleanup and recovery panels, clear pane-local state at owner startup, and keep normal Git Review. Active rules are in `DECISIONS.md`; the implementation evidence above records the earlier retention policy, not the current contract.

- Removed saved-tab/archive DTOs, routes, native commands, CLI addressing, recovery panels and saved-work close guards. Current-run annotation/comment capture and inline cleanup failures remain.
- Exclusive-owner startup resets browser work, comments/paste receipts and Review scratch before publishing IPC or constructing dependent services. Observers joining an active owner do not reset it.
- The real crash/restart fixture `/tmp/cpol-fgmi0dbw` initially exposed persistent Playwright session metadata after successful close. Shutdown confirmation now validates derived workspace/profile metadata and actual daemon generation, socket and CDP state rather than requiring the metadata file to disappear. Corrected startup succeeded.
- The corrected restart removed the old profile, receipt, draft, archive and comment/Review sentinels. Owner-lock inode `5887234`, Library data, project-operation state, an external symlink target and the real Herdr pane `w1:p1` were preserved. A real CLI observer left a current-run comment sentinel intact.
- Browser smoke rendered actual README Markdown and Git changes (`Changed line five`, `Added review line`) with no saved/review cleanup panels. A seeded obsolete `pane_id` Review cache was removed without decoding; new versioned viewer-bound snapshot/file caches were created. Ordinary Browser close returned to the remaining panes and removed its profile, receipt and drafts.
- Native smoke persisted a revision-2 Region draft. The private compositor delayed the original close reply: GDB showed all Tokio workers parked and the GTK main thread blocked in Wayland GL buffer swap. No speculative product timeout or lifecycle change was added. With `WEBKIT_DISABLE_COMPOSITING_MODE=1` and `GDK_GL=disable`, the same native draft-close acknowledged `closed/done` in **823 ms**, removed Browser and all association artifacts, and returned to the terminal. This is not a GPU responsiveness pass.
- Verification isolated this change from concurrent, user-owned Herdr-shell work in a detached baseline worktree. TypeScript, **566 frontend tests**, production frontend build, **605 workspace Rust tests**, the native gate (**1 passed, 1 ignored**), and explicit host/native builds passed. Existing ts-rs, jsdom/localStorage and bundle-size diagnostics remain.
- All follow-up disposable fixtures were stopped. No user-session or plugin-install mutation was issued.

Evidence: [`ephemeral-runtime-result.json`](evidence/browser/ephemeral-runtime-result.json), [`main surface after close`](evidence/browser/ephemeral-main-after-close.webp), [`native draft-close result`](evidence/native-ephemeral/software-native-close.json), [`native close correlation`](evidence/native-ephemeral/native-close-correlation.json), and [`native terminal after close`](evidence/native-ephemeral/software-native-close.png).
