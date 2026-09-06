# Existing-code repairs

Status: implementation plan. This document turns CODE-01 through CODE-07 in
the existing-code review into five bounded repair stories. CODE-08 belongs to
the terminal stability decision in [13-terminal-stability.md](13-terminal-stability.md).

The review was run against application baseline `8fac660`. Its reducer and
native-client probes reproduced CODE-01 and CODE-02. The browser/xterm probe
reproduced the CODE-05 propagation failure. CODE-04, CODE-06, and CODE-07 are
source-level failure scenarios until their disposable runtime fixtures pass.
The existing 55-test suite is a baseline, not evidence that these orderings
are safe.

The user has selected a verified stable Herdr build as the daily-use target.
The protocol-22 path from `7e8fe25` remains parked while TERM-01/02 establish
the supported terminal pair. If that path is removed during the terminal
decision, the equivalent stream lifecycle checks below still have to pass on
the selected stable path. A deleted implementation is not evidence that the
failure was fixed.

No Herdr server change is part of these stories. `cockpit-herdr` is a client
adapter and may change its timeout, validation, cache, and endpoint code. The
stable target is selected; BOOT-01 verifies its actual wire contract. Changes to
`terminal_wire.rs` are owned by the terminal decision and must be serialized
with REPAIR-03 and any renderer/transport work. Do not add a new server
revision or protocol field to make a Cockpit ordering rule appear provable.

The startup constraint is strict. BOOT-01 restores stable protocol
compatibility before any runtime smoke or temporal reproduction. Astra runs in
the default Herdr session. Orchestration must never restart, kill, upgrade,
downgrade, mutate, or send test input to that default session. A default
session inventory is read-only. All repair fixtures use explicitly named
disposable sessions and the pinned stable executable. No story requires a
pre-BOOT-01 protocol-22 repro.

## Ordering and ownership

BOOT-01's stable compatibility restore comes first. TERM-01 temporal evidence
and TERM-02's stable transport verification then establish the supported terminal
baseline without touching the default session. After that, land the repairs in
this order:

1. REPAIR-01, session ordering and stream generations.
2. REPAIR-02, workbench shortcut input routing.
3. REPAIR-03, terminal attachment identity and cancellation, together with
   the production ownership cleanup currently called CODE-03.
4. REPAIR-04, finite Herdr request and subprocess lifetimes.
5. REPAIR-05, compatibility, validation, error classification, and endpoint
   registry behavior.

REPAIR-01 precedes the focus/mutation extraction in CLEAN-02. REPAIR-03
provides the actual terminal controller behavior that CLEAN-02 must consume;
it does not make the global reducer's unused attachment state authoritative.
REPAIR-04 precedes source-provider subprocess work. REPAIR-05 is the Rust
transport/core cleanup that CLEAN-03 can then extract without hiding a cache
or error-policy change inside a file move.

Every story has two gates. Unit and contract tests prove the named transition.
An automated disposable runtime fixture then drives the real adapter or DOM
path. If the required Herdr executable, WebSocket/Chromium runtime, or native
runtime is absent, record the gate as inconclusive with the missing path and
continue only with tests that do not claim runtime proof. There is no separate
manual user-acceptance requirement for these stories.

## REPAIR-01: order snapshots and generations (CODE-01, CODE-02)

Owner: session/client transport owner. Primary paths are
`src/client/streamOrder.ts` (new), `src/client/native.ts`,
`src/client/browser.ts`, `src/app/sessionReducer.ts`, and the mutation/session
coordination code currently in `src/app/App.tsx`. Tests belong in
`src/client/client.test.ts` and `src/app/sessionReducer.test.ts`, with one
disposable adapter fixture exercising both client transports.

This is a behavior fix. It must not be folded into CLEAN-02's extraction or
described as a reducer rename.

### Failure reproductions

CODE-01 currently accepts a mutation response snapshot as if it were ordered
after live stream events:

```text
stream g1/s1: focused pane-old
stream g1/s2: focused pane-new
late mutation response: focused pane-old, preserveStream=true

before: pane-new, g1/s2, live
after:  pane-old, g1/s2, live
```

