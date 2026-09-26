# Existing-code review for the next-level plan

Date: 2026-09-04. Application baseline: `8fac660`, whose application files are unchanged from `7e8fe25`. This is a focused code/design review, not a claim of a complete security or runtime audit. No production code was changed. Findings distinguish executed component probes from source-level failure scenarios.

## Verification performed

- Inspected the current App/session reducer, terminal component, native/browser client sequence checks, core adapter interface, and Herdr request/subprocess paths.
- Ran `bun run test`: all 55 tests in three files passed.
- Executed a temporary Bun script importing the actual `sessionReducer` and `createNativeClient`, with local in-memory snapshots and an injected native channel. It did not connect to Herdr or touch user sessions.
- Compared production references to the reducer's attachment actions and the existing test entry points.

The green suite does not contradict the findings below. The probes exercise missing failure orderings, and the current App tests mostly exercise exported helpers rather than mounting its lifecycle.

## CODE-01: late mutation snapshots can roll back live state

Priority: fix before widening the focus/mutation coordinator. Evidence: reproduced against the real reducer; actual network timing was not induced.

`src/app/App.tsx:937` accepts the snapshot returned by `client.mutate`, dispatches `snapshot/received` with `preserveStream: true`, and can separately set selection/control from that response. `src/app/sessionReducer.ts:131` replaces the current snapshot while retaining its current stream generation and sequence. The mutation snapshot has no ordering cursor that proves it is newer than the last streamed snapshot.

Executed sequence:

```text
stream generation 1, sequence 1: focus pane-old
stream generation 1, sequence 2: focus pane-new
late snapshot/received, preserveStream=true: focus pane-old

before: focus=pane-new, sequence=2, sync=live
after:  focus=pane-old, sequence=2, sync=live
```

This can restore old focus/layout while labeling it current. It matters more once paste targets and graphical panes depend on that state.

Planned repair: separate operation acknowledgement from authority to replace the live snapshot. Do not order independent response and event channels by arrival time. Use a Cockpit-side reconciliation rule: track the stream position/attempt around an operation and resnapshot/resubscribe when response ordering cannot be established, or unify observation ordering in the adapter. A snapshot arriving from an older observation must not grant terminal control. No Herdr-server change is required or proposed; do not invent a server revision the API does not supply.

Acceptance must cover an event before a mutation response, an event after it, a session switch, concurrent external focus, an operation whose effect succeeded but snapshot failed, and reconnect during an outstanding mutation. Review the existing test named "adopts an authoritative snapshot while preserving a live stream"; its title assumes the ordering property this code does not prove.

## CODE-02: a generation transition can silently stop state updates

Priority: fix with CODE-01. Evidence: reproduced through the actual native client and reducer; browser implementation has the same relevant condition by inspection.

The private sequence checks in `src/client/native.ts:68` and `src/client/browser.ts:104` require sequence 1 for the first message and consecutive sequences within a generation. On a transition to the next generation they do not require sequence 1. `sessionReducer` refuses such a message in `acceptsStream`, but does not mark that future-generation gap stale in its earlier error branches.

Executed injected-channel sequence:

```text
{generation: 1, sequence: 1}
{generation: 2, sequence: 5}
{generation: 2, sequence: 6}

client delivered messages: 3
client errors: 0
reducer remained: generation=1, sequence=1, sync=live
```

The user can see an apparently live view that is no longer updating. The probe establishes handling of missing/invalid transition frames, not that a healthy Herdr stream emits them.

Planned repair: one shared, tested sequence-transition policy for browser/native adapters and state acceptance, with explicit stale/error behavior rather than silent non-progress. Keep duplicate/stale-frame handling distinct from missing first frames. Preserve per-subscription and session identity checks. Feed the same valid/invalid event corpus through both client adapters and the reducer.

## CODE-03: terminal attachment tests exercise an unused state path

Priority: correct ownership during CLEAN-02. Evidence: production reference search and source inspection.

