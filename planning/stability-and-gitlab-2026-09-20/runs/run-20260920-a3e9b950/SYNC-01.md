# SYNC-01 — cancellation ownership

## Accepted implementation plan

- Run: `run-20260920-a3e9b950`; orchestrator/integration owner: Main (Astra). Accepted 2026-09-21 against `6593cf4ecba87687c809ee9f3d2a295c757db787`.
- Dependency TERM-02 is complete at `3ad68c5d547085719af7ab3cd101e57002a56284`. Preserve its authoritative visibility gates, frozen outgoing projection, first-render readiness and non-reclaiming control intent.
- Protected existing changes: VIEW-01 owns `src/app/context/ContextViewer.tsx`, `src/app/review/ReviewPane.tsx`, `ReviewViewer.tsx` and their tests. Do not edit these. No unrelated worktree changes were present at recovery.
- Locks: app-shell, terminal, client-transports, browser-ui, host-streams. WEB-07 concurrently owns only browser configuration/dependency resolution/helper launch. No concurrent runtime mutations.

## Observable outcome and design

Obsolete snapshot/subscription/terminal/browser work retires with its originating identity; current errors retain their code/message; dispatched uncertain mutations retain operation identity and require reconciliation. No automatic replay or new session/provider abstraction.

Current source has session epoch/observation guards but `sessionSnapshot` and `subscribeSession` have no signal. Native session subscriptions can receive an ID after retirement. Terminal native cancellation already tracks late IDs but retirement/error/listener behavior needs consistent settlement. Extend the existing optional AbortSignal convention to session snapshot and subscription, migrate callers, and let App create/abort one controller per observation. Browser fetch/WebSocket work must stop at transport; native finite invoke cannot itself be unsent, so settle the retired caller quietly and discard its result, while every eventual resource ID is explicitly cancelled. Do not claim that retiring a promise kills a dispatched native command.

Keep cancellation distinguishable from genuine transport failures using existing error conventions. Capture the initiating identity at each asynchronous boundary. Terminal/browser queued sends belong to that exact stream generation; discard them on retirement, never forward them to the replacement. A send against an intentionally retired stream must be a quiet no-op or classified stale, not an uncaught exception. A genuine current send failure must remain actionable. Remove abort listeners on every settlement/close path and close late resources exactly once.

Preserve focusCoordinator/mutationCoordinator ordering tokens. Cancellation before mutation dispatch is cancellation; loss after dispatch is outcome_unknown with the existing operation ID, payload hash and receipt/reconciliation workflow. Never infer acceptance from similar snapshot text and never retry implicitly.

## Exclusive writing contract

Worker owns `src/client/CockpitClient.ts`, `src/client/browser.ts`, `src/client/native.ts`, their existing focused tests; session observation and cancellation integration in `src/app/App.tsx`; `src/app/TerminalPane.tsx`; `src/app/browser/BrowserPane.tsx`; affected existing tests and session coordinators only where required by this contract. Host stream registry/native cancellation code may be repaired if needed, but **not** `crates/cockpit-core/src/browser.rs`, `config.rs`, `crates/cockpit-host/src/browser_helper.rs` or `browser-runtime/browser-helper.mjs` (WEB-07). Shared generated protocol changes require Main integration, not concurrent edits. Run LSP references before changing exported TypeScript symbols. Do not edit campaign records.

## Bounded recipe and gates

1. Inspect existing cancellation/error helpers and exported callsites; implement the above clean cutover, not a parallel lifecycle abstraction.
2. Add only regression cases that expose plausible races: pre-aborted creation, deferred native ID after abort, stale error after replacement, current error identity, queued input isolation and pre/post-dispatch mutation distinction. Workers skip all validation, formatters, services and commits.
3. A read-only lifecycle/error-identity review covers both transports before runtime. Main integrates all wave edits once and runs narrow checks once.
4. Per user steering OBS-011, use a short browser-first smoke during implementation. Batch exhaustive deferred-timing/native/TUI and integrated acceptance into the end-of-wave/final campaign round; do not repeatedly run large switch sweeps. A smoke is not full task acceptance.

## Original criterion coverage (unchanged)

1. Browser: delay old snapshot/session-open; switch to replacement session, then release old response. No stale render/error.
2. Native: delay invoke/channel handshakes, retire before ID, release and verify eventual host ID cancellation; cover terminal and browser views as well as session work.
3. Queue input for old pane/binding; retire; prove it never arrives in replacement, while new current input does.
4. Force a real current browser/native failure and record original code/message in its correct surface; expected abort remains quiet.
5. Deliver one safe owned mutation to host, drop only response, retain ID/hash, observe unknown state, reconcile receipt without replay. A separate pre-dispatch abort has no mutation.
6. Bounded rapid focus/control transitions preserve once-only gestures and do not reclaim ownership or overlay stale errors.
7. Disposable TUI/snapshot comparison confirms presentation cancellation does not mutate Herdr focus, ownership or hierarchy.

The final deferred-response matrix covers before creation, during handshake, after ID/before frame and live stream; old promises must resolve after replacement is mounted. Include browser WS close, native invoke rejection, channel cancellation, helper shutdown, terminal resize/frame/mouse/key/clipboard and browser pointer/wheel/key/clipboard/frame ACK. Capture positive current-operation observations so zero obsolete sends is not a vacuous pass.

## Resources, authorization and stop boundaries

Main alone owns runtime under `/tmp/csg-a3e9b950`; all launches go through resource_guard with complete owned HOME/config/runtime environment. Use uniquely named disposable Herdr sessions and owned browser/native surfaces, never default/user sessions. A second session, if needed, must be registered in the owned resource ledger before creation. No provider writes, host-desktop input or personal-browser attachment. OBS-008 protected-home disposition remains unresolved; do not alter those six files. Record only bounded redacted IDs/evidence, clean up only scenario-created resources. If host cancellation requires a shared protocol or launch change, stop that edit and report the exact integration seam to Main.

## Status

Plan accepted; implementation and all runtime acceptance remain unverified. Sole task status is `tasks.json`. No completion or commit claim.

## Settled implementation handoff — 2026-09-21

CancellationLifecycle returned client/App/terminal/browser changes and focused regressions without running tests/builds/runtime. Consumer review found no introduced defect; transport review found premature removal of the native live subscription abort listener and delayed settlement of a cancelled native snapshot. Main repaired both and added focused regressions, not yet run. Terminal/App/browser UI writing passes to TERM-03/WEB-01 against these settled interfaces; client-transports/host-streams stay reserved. Full required acceptance remains deferred under OBS-011, not claimed.