This sequence is already reproduced against the real reducer. It does not
prove that a live Herdr server emits this timing.

CODE-02 currently accepts a generation transition without requiring the first
sequence in that generation:

```text
g1/s1, g2/s5, g2/s6

native client messages delivered: 3
native client errors: 0
reducer: still g1/s1 and live
```

The same transition is accepted by the browser checker by inspection. The
fixture must make the resulting stale state visible by asserting that the
consumer requests resync or reports a stream error.

### Implementation steps

1. Add one pure transition policy used by both clients and the reducer. The
   policy must require sequence 1 for the first message and for every accepted
   generation transition, require consecutive sequence numbers within a
   generation, reject an older generation, reject a jump of more than one
   generation, and distinguish duplicate/stale frames from a missing first
   frame. It must retain the session identity check.
2. Make native and browser adapters run the same corpus through that policy.
   Transport code still owns channel/WebSocket closure, cancellation, and
   delivery. The policy owns only the meaning of a transition and its stable
   error classification.
3. Change reducer failure handling so an invalid future-generation transition
   cannot leave `sync: "live"` with the old cursor. Mark the stream stale with
   an explicit `stream_generation` or `stream_sequence` error and request a
   fresh snapshot/subscription. Do not silently accept g+1/s5.
4. Separate a mutation acknowledgement from snapshot authority. While a live
   stream is active, the mutation response confirms the operation and starts a
   reconciliation attempt. It does not replace the live snapshot merely
   because it arrived later. Apply a snapshot as authoritative only when its
   observation attempt is ordered, such as the first valid frame of a fresh
   resubscription. If ordering cannot be established with the current Herdr
   contract, resnapshot/resubscribe and keep the old state marked stale.
5. Keep a mutation whose effect succeeded but whose follow-up snapshot failed
   in an acknowledged/unknown reconciliation state. Show the last known
   snapshot, request resync, and do not retry the mutation automatically.
   Session switches and reconnects cancel the old reconciliation attempt.
6. Remove or rewrite the existing test named "adopts an authoritative snapshot
   while preserving a live stream". Its current assertion assumes an ordering
   cursor that the response does not contain. Preserve a test for an ordered
   fresh stream snapshot.

### Verification

The transition corpus contains at least these 12 cases: initial g1/s1, initial
g1/s2, same-generation +1, same-generation duplicate, same-generation gap,
older generation, g+1/s1, g+1/s5, g+2/s1, sequence overflow, wrong session,
and a stale/error frame. Each case runs through the shared policy, native
adapter, browser adapter, and reducer acceptance path. Expected errors are
asserted by code, not only by message text.

The mutation reconciliation tests cover at least 6 orderings: stream event
before response, event after response, session switch before response,
concurrent external focus, successful operation with failed snapshot, and
reconnect during the operation. The reducer must never report the late old
snapshot as live at the previous stream cursor.

The disposable runtime fixture opens one real session stream and uses a
test-owned transport proxy to inject a generation transition with s5, and verifies a visible stale/resync result. It
then performs one mutation while two stream events arrive around the response
and verifies that the later event or fresh ordered snapshot wins. Native and
browser runs each require 2 successful ordering scenarios and 0 silent-live
failures. A missing runtime is inconclusive.

Failure handling is deterministic. Malformed or identity-mismatched input
closes the affected stream and emits its stable error. A sequence gap keeps
last-known state, sets stale, and starts bounded recovery. A late response
from an old epoch is ignored. No response data grants terminal control until
the corresponding ordered stream state confirms it.

Parallel safety: after TERM-02, REPAIR-01 may run beside REPAIR-02 and
REPAIR-04. It must merge before CLEAN-02, and its shared transition helper
must not be edited concurrently by the browser/native owner and the reducer
owner. It does not touch `terminal_wire.rs`.

## REPAIR-02: consume workbench shortcuts before xterm (CODE-05)

Owner: frontend input owner. Primary paths are `src/app/App.tsx` until
CLEAN-02 creates `src/app/input/keymap.ts`, `src/app/TerminalPane.tsx`, and
the mounted DOM tests in `src/app/App.test.ts` or a focused replacement. This
story fixes event behavior before the keymap extraction.