`src/app/sessionReducer.ts:15` defines `PaneAttachment`; attachment action cases occupy lines 199 onward. Outside tests, searches for `attachment/*` dispatches and `attachments` find only this reducer. `TerminalPane.tsx:160` instead owns attachment/control state locally and validates frames in its subscription callback around line 368.

This is not evidence that the terminal itself is broken. It is evidence that part of the test suite can remain green without testing the implementation the UI uses. Preserving both models would make future code changes and metrics less trustworthy.

Planned cleanup: give the real terminal controller one state owner. Keep frequent terminal frames out of the global React session reducer. Extract the actual frame/control transition logic into a small module used by `TerminalPane`; migrate useful tests to that interface and add mounted component/client-lifecycle tests. Remove the unused reducer attachment cases and their redundant tests once the production behavior is covered. Do not wire terminal traffic through the global reducer merely to make unused code appear useful.

## CODE-04: bounded output does not provide a bounded request lifetime

Priority: repair the request/process runner before adding source-provider subprocesses. Evidence: source-level failure scenarios; no hanging process or live Herdr server was launched.

`cockpit-herdr/src/cli.rs:1051` connects, writes a request, and waits for a matching response without a request deadline. The bounded line reader limits bytes, but a peer that keeps the socket open without finishing a response can leave the request pending. App focus fallback is scheduled only after the request resolves as accepted, so it does not bound this wait.

`cli.rs:920` also launches a subprocess and joins bounded stdout/stderr readers before checking either error. `read_bounded_output` at line 244 returns an error after its byte ceiling. If one reader reaches that ceiling while the other remains open, `tokio::join!` waits for both; the subsequent kill path is not reached yet. A byte limit therefore does not guarantee prompt termination on overflow. The runner has no visible overall deadline or `kill_on_drop` setting.

Planned repair: a small request runner with connect/write/response deadlines and a subprocess runner with a total deadline, output bounds, cancellation, and child reaping. Stop the process when either capture fails instead of waiting indefinitely for the other pipe. Record whether a mutating request was dispatched: timeout after dispatch is unknown, not proof it failed or permission to retry. Preserve streaming-session semantics separately from finite command deadlines.

Acceptance uses disposable fake peers/processes: no response, partial line, unrelated response IDs, stdout overflow with stderr held open, stderr overflow, child never exits, caller cancellation, and response after deadline. Add no process-runner framework or arbitrary command execution UI.

## CODE-05: prefix event handling can also send terminal input

Priority: repair before adding GUI shortcuts. Evidence: current source and a Chromium probe with the installed xterm build; the complete Cockpit App was not mounted in this probe.

`App.tsx:730` handles Ctrl+B and the following command in a window capture listener using `preventDefault()` without stopping event propagation. `TerminalPane.tsx:249` permits these events through its custom xterm handler. The installed xterm `_keyDown` implementation does not check `defaultPrevented` before emitting its translated input.

A browser probe loaded the actual installed xterm build, installed the same prefix capture behavior, and pressed Ctrl+B then Z. It recorded GUI actions `["prefix", "z"]` and terminal data `["\\x02", "z"]`. This establishes the propagation failure mechanism. A mounted Cockpit acceptance test must verify actual input/control state as well.

Planned repair: the input coordinator must consume a recognized workbench shortcut so xterm cannot also process it. Verify keydown/keypress/keyup, modified keys, modal focus, and ordinary terminal input. Do not globally suppress typing or assume `preventDefault` alone stops a JavaScript listener. Test actual DOM routing, not only `prefixCommandForKey`.

## CODE-06: stream identity is lost when commands enter the shared endpoint

Priority: repair before GUI renderer switching. Evidence: source-level lifecycle trace, not an induced live race.

`terminal_wire.rs:122` registers a subscriber with pane and stream IDs, but the forwarding task at line 145 sends `ActorCommand::Pane` with only pane ID. Registrations at lines 324 and 344 replace the subscriber under that pane ID. `handle_command` at line 421 then resolves the current subscriber by pane ID alone. Its release branch removes that subscriber. The separate `Unregister` branch correctly checks stream ID, but command/release routing does not.

If an older attachment overlaps a new attachment for the same pane and client surface, an old queued input/release command can act on the new subscriber. This is local subscription identity, not the retired exclusive Herdr takeover model.

