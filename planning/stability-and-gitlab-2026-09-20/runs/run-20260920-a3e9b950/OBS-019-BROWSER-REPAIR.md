# OBS-019 — browser repair checkpoint

Date: 2026-09-22. Source parent: `fc74e47` (`fix(browser): preserve input across viewport updates`). macOS native acceptance is explicitly deferred by the user and is not represented as passed. Linux-native annotation acknowledgement remains required under WEB-05.

## WEB-04 code change

The UI now consumes wheel input during a viewport/frame transition instead of dropping it before dispatch. It waits for a frame matching current metadata, maps the original client pointer position against that frame, and dispatches the wheel with the fresh viewport identity. Adjacent pending wheel events at the same client point and modifier state accumulate their deltas while preserving queue boundaries. One two-second deadline covers the whole stalled frame transition, so queued wheel work cannot hold later keyboard/navigation boundaries for a new timeout per event. Disabling live input advances the input generation and invalidates a waiting wheel before a quick re-enable can revive it.

## Verification

- `bunx vitest run src/app/browser/BrowserPane.test.tsx src/app/browser/framePresenter.test.ts` — 5 tests passed across 2 files. BrowserPane regressions cover frame-mismatch wheel retention, accumulated pending deltas, fresh-frame viewport/coordinate mapping, queued keyboard order, timeout allowing a later tab command, and disable/re-enable cancellation.
- `bun run build` — TypeScript check and Vite production build passed. Vite printed its existing large-chunk advisory.
- `python3 planning/stability-and-gitlab-2026-09-20/campaign.py check` — passed for all 27 task records.
- Independent read-only review by GPT-6 Luna found the disable/re-enable race; the input-generation invalidation and regression were added before this checkpoint. A prior review also found the per-wheel timeout hazard; the implementation and timeout regression now use a shared transition deadline.

## Still open

This is component-level regression proof, not a live browser acceptance run. Repeated wheel scrolling, nested scrolling, page-link navigation and recovery must still be exercised through a run-owned browser gateway before WEB-04/WEB-06 can close. The Linux-native annotation save still persisted without UI acknowledgement in the prior run; source review found no confirmed adapter field mismatch, and property-order normalization was ruled out by the live client parser. Capture the actual Linux Tauri response/lifecycle before changing that path. WEB-05 remains blocked on that proof and its broader acceptance. FLOW-01, PERF-01, WEB-08 and ACCEPT-01 still have their separately recorded evidence gaps.