### Failure reproduction

The current capture listener at `App.tsx:730-744` calls `preventDefault()` for
Ctrl+B and the following command but does not stop propagation. The installed
xterm handler still sees both keydowns. The Chromium probe using the installed
xterm build recorded:

```text
GUI actions:     ["prefix", "z"]
terminal bytes:  ["\\x02", "z"]
```

The expected result is GUI actions `["prefix", "z"]` and terminal bytes `[]`
for the recognized prefix sequence. Ordinary terminal typing must still reach
the terminal.

### Implementation steps

1. Give the input coordinator one consume decision for a recognized workbench
   shortcut. On the keydown that starts or completes a recognized prefix,
   prevent the browser default and stop the event before xterm's handler can
   translate it. Do not rely on `defaultPrevented`; xterm's current handler
   does not use it as a gate.
2. Keep editable fields, modal focus, and terminal focus as explicit routing
   rules. Escape cancels an active prefix. Unknown prefix keys are consumed by
   the prefix coordinator only when the current policy says so. A normal key,
   Ctrl key, IME composition event, and modified terminal key retain terminal
   input behavior.
3. Keep focus confirmation and control ownership separate from keyboard
   consumption. Consuming Ctrl+B must not grant ownership. A command that
   requests focus still waits for Herdr confirmation.
4. Preserve application mouse behavior. A terminal click may select/focus the
   pane and request control; terminal mouse reporting may then send the
   structured mouse command. Explicit `pane.send-keys`/`pane.send-text` SGR
   injection is a verified diagnostic fallback, but it must not bypass focus or
   ownership. The fix must not swallow pointer events or turn a click into text
   input.

### Resize ownership investigation

Resize is an investigation in this story, not a claimed bug. `TerminalPane`
currently sends resize through the terminal stream, and `terminal_wire.rs`
updates shared surface geometry. An observing stream may send resize under the
current contract. Measure whether two panes or an observer/controller pair
race on one surface. If a repair is needed, define one surface coordinator
that coalesces dimensions and identifies the last confirmed geometry while
leaving observation allowed. Do not silently make observation writable for
text, mouse, scroll, or release. Any `terminal_wire.rs` change waits for the
TERM owner.

### Verification

The mounted DOM fixture runs at least 8 input cases: Ctrl+B prefix start,
prefix command, unknown prefix key, ordinary `z`, editable input, modal input,
IME/composition key, and modified terminal key. It asserts GUI action count,
terminal data count, default-prevented state, and propagation for each case.
It separately runs 4 pointer cases: pane click focus, application mouse
command, terminal mouse reporting, and click while a modal is open. The focus
case requires one focus request and no ownership claim before confirmation.

The runtime gate uses the installed xterm build in Chromium and the native
WebKit path when available. Each path must record 0 terminal bytes for the
recognized Ctrl+B prefix sequence, 1 GUI action per recognized command, and
ordinary text delivered once. Native WebKit missing from the environment is
inconclusive for that path, not a pass.

Failure handling: a consumed shortcut is reported once to the coordinator;
duplicate DOM listeners do not execute it twice. A terminal that is observing
rejects text/mouse/scroll commands through its existing ownership rule. That
stream rule does not contradict explicit one-shot Herdr pane APIs, which are
separate and must remain explicit ownership-gated actions. Input after
ownership loss is queued or dropped according to the current bounded queue
policy and never sent to a replacement stream.

Parallel safety: REPAIR-02 can run beside REPAIR-01 and REPAIR-04 after
TERM-02. Do not extract `keymap.ts` as part of this repair. CLEAN-02 owns the
later file move after the mounted behavior gate passes.

## REPAIR-03: retain attachment identity and cancel pending streams (CODE-06,
CODE-03)

