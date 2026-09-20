# WEB-02 — Deliver first browser click and element pick

## Outcome

Make first-use browser interaction deterministic. After a fresh inline view opens, one measured click on a visible fixture control is delivered exactly once without requiring a compensating move or second click. If the control lease is stale or refused, the pane shows an actionable local state and does not replay an uncertain gesture. Selecting Element on a supported ordinary DOM target after the first eligible frame performs a real bounded inspection and creates the annotation without a prior Browse gesture. Transient readiness resolves without a compensating Browse interaction; unsupported/stale negative cases receive an explicit limitation.

This task addresses issue #10 and residual issue #9 finding 5 while preserving document/target/frame identity checks.

## Evidence and starting points

- Baseline: `6f6222b74e4f552ce697e61364cf653f4b6be29f`; #10 and #9 reports are historical disposable-client evidence, not a current rerun.
- Issue #10: https://github.com/nnexai/cockpit/issues/10. Umbrella findings: https://github.com/nnexai/cockpit/issues/9.
- Read `planning/inline-space-browser-2026-09-13/HANDOFF.md` and `01-architecture-and-transport.md`; input contract requires explicit takeover, ordered down/up boundaries, and no uncertain replay.
- Source starts: `src/app/browser/BrowserPane.tsx` `runInputJobs`, `command`, `ensureControl`, `inspect`, `remotePointer`, `onPointerDown`, and `onPointerUp` (current lines around 144, 172, 243, 425, 434, 463, 514).
- Source starts: `browser-runtime/browser-helper.mjs` `requireControl`, `command`, input sequencing, and inspection command handling (around 373, 966–1195).
- Preserve `src/app/browser/transform.ts` and `framePresenter.ts` identity/geometry validation; do not fabricate DOM evidence from JPEG coordinates.
- Use `planning/stability-and-gitlab-2026-09-20/tasks.json` and `../ORCHESTRATOR.md` for lock ownership and verification discipline.

## Changes

1. Trace the first pointer boundary from presentation through `ensureControl` and helper `requireControl` to the CDP dispatch. Make takeover confirmation precede the first down while retaining that one user intent in an ordered transaction.
2. Distinguish accepted, rejected, stale, unsupported, cancelled-before-dispatch, and outcome-unknown results. Replace silent pointer/wheel stale returns with a visible browser-local state and safe retry affordance.
3. Never auto-replay a dispatched uncertain click, down, up, or drag. On lease loss/cancel, release held remote buttons and clear local capture state.
4. Initialize the valid pointer/inspection sample needed for a first Element pick without requiring Browse first. Retain only identity-valid read-only inspection intent through transient readiness and obtain real DOM evidence. Preserve target, document, viewport, frame and pointer-sample checks; an indefinite “inspection not ready” message is not a fix.
5. Preserve late-inspection rejection: an old target/document/viewport reply cannot move the current outline or create an annotation in a new context.
6. Add a required input-ownership review for first takeover, pointer capture/release, stale outcomes, and duplicate-dispatch risk. Keep changes serialized with all other browser UI/helper writers.

## Non-goals

- No selector-only element annotations, image-coordinate guesses, page-injected annotation host, or privileged generic CDP API.
- No broad retry policy, hidden error suppression, or duplicate “helpful” click dispatch.
- No redesign of wheel, keyboard, clipboard, navigation, dialogs, or full browser facilities; those are covered by WEB-04.
- No changes to Herdr terminal ownership semantics beyond the browser-local interaction callback.

## Acceptance

1. Browser gateway fresh-open fixture: one click on a visible button changes its page state exactly once on the first gesture; no second move/click is required.
2. Linux-native Tauri fresh-open fixture: the same first click reaches the page exactly once through the native adapter and remains correct after returning from terminal focus.
3. Exercise left, right, and middle click, drag with release outside the pane, pointer cancel, letterbox/chrome presses, and lease loss; no stuck remote button or leaked annotation gesture remains.
4. Force a stale/refused takeover: the pane shows an actionable status, preserves only a safe retry intent, and never replays a click whose dispatch outcome is unknown.
5. Select Element after the first eligible frame without Browse and click an ordinary supported DOM target: a real bounded evidence result and annotation appear on both browser and Linux-native surfaces. No prior Browse gesture or indefinite readiness message is needed.
6. Repeat Element after navigation, resize, same-URL reload, cross-origin iframe/closed-shadow/canvas boundary cases, and delayed replies; inaccessible content is honestly reported and late replies cannot alter current state.
7. Stationary-pointer cursor changes continue to update from metadata without requiring a click; no page-supplied cursor asset is fetched by the client.

8. Keep the pointer transaction tied to the original target/document/viewport while takeover is pending, then use the confirmed lease. A changed document or geometry invalidates the old coordinate; a newer frame sequence alone is not permission to discard an otherwise valid gesture.
9. Treat an explicit user retry as a new gesture with a new input sequence. Never infer that a second browser-surface event is permission to resend a previously uncertain down.
10. Report whether Element is blocked by missing cursor sample, stale frame, inaccessible browsing context, or unsupported content. These states must be distinguishable in the pane and logs without exposing page internals.
11. Preserve the valid local pointer sample through the initial metadata barrier only for inspection; do not use it to bypass the matching-frame requirement for mutation or capture.
12. Keep browser focus acquisition separate from Herdr terminal focus. A first browser click may release terminal intent, but it cannot fabricate a terminal focus acknowledgement.

The implementation should prefer one ordered command path over parallel takeover and pointer promises. If the helper has already dispatched a mutation and the response is lost, retain outcome-unknown semantics and require explicit user resolution.

## Verification

The integration owner must run disposable browser-gateway and Linux-native Tauri sessions with isolated profiles and fixtures. Capture first-gesture input sequence, control lease generation, helper/CDP dispatch outcome, page event counter, presented frame identity, inspection identity, and visible refusal text. Exercise A04, A10, A15, and relevant A08/A11 takeover cases, including negative uncertain-outcome controls. Compare browser/native results and clean all owned sessions; a source trace or screenshot alone is insufficient.

The evidence record must include:

- first-click page event count, input sequence, lease generation, and helper/CDP dispatch outcome;
- successful no-prior-Browse Element annotation on supported content, with presented-frame and inspection identity; explicit stale/unsupported limitations are separate negative evidence;
- stale/refused takeover and lost-response negatives showing no duplicate page mutation;
- browser/native resource cleanup and the TUI focus comparison where the gesture crosses terminal ownership.

Do not mark a click as delivered from a changed screenshot alone; correlate the authoritative fixture event with the exact gesture.

Record one successful first-click control event and one refused/unknown negative in the same run so the report distinguishes delivery from silent suppression.

The evidence must identify whether the initial Element result was based on cursor metadata or an explicitly valid local pointer sample.

## Handoff

Return changed paths, the result classification policy, and unresolved input/inspection risks. The integration owner must write `runs/<run-id>/WEB-02.md`, record durable artifacts and exact acceptance rows, and land a real commit for the owned change. Keep `browser-ui` and `browser-helper` edits serialized with WEB-01/03 per `../tasks.json`; do not alter the ledger from this brief. Do not claim completion from a focused unit test without real first-click and first-Element surface proof.