Planned repair: carry a stream/attachment token through every command and release, reject retired tokens, and define duplicate-registration behavior. Preserve the stream ID checks already present on unregister. Test register A, replace with B, then delayed A input/release/unregister; B must remain active and receive none of A's input.

`TerminalPane` closes a pending open only once the promise resolves. Browser/native opening APIs need cancellation or bounded completion so a never-settled handshake does not outlive unmount indefinitely. Separate that resource cancellation from input permission and server pane lifetime.

## CODE-07: compatibility and validation policy need a single owner

Priority: consolidate during CLEAN-03, with correctness tests where behavior changes. Evidence: source inspection; server upgrade race not reproduced.

`cockpit-core/src/lib.rs:266` retains a compatible session entry without reconnect/identity invalidation in normal operation calls. `clear_compatibility` at line 312 clears only installation-wide state. `cli.rs` parses snapshot version/protocol without tying that observation back to cached session compatibility. After replacement/restart of the server at the same session endpoint, the cache may outlive the identity it certified. Revalidate on reconnect or identity mismatch while retaining per-session isolation; do not perform expensive discovery for every terminal frame.

The core resource validator permits a broader ID alphabet/length than the host session validator and Herdr adapter. Those are distinct kinds of identity, so use explicit session/resource validators shared by the relevant callers rather than applying one loose validator everywhere. Preserve stable error codes across native/browser transports. HTTP status mapping remains host-specific but should derive from the same error classification.

`EndpointRegistry::open` also holds its registry mutex across external connection/handshake work. Give connection establishment a deadline and an in-flight entry or compare-before-insert pattern so an unrelated endpoint is not serialized behind a stalled handshake. Do not replace the registry with a general connection framework.

## CODE-08: redraw stability invalidates the current renderer baseline

Priority: first. Evidence: user-reported repeated whole-view dark frames during agent output and Codex `/pets`, plus confirmed full-reset behavior in source. The visual failure has not been reproduced by this review.

The user clarified that flicker may happen on any redraw, not only with Kitty images, and also reports very poor scrolling. Scrolling must be part of the same reproduction and regression gate. Current `send_all_patch` iterates pane subscribers; `send_pane_patch` emits full text and graphics, and the frontend resets xterm for each full frame. Static image checks do not validate this update path. Treat the current renderer/transport decision as reopened and follow [TERM-01/02](../planning/next-level/13-terminal-stability.md) before selecting a fix or preserving this behavior through cleanup.

## Current decisions worth changing carefully

- The exact Herdr version check in `cli.rs:1222` rejects a new release before inspecting compatible protocol/schema behavior. Retain strict runtime contracts, but document a fixture/smoke-based process for approving additional tested releases. Do not silently accept unknown versions.
- `App.tsx:877` retries stale/disconnected state on a fixed 250 ms timer. Prefer a bounded recovery policy with backoff, explicit manual retry, and distinct treatment of permanent incompatibility versus temporary disconnect. It must preserve last-known state and avoid restoring control from stale focus.
- Browser/native stream validation is duplicated. Share the pure transition/error policy, while keeping transport-specific opening, cancellation, backpressure, and delivery mechanisms separate.
- `HerdrAdapter` in `cockpit-core/src/lib.rs:64` is a useful existing seam with real host consumers. Keep it focused on Herdr. Future source providers and filesystem operations should not be appended to this trait solely because they also run in Rust.

## Delivery recommendation

Add a bounded correctness pass after CLEAN-01's evidence capture, before behavior-preserving extraction. TERM-01/02 establishes the reliable renderer/transport baseline first. CODE-01/02 can share an ordering/sequence repair increment; CODE-05/06 need focused input/attachment lifecycle repairs. CODE-04 is a separate request-runner repair, and CODE-03 belongs to terminal-state cleanup. Record before/after tests and run the disposable native/browser/Herdr acceptance path for the production changes later.

These are planned repairs, not fixes performed in this task. The HTML mock and the 55 passing tests do not establish that these failure scenarios have been resolved.