Owner: terminal lifecycle owner, coordinated with the TERM-02 terminal
decision. Primary paths are `crates/cockpit-herdr/src/terminal_wire.rs`,
`src/client/CockpitClient.ts`, `src/client/browser.ts`, `src/client/native.ts`,
`src/app/TerminalPane.tsx`, and the native stream registry in
`src-tauri/src/lib.rs`. The terminal decision owns any concurrent changes to
`terminal_wire.rs`; an integration owner serializes this story with renderer
or protocol work.

CODE-06 is a behavior fix. CODE-03 is a production ownership correction. The
global `PaneAttachment` actions in `sessionReducer.ts` have no production
dispatcher, while `TerminalPane` keeps the real attachment state locally.
Do not route terminal frames through the global session reducer to preserve a
dead test path.

### Failure reproduction

The current forwarding task captures `pane_id` but drops `stream_id` when it
sends `ActorCommand::Pane`. Registration B replaces registration A under the
same pane key. A delayed A command then resolves the current subscriber B.
The current unregister path checks stream ID, but command and release routing
do not.

```text
register pane-1/A
register pane-1/B, replacing A
delayed A input
delayed A release
delayed A unregister

before: B is active
bad result: A input reaches B or A release removes B
expected: all three A operations are rejected or ignored; B remains active
```

The browser/native open promise has a second lifecycle case. If the handshake
never settles and `TerminalPane` unmounts, the current cleanup can only close
the stream after the promise resolves. The pending operation can outlive the
pane.

### Implementation steps

1. Carry a stream/attachment token through `ActorCommand::Pane`, release, and
   unregister. Resolve commands by `(pane_id, stream_id)` and reject retired
   tokens before validation or encoding. Release removes only the matching
   subscriber.
2. Define replacement behavior. Registering B for the same pane sends A a
   bounded ownership/lifecycle end or closes A's sender, then installs B. A
   delayed command cannot be delivered to B. Preserve the stream ID on every
   output message, including ownership, error, closed, and disconnected.
3. Give the client open operation a cancellable lifecycle. Browser opening
   closes the WebSocket and rejects once on abort, timeout, or unmount. Native
   opening must expose a cancellation token/handle that can cancel before the
   Herdr open returns, or arrange a bounded command-side pending registry.
   Cancellation is separate from sending terminal `Release` after a stream is
   established.
4. Keep terminal ownership states explicit: `pending` while control is being
   requested, `observing` for read-only observation, `owned` for writable input,
   `conflict` or `lost` after external ownership failure, and `released` after
   close. Only `owned` flushes queued text/mouse/scroll input. Preserve the
   current allowance for resize on an observing stream until the resize
   investigation decides otherwise.
5. Extract the actual attachment transition/controller module used by
   `TerminalPane`. Migrate useful sequence, ownership, close, and cancellation
   tests to that module and a mounted lifecycle fixture. Remove the unused
   reducer attachment actions only after production coverage passes. This
   removal belongs to CLEAN-02's integration boundary even though the behavior
   correction is tracked here.

### Verification

The Rust actor test has exactly 7 steps: register A, send A input, register
B, send delayed A input, send delayed A release, send delayed A unregister,
then send B input. It asserts 1 B registration, 0 A commands delivered after
replacement, 0 B removals, and 1 B input delivered. It also checks duplicate
registration behavior and bounded sender closure.

The client lifecycle tests cover at least 6 cases in both browser and native
adapters: open success, abort before handshake, timeout before handshake,
unmount after handshake, stream ID mismatch, and late message after close. A
case emits at most 1 error callback and 0 callbacks after closure.

The mounted terminal fixture covers at least 6 state transitions: observing,
pending control, owned, ownership conflict, ownership lost, and released. It
checks that text/mouse/scroll are sent only in owned, resize follows the
documented observer rule, and queued input is flushed once after ownership.
The verified CLI SGR injection path and the user-observed xterm.js wheel path
are positive controls. Physical click/focus, application-mode mouse,
coordinates/buttons/modifiers, and scrolling quality remain distinct gates on
the selected stable Herdr path, even though a blanket Herdr mouse block is no
longer valid.

