# SYNC-01 — Cancel obsolete work without false errors

## Outcome
Session snapshots, subscriptions, terminal attaches, browser streams, and browser commands stop doing work for obsolete identities. Expected cancellation is quiet; a real failure on the current identity remains visible with its original error code/message. A dispatched mutation whose response is lost remains outcome-unknown rather than being silently retried or reported as accepted.

## Evidence and starting points
- Baseline and dependency/locks are authoritative in [../tasks.json](../tasks.json); this task follows TERM-02 and spans App, terminal, browser/native transports, and host streams.
- Related historical anchors: [GitHub #9](https://github.com/nnexai/cockpit/issues/9) and [GitHub #12](https://github.com/nnexai/cockpit/issues/12); neither proves a fresh cancellation failure.
- `src/app/App.tsx:1517-1584` uses active/epoch/observation guards but snapshot and session subscription work can outlive a switch before settling.
- `src/client/browser.ts` session stream/open-terminal paths and `src/client/native.ts` session/terminal/browser subscriptions define transport cancellation and stream close behavior.
- `src/app/session/sessionStore.ts`, `focusCoordinator.ts`, and `mutationCoordinator.ts` contain identity/epoch semantics to preserve.
- `src/app/TerminalPane.tsx:555-692` owns terminal AbortController, stream retirement, pending input, and error presentation.
- `src/app/browser/BrowserPane.tsx` owns browser stream identity, command queues, and stale/error presentation.
- `src/protocol/generated/v1.ts` and `crates/cockpit-core/src/lib.rs` define operation/error identities; do not alter them casually.

## Changes
- Give every pending finite snapshot, subscribe/open handshake, and stream resource a cancellation path tied to session epoch, pane/binding identity, and component lifetime.
- Make both browser and native transports cancel before an open/subscription ID exists, and cancel the host stream once a late ID arrives after retirement.
- Classify expected abort, stale epoch, and superseded identity as normal cancellation; do not publish them as current errors.
- Guard BrowserPane error publication and queued input by the stream identity captured when the operation began.
- Make TerminalPane sends against a retired stream non-throwing or explicitly classified as stale; clear pending input without replaying an uncertain command.
- Preserve unknown-outcome semantics for dispatched mutations: pre-dispatch cancellation is cancellation, dispatch followed by lost response is `outcome_unknown`, and retry requires the existing duplicate-risk/reconciliation path.
- Keep focus/control token guards and do not add automatic command replay or broad retries.
- Require a lifecycle/error-identity review covering browser and native implementations before runtime proof.

## Non-goals
- No weakening of server visibility, ownership, or mutation validation.
- No silent retry/replay of terminal input, browser clicks, clipboard, or mutations.
- No replacing real errors with a generic cancellation message when the current operation genuinely failed.
- No new provider, session, or persistence abstraction.

## Acceptance
1. Browser snapshot and session subscription work cancelled during a session switch produces no late state or error in the replacement session.
2. Native snapshot/subscription and browser-view/terminal channel work cancelled before handshake completion closes or cancels the eventual host stream and produces no stale error.
3. Terminal input and browser input queued for an old pane/binding never reach the replacement pane, while current input still reaches the selected owner once ready.
4. A current transport failure retains its operation code and actionable message in the browser and native surfaces; expected cancellation is not rendered as an error.
5. A dispatched mutation with a lost response is shown as outcome unknown and requires receipt/reconciliation before retry; a pre-dispatch abort does not create a false unknown mutation.
6. Switching focus/control rapidly does not cause duplicate gestures, ownership reclaim loops, or old errors over the new pane.
7. The TUI oracle confirms that Herdr focus/ownership and hierarchy did not change because of a cancelled Cockpit presentation request.

## Verification
Use separate disposable browser and native sessions with deferred snapshot, stream-open, terminal, and browser-command responses. Switch session, Space, tab, and pane before each response, then release old responses and assert no stale render/error/input. Exercise a real current transport failure and preserve its identity. For mutation proof, use a safe disposable action whose request reaches the host, drop only the response, and reconcile without replay. Compare focus and ownership with a uniquely named Herdr TUI session; record every stream/process created and cancelled.

## Handoff
The evidence record must list operation identities, epoch/binding values, cancellation timing, delivered error codes, unknown-outcome receipt, browser/native observations, and resource cleanup. Link the record and real commit from the ledger. Do not claim completion from promise guards or mocked adapters alone.
- Assign every request a visible identity in the evidence: session epoch, pane/binding, stream ID when known, and operation ID for mutations.
- Test cancellation before transport creation, during handshake, after handshake but before first frame, and after a stream is live.
- Resolve old promises after the replacement view is mounted to catch late callbacks that would otherwise be hidden by timing.
- Exercise browser WebSocket close, native invoke rejection, channel cancellation, and helper/process shutdown as separate negative cases.
- Verify a current operation error still reaches the right pane after an unrelated pane or session is switched away.
- Verify an expected abort does not increment the replacement pane's error state, status banner, or retry counter.
- Exercise terminal resize, frame, mouse, keyboard, and clipboard sends at the retirement boundary.
- Exercise browser pointer, wheel, keyboard, clipboard, and frame ACK work at the retirement boundary without replay.
- For mutations, retain the operation ID and payload hash through the lost-response case; reconcile the receipt before any retry.
- Confirm an unknown outcome cannot be converted to accepted merely because the next snapshot happens to contain similar text.
- Check that cancellation of an old browser view cannot close or cancel the newly bound stream sharing a resource key.
- Preserve current focusCoordinator and mutationCoordinator token semantics while adding transport cancellation.

The evidence should distinguish:
- expected cancellation/no-op;
- a real current transport or command error with its original identity;
- a dispatched mutation with unknown outcome;
- a pre-dispatch cancellation with no host side effect.

No timing-only pass is sufficient: force each old response after the replacement identity is live.

- Include both browser and native error text/codes and show where each was rendered.
- Record whether a late stream ID was cancelled after an early abort.
- Preserve the mutation receipt and payload hash without including credentials or giant captures.
- The handoff must identify any transport that could not expose pre-open cancellation.
- A missing deferred native fixture blocks that surface criterion rather than accepting browser-only guards.