Failure handling: a retired token is a no-op with a diagnostic counter, not a
command sent to the current subscriber. A malformed command produces the
existing stable invalid-command error for the matching stream. A pending-open
timeout closes transport resources and reports `terminal_attach_timeout` once.
If release cannot reach Herdr before its bounded cleanup window, mark the
local stream released and reap the task; never keep a pane alive indefinitely.

Parallel safety: this story cannot edit `terminal_wire.rs` while TERM-01/02,
renderer fallback, or another endpoint-registry change is in progress. The
client cancellation tests and production attachment controller can proceed in
parallel with REPAIR-01/02/04. CLEAN-02 consumes the resulting controller and
owns the final reducer cleanup.

## REPAIR-04: bound Herdr requests and subprocesses (CODE-04)

Owner: Herdr transport/process owner. Primary paths are
`crates/cockpit-herdr/src/cli.rs`, its tests in
`crates/cockpit-herdr/tests/compatibility.rs` and `session.rs`, and a new
focused runner module only if it keeps the Interface narrower. Do not create
a general process-execution framework.

### Failure reproductions

`socket_request` connects, writes a request, and waits for a matching response
without a response deadline. A peer that sends a partial line or keeps the
socket open can leave focus, mutation, or inspection pending forever. The
bounded line reader limits bytes but does not limit time.

`run_json_for` reads stdout and stderr with `tokio::join!`. If stdout exceeds
its bound while stderr stays open, the join waits for stderr before reaching
the kill path. A byte ceiling therefore does not bound process lifetime.

### Implementation steps

1. Introduce one finite request runner with separate connect, write, and
   response deadlines. Use proposed defaults of 500 ms for connect, 500 ms for
   write, and 2 s for a finite response, held in named constants and adjusted
   only from fixture evidence. Event subscription setup has the same bounded
   connect/write/ack phase, then remains a streaming operation with explicit
   cancellation and reconnect handling.
2. Track dispatch state. Before any request bytes are written, failure is
   `request_not_dispatched`. After a complete write, a response timeout or
   disconnect is `request_outcome_unknown` for a mutating method. A partial
   write is also unknown because Herdr may have received a valid prefix. Never
   retry an unknown mutation automatically. A caller cancellation records
   `request_cancelled` alongside dispatch state. After any bytes are written,
   the mutation outcome remains unknown; cancellation does not prove rollback.
3. Preserve request IDs and ignore unrelated response IDs within the bounded
   response window. A matching malformed response is `malformed_response`.
   An oversized or unterminated line is `bounded_output`. Keep server error
   code and message when the matching error envelope is valid.
4. Run command stdout and stderr concurrently but cancel/kill as soon as
   either capture errors or exceeds its limit. Apply a total 5 s command
   deadline, kill the child, and await it before returning. Set `kill_on_drop`
   where supported and retain explicit kill/reap for the timeout path.
5. Apply the same bounded child lifecycle to server autostart. On startup
   timeout or caller cancellation, kill and reap the child. A successful ready
   status may hand the long-lived server to a detached monitor as today.
6. Keep terminal endpoint streaming separate. Do not apply a finite command
   deadline to live terminal frames. Terminal open still has a bounded
   handshake and cancellation path from REPAIR-03.

### Verification

The transport fixture has exactly 8 fake-peer cases: no response, partial
line, unrelated response then matching response, oversized line, malformed
JSON, matching Herdr error, response after deadline, and caller cancellation.
It asserts completion within the configured deadline, the exact error code,
the dispatched/unknown classification, and zero automatic mutation retries.

The process fixture has exactly 6 disposable child cases: stdout overflow
with stderr held open, stderr overflow with stdout held open, child never
exits, malformed JSON after clean exit, nonzero exit, and caller cancellation.
Each case asserts the child is reaped, the result returns within 5 s of the
deadline, and no descendant remains. Tests may use paused Tokio time plus a
pipe or shell fixture; they must not execute arbitrary user commands.

The runtime gate drives the real `HerdrCliAdapter` against a disposable Unix
socket/fixture executable for 2 successful requests and all 8 failure cases.
If the selected Herdr executable or Unix process environment is unavailable,
record the adapter gate as inconclusive. Unit tests still have to pass.

Failure handling is part of the contract. Connect/write timeout before any bytes are written is not dispatched.
A partial write is outcome-unknown. Response timeout after dispatch is unknown.
Malformed, oversized, and wrong-session responses stop the request and keep
their stable classifications. Every killed child receives a reap attempt; a
reap failure is included in the returned diagnostic rather than silently
detaching the process.

Parallel safety: REPAIR-04 can proceed beside REPAIR-01/02 and client-only
work in REPAIR-03. It must merge before source-provider subprocess stories.
Do not extract `transport.rs` or `capabilities.rs` while changing timeout
semantics; CLEAN-03 performs that behavior-preserving split afterward.

## REPAIR-05: own compatibility, validation, errors, and registry state
(CODE-07)

Owner: core/Herdr adapter and host error-policy owner. Primary paths are
`crates/cockpit-core/src/lib.rs`, `crates/cockpit-herdr/src/cli.rs`,
`crates/cockpit-herdr/src/terminal_wire.rs` after TERM-02 coordination,
`crates/cockpit-host/src/server.rs`, and the corresponding core, Herdr, host,
and client contract tests. This story may change the Cockpit host adapter's
mapping. It does not modify the Herdr server.

### Failure scenarios

`CockpitService::inspect_session_if_needed` trusts a compatible per-session
cache entry on later operations. A reconnect or replacement server can make
that entry stale. `clear_compatibility` clears only installation-wide state,
so the two caches can disagree. A same-version replacement cannot be detected
from version/protocol alone unless the transport observes a new connection or
identity token. The plan must be honest about that limit.

The core resource validator allows a 128-byte ID with `:` while the host
session validator allows 96 bytes without `:` and the Herdr session validator
has its own rule. One loose validator cannot serve both identities.

Host errors currently classify all non-invalid, non-conflict failures as 503,
while browser and native clients use different transport-level codes. The
wire envelope already has stable `code` and `message` fields; policy needs one
owner that preserves them.

`EndpointRegistry::open` holds its registry mutex while `connect_endpoint`
performs an external handshake. A stalled handshake for one endpoint can block
an unrelated session/surface.

### Implementation steps

1. Pin the selected stable Herdr executable, version, protocol, schema, and
   required method set from TERM-02 evidence. Update the compatibility fixture
   and constants to that verified target. Keep strict rejection of unknown
   versions and protocols. Protocol 22 artifacts are experimental/parked and
   do not remain the default merely because the current fixture used them.
2. Make compatibility ownership explicit. Keep installation and session
   compatibility isolated, but invalidate the affected session entry on any
   connection failure, reconnect, malformed identity response, or stream
   generation reset. On every new event-stream connection, run the bounded
   identity check before marking it live. On an identity mismatch, discard
   session state and return `session_identity_mismatch`.
3. Do not claim detection of a silent same-version server replacement without
   a server-provided identity. The adapter may use a transport connection
   generation and fresh `ping` on reconnect. It must not invent a revision or
   make an expensive identity request for every terminal frame.
4. Export separate validators for session IDs, resource IDs, and client
   surface IDs. Use the strict stable session rule at every session boundary
   and the qualified resource rule for pane/workspace/tab IDs. Apply the same
   rules in core, host, Herdr, and TypeScript parsing, with exact length and
   character tests. Keep path traversal and control-character rejection.
5. Define one error classification table. Preserve Herdr's arbitrary error
   code/message in `ErrorResponse`; map invalid input to 400, ownership/focus
   conflicts to 409, request deadline after dispatch to an explicit unknown
   outcome, and unavailable/disconnected infrastructure to 503. If the host
   uses 504 for a finite pre-dispatch timeout, test and document that choice.
   Browser and native clients must expose the same operation code and outcome
   classification even though their outer transport codes remain HTTP or
   native.
6. Refactor `EndpointRegistry::open` so the mutex is not held during the
   external handshake. Use an explicit in-flight entry or a compare-before-
   insert path with the connection deadline from REPAIR-04. If a concurrent
   open wins the race, close the unused connection and use the registered
   actor. An unrelated endpoint must continue while another handshake stalls.
   Do not replace the registry with a general connection framework.
7. Keep terminal stream identity and resize ownership compatible with
   REPAIR-03 and TERM-02. The observer allowance for resize remains a stated
   compatibility rule until the single-surface coordinator investigation has
   evidence. Serialize all `terminal_wire.rs` edits with the terminal owner.
8. Publish capabilities per operation. `terminal_mouse_input` is true only
   when the selected stable pair proves the actual structured application
   mouse path. Click/focus, application mouse, scroll, resize, and graphics
   are separate observations. A one-shot CLI SGR emulation route must not be
   silently mapped to structured-pointer capability. A parked or absent
   graphics path must not be advertised as available; an unsupported operation
   returns a stable capability error and leaves ordinary text terminals usable.

### Verification

Core cache tests cover at least 7 cases: installation cache hit, per-session
isolation, session reconnect invalidation, malformed ping invalidation,
identity mismatch, one session failing while another remains compatible, and
same-version replacement documented as undetectable without a new connection
identity. The last case asserts no false claim of detection.

Validation tests cover at least 10 boundaries: valid strict session, session
colon rejection, session over-length rejection, valid qualified resource,
resource traversal rejection, resource control-character rejection, resource
over-length rejection, valid surface ID, invalid surface ID, and invalid
terminal dimensions. Core, host, Herdr, and TypeScript fixtures must agree on
the relevant boundary.

Error mapping tests cover at least 8 outcomes across browser and native
clients: invalid request, focus conflict, terminal ownership conflict,
pre-dispatch timeout, post-dispatch unknown outcome, malformed response,
server unavailable, and disconnect. Each asserts stable operation code and
message plus the host status where applicable.

Capability tests cover at least 5 states: structured mouse available,
structured mouse absent or unverified, explicit SGR injection available,
graphics parked, unsupported operation, and malformed capability payload. Both
clients preserve the state and operation code without turning absence into a
successful no-op.

Registry tests use 3 endpoints and 3 concurrent opens: one stalled handshake,
one unrelated endpoint that must complete, and one duplicate endpoint that
must converge on one actor. The stalled endpoint must not hold the registry
lock beyond its configured deadline. The duplicate test asserts one active
actor and no leaked connection.

The stable-target runtime gate runs status/ping, session snapshot, one focus,
one mutation, one event reconnect, and one terminal open on the exact pinned
binary/hash pair. It must record capability fields and stable errors. Missing
Herdr, Unix sockets, or native/browser runtime is inconclusive for that part;
it cannot be reported as compatible by assumption.

Failure handling: cache entries are removed on the owning session's failure,
not by a failure in another session. Unknown Herdr error codes retain their
code and message and map to the generic classified outcome. A stale endpoint
cannot restore terminal ownership or live UI state. A stalled registry open
returns its bounded attach error and leaves no in-flight entry behind.

Parallel safety: REPAIR-05 follows REPAIR-04's request deadlines and the
TERM-02 stable target. Its `terminal_wire.rs` portion is serialized with
REPAIR-03 and terminal renderer work. Core validation/error tests may run in
parallel with frontend repairs. CLEAN-03 may extract capabilities, transport,
operations, and host error modules only after these behavior tests pass.

## CLEAN boundary and completion record

The repairs above change behavior. CLEAN-02 and CLEAN-03 then preserve those
contracts while moving code behind the planned Modules. CLEAN-02 owns the
final removal of unused `PaneAttachment` reducer actions after REPAIR-03's
mounted controller tests pass. CLEAN-03 owns the file split of capabilities,
transport, operations, and host errors after REPAIR-04/05. No extraction may
reopen the mutation ordering, input consumption, stream identity, deadline,
or error classification rules without a new repair story.

The integration record must contain the baseline and after counts: JS
typecheck, `bun run test`, Rust `cargo fmt --check`, `cargo check --workspace`,
`cargo test --workspace`, and the disposable adapter/runtime gates above. It
must list every inconclusive runtime gate with the missing executable or
environment. A green narrow test, static screenshot, or deleted protocol-22
path is not a substitute for the actual stable-path lifecycle checks.
