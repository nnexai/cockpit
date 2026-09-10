# Cockpit UI implementation constraints

Current-authority note, 2026-09-04: protocol-22 client-shell behavior in `../DECISIONS.md` overrides historical ownership/renderer details below. The next Context/Review design is `../planning/next-level/08-ui-design.md`: detect real extension panes and replace their renderer, with no extension IPC or separate dock/tab authority. The current implemented workbench remains the baseline; these plans add future behavior.

## Scope and authority

This is an implementation constraint note, not a visual direction. Cockpit is a dense desktop developer tool: the first screen is a graphical mirror of a live Herdr session (Spaces, Agents, tabs, panes, and terminals), not a landing page or a locally invented workspace model.

**Herdr-server is authoritative** for named sessions, Spaces/workspaces, tabs, panes, PTYs/processes, focus, layout, agent state, and Herdr metadata/inbox ordering. Cockpit state is a cache and presentation projection; it must not become a second registry or lifecycle owner ([CONTEXT.md §3.2–3.3](../CONTEXT.md), [DECISIONS.md “Herdr authority and transport”](../DECISIONS.md)). The installed Herdr schema/capability surface is checked before use. Unsupported operations must be represented explicitly, never approximated silently. The documented socket API is the source for persistent interaction and subscriptions; CLI wrappers remain appropriate for one-shot/debug/setup operations ([Herdr Socket API](https://herdr.dev/docs/socket-api/)).

## Invariants

### 1. Snapshot plus ordered events

- Bootstrap each selected session from `session.snapshot`; it includes focus identifiers, workspace/tab/pane records, layouts, agents, and protocol metadata.
- A snapshot is not a subscription. Subscribe only after bootstrap, then apply ordered events to the cache.
- Treat sequence gaps, reconnects, session changes, malformed events, and suspected stale state as invalidating the affected cache. Resnapshot and resubscribe rather than guessing or replaying local mutations.
- Never make optimistic local focus/layout/ownership state authoritative. A command response or subsequent Herdr event is the confirmation; until then, expose pending/failure state without pretending the operation happened.
- Every displayed resource must carry enough identity and session context to reject late events from an old selected session.

Herdr explicitly documents that `session.snapshot` is a one-time bootstrap and that clients should subscribe afterward and call it again after reconnect or possible staleness ([Herdr Socket API, “Raw methods”](https://herdr.dev/docs/socket-api/)).

### 2. Selection and focus are server operations

Selecting a Space, tab, pane, or Agent is not merely a React/store update:

1. identify the Herdr resource ID in the currently selected session;
2. send the corresponding Herdr focus operation;
3. accept the authoritative response/event and update local selection;
4. focus the owning resource; for a terminal pane, attach/render only when it is selected and visible.

A local click may indicate intent immediately, but it must not overwrite a newer server focus event. Agent selection resolves to its owning pane and follows the same focus/attachment rules. If a selected resource is renamed, moved, closed, or replaced, reconcile by ID and clear or relocate selection only according to the authoritative event.

Do not infer focus from DOM focus alone. Keyboard focus (which button/input currently receives browser events) and Herdr semantic focus (which workspace/tab/pane Herdr considers focused) are related but distinct state that must be synchronized deliberately.

### 3. Writable ownership follows local intent

Herdr owns the PTY, process, terminal state, and writable-owner arbitration. xterm.js owns rendering and browser interaction only; it must not create a parallel PTY, own process lifetime, or treat its local buffer as canonical. Terminal attachment uses Herdr’s stream semantics, including authoritative current screen/scrollback followed by live output where supported.

There is one writable attachment owner for a terminal. Cockpit keeps three concepts separate:

- **semantic focus** — the workspace/tab/pane Herdr reports as focused;
- **DOM focus** — the browser element currently receiving keyboard or pointer events;
- **control intent** — whether the local user has asked Cockpit to own the selected terminal.

The initially focused pane and a local selection or terminal click may request writable takeover. If another client subsequently takes semantic focus or terminal ownership, Cockpit must clear local control intent, reopen or retain an observer attachment, and continue rendering. It must not request control again until another local user action. Implementers must:

- make ownership/attachment status observable in resource state;
- focus xterm before forwarding the pointer gesture that requested control;
- route ordinary input and resize only from the focused writable attachment;
- serialize or reject input while attach/takeover is pending;
- handle ownership loss as a normal attachment transition, not process closure;
- preserve the last rendered frame and observer updates after ownership loss;
- never close or kill a Herdr process because attachment failed or a pane became hidden;
- preserve a visible retry/resync path when takeover or attach fails.

xterm.js-specific consequence: `Terminal.onData` and `Terminal.onBinary` are user-input hooks whose returned disposables stop listening; `Terminal.onResize` reports viewport size changes, and `Terminal.dispose()` releases the terminal instance ([xterm.js Terminal API](https://xtermjs.org/docs/api/terminal/classes/terminal/)). Forward those signals through `CockpitClient` with pane/session identity and ownership checks. Do not send raw DOM keyboard events directly to a backend.

`Shift+Enter` is an intentional Cockpit input mapping: send one bare LF character and suppress xterm’s ordinary Enter handling for that key combination. Unmodified Enter and all other terminal keys retain normal xterm/Herdr behavior.

### 3.1 Renderer readiness and glyph continuity

The stable DOM renderer must be initialized before its stream:

1. create and open xterm;
2. load the version-compatible Fit addon;
3. fit to the actual pane bounds;
4. only then attach to Herdr using the fitted rows and columns;
5. refit from `ResizeObserver` when pane geometry changes.

Use the final terminal metrics baseline: native system monospace first (`ui-monospace`, then FiraCode Nerd Font Mono, Hack Nerd Font Mono, IBM Plex Mono, Noto Sans Mono, `monospace`), line height `1`, and an explicit visible 8 px scrollbar. Keep the DOM renderer and existing xterm beta/addon versions; do not add Canvas or other renderer dependencies. Verify continuous box-drawing glyphs at zoom when renderer or font metrics change.

### 4. Visible-pane subscription lifetime

Only panes visible in the selected tab keep active xterm renderers and terminal output subscriptions. Hidden tabs detach UI renderers/subscriptions while Herdr panes and processes continue running. Loading hierarchy metadata for every resource is acceptable; maintaining a DOM terminal and live output stream for every pane is forbidden.

Visibility is a lifecycle contract, not a CSS optimization:

- mount/attach when a pane enters the selected tab’s visible layout;
- subscribe and hydrate from authoritative terminal state before live frames are rendered;
- unsubscribe and dispose/detach when it leaves visibility, tab/session selection changes, or the component unmounts;
- make cleanup idempotent and prevent late frames from writing into a reused pane view;
- on becoming visible again, request/resume through the client contract and resync rather than assuming an off-screen xterm retained truth.

A pane can remain visible in the hierarchy while its terminal view is stale/disconnected. That distinction must be represented in state.

### 5. Magic escape has priority

Herdr’s magic escape key has priority over GUI shortcuts and ordinary terminal input. The global key-routing policy must recognize and consume the magic escape before dispatching pane input, browser shortcuts implemented by Cockpit, or command/palette actions. Do not let an xterm `onData` callback, focused text field, composition path, or bubbling click/keyboard handler bypass this priority. Preserve normal text entry and IME behavior for all non-magic input, and avoid claiming browser-reserved behavior unless the Herdr contract requires it.

The exact key encoding/binding comes from the installed, schema-gated Herdr capability surface; do not hard-code undocumented prefix strings. Herdr’s documented key APIs accept semantic key-combo values such as `esc`, modifiers, function keys, and named punctuation, and reject `prefix+` binding strings ([Herdr Socket API, pane key methods](https://herdr.dev/docs/socket-api/)).

### 6. Hierarchy operations are Herdr operations

Spaces map to Herdr workspace resources internally; preserve that translation explicitly while using Herdr-native “Spaces” terminology in the client. Tree and layout controls are affordances for supported Herdr mutations, not local drag-and-drop state.

For supported capabilities, operations include Space expand/collapse, create, rename, reparent, reorder, close, selection, and drag/drop; tab/pane create, close, rename, focus, split, resize, reorder, move, swap, zoom, and layout changes. Use stable IDs, retain server ordering, and reconcile responses/events atomically. A move may change public IDs or parent membership; update references from the authoritative payload rather than assuming a close/create pair. Disable or clearly mark unsupported operations based on capability data and show explicit errors when a server rejects an operation.

For reorder blocks, use Herdr’s atomic operation where available rather than issuing a sequence of locally simulated moves. For pane/layout decisions, ask Herdr for current layout/neighbor/edge data; do not duplicate BSP/layout rules in the frontend. Herdr documents `workspace.move_block`, `pane.move`, `pane.swap`, `pane.layout`, and related authoritative responses/events ([Herdr Socket API](https://herdr.dev/docs/socket-api/)).

### 7. Inline stale and error behavior

When attach, focus, mutation, subscription, reconnect, or resync fails, keep the affected Space/tab/pane/Agent visible with its last-known authoritative state and an inline stale/disconnected/error indication. Explain what failed and expose an actionable retry and/or resync operation. Toasts may supplement this, but a toast alone is forbidden because it disappears and lacks resource context. Do not disable unrelated resources and do not silently close a pane/process.

Distinguish at least:

- **pending** — request sent, authoritative result not received;
- **stale** — displayed cache may no longer reflect Herdr;
- **disconnected** — stream/transport is unavailable;
- **unsupported** — installed schema lacks the capability;
- **failed** — operation was rejected or could not complete.

Clear stale/error state only on an authoritative refresh or successful operation, not merely because a retry button was clicked. Preserve enough operation/resource IDs to prevent an old failure from marking a newer selection stale.

## Event and state implications

A practical UI state model should include:

- selected session ID plus connection/schema/capability status;
- authoritative entity cache keyed by session and Herdr IDs;
- server focus IDs for workspace/tab/pane, separately from DOM focus;
- ordered event cursor/sequence and a resync-required marker;
- selection intent and pending operation status;
- per-pane visibility, renderer lifetime, attachment state, writable-owner/client identity when exposed, and terminal stream cursor;
- per-resource freshness and inline operation error/retry details.

Session switching is a transaction: detach old subscriptions/renderers, connect to the new named session, load its snapshot, subscribe, and discard old selection/late events. Multiple Cockpit windows may coexist; each has a client identity, but Herdr remains authority for focus and writable ownership.

Events must be applied by type and stable resource identity, including workspace/tab/pane lifecycle, focus, move/reorder, layout, agent status, and scroll changes. A closed or moved resource must not leave orphaned renderer subscriptions. If an event cannot be applied safely because its predecessor is missing, mark stale and resnapshot.

## `CockpitClient` boundary

Components, stores, and terminal views consume one transport-neutral, versioned `CockpitClient` interface. They must not import Tauri APIs, call `fetch`/WebSocket directly, know Herdr socket framing, or branch on native versus browser transport.

The contract should cover task-level operations and typed streams for:

- session selection, snapshot, capability/schema status, and resync;
- hierarchy queries and Herdr focus/layout mutations;
- visibility-scoped terminal attach/detach, authoritative view hydration, output events, input, binary input, resize, and ownership/takeover status;
- ordered lifecycle/status events with cursors and cancellation/disposal;
- typed errors distinguishing stale, disconnected, unsupported, rejected, and ownership failures.

The native adapter maps request/response calls to Tauri commands and ordered streams to Tauri channels; the browser adapter maps calls to HTTP and streams to WebSocket. Adapter selection happens once at startup. Tauri documents commands for typed argument/return/error calls and recommends channels for ordered/high-throughput streams ([Tauri calling Rust](https://v2.tauri.app/develop/calling-rust/#channels)); this supports keeping terminal/status streams out of ad hoc global events. Tauri handlers remain thin transport adapters and must delegate business rules to the shared core.

The interface must make subscription disposal explicit and safe, and all adapters must expose equivalent observable semantics. UI code cannot rely on a Tauri-only event, a browser-only reconnect behavior, or transport-specific error text.

## Forbidden shortcuts

- Maintaining a Cockpit-owned session, workspace, focus, agent, PTY, process, or inbox registry.
- Treating local selection, optimistic layout, or an xterm buffer as authoritative.
- Keeping live terminal subscriptions/renderers for every pane or hidden tab.
- Creating a browser-side or Tauri-side PTY, shell, raw socket forwarder, or process lifecycle owner.
- Sending terminal input without focused-pane and writable-owner checks.
- Bypassing magic-escape priority through xterm callbacks or GUI shortcut handlers.
- Faking unsupported Herdr operations, silently falling back to a different operation, or assuming undocumented protocol fields.
- Applying stale events after session switch, reconnect, move, close, or resnapshot.
- Replacing inline resource errors with toasts only, hiding failed resources, or killing processes on attach failure.
- Branching component behavior on Tauri/browser transport or exposing raw Tauri commands to presentation code.
- Reimplementing Herdr’s hierarchy/layout algorithms or custom terminal scrollback as a second authority.
- Using color as the sole indication of agent state, connection, ownership, freshness, or error.

## Minimal accessibility behaviors

Accessibility is best effort in this proof of concept, but these behaviors are still implementation requirements where practical:

- Every interactive Space, Agent, tab, pane, hierarchy action, retry, resync, and takeover/ownership status has a meaningful accessible name.
- Keyboard navigation reaches the session selector, tree, tabs, panes, controls, and inline error actions; current selection and focus are visibly indicated.
- Tree expand/collapse, selected/active resource, stale/disconnected state, agent state, and ownership are conveyed with text or semantics, not color alone.
- Inline errors are programmatically associated with the affected resource/control and contain actionable text; focus moves to or is intentionally preserved near an error without trapping the user.
- Terminal panes expose a usable label and a non-terminal status summary (for example, disconnected or read-only); terminal keyboard handling must not make the surrounding application unreachable.
- Dynamic state changes that matter to task completion (connection loss, takeover result, operation failure) are announced without flooding live terminal output into an accessibility announcement channel.

## Implementation acceptance checklist

- [ ] First screen mirrors one authoritative Herdr session: selector, hierarchical Spaces, Agents, selected Space tabs, and Herdr pane layout.
- [ ] Startup/session switch performs snapshot → subscription, detaches old streams/renderers, and rejects late old-session events.
- [ ] Snapshot/event cursor, gap detection, reconnect, and resnapshot/resubscription behavior are explicit.
- [ ] Space/tab/pane/Agent selection sends Herdr focus and updates from authoritative result/event; DOM focus is not treated as semantic focus.
- [ ] Hierarchy/layout mutations use capability-gated Herdr operations, stable IDs, authoritative ordering, and no local layout algorithm.
- [ ] Exactly one writable terminal attachment is honored per pane; external ownership loss falls back to observation without a reclaim loop, and an explicit local action can take control again.
- [ ] xterm.js is renderer/input glue only; no Cockpit PTY/process or authoritative scrollback exists.
- [ ] Stable DOM renderer and Fit initialize before stream attachment; initial rows/columns match pane bounds, terminal metrics use the explicit baseline, and box-drawing glyphs remain continuous at zoom.
- [ ] Terminal renderers and live subscriptions exist only for visible panes in the selected tab; disposal is idempotent and remount resyncs.
- [ ] Ordinary input/resize is routed through `CockpitClient` with pane/session/owner checks.
- [ ] Magic escape is intercepted with Herdr priority before terminal input or GUI shortcuts.
- [ ] Attach/reconnect/mutation errors remain inline on the affected resource with last-known state, stale/disconnected classification, retry, and resync.
- [ ] Unsupported capabilities are explicit and do not silently degrade into another operation.
- [ ] Components use only the shared `CockpitClient`; Tauri commands/channels and browser HTTP/WebSocket are adapter concerns.
- [ ] Native and browser builds exercise the same observable contract, including event ordering, cancellation/disposal, reconnect, and error mapping.
- [ ] Keyboard access, visible focus, meaningful labels, non-color-only state, and actionable error text are present for the core surface.

## Sources

- Repository requirements and decisions: [CONTEXT.md](../CONTEXT.md), [DECISIONS.md](../DECISIONS.md).
- Herdr, **Socket API** (latest documented schema and control surface): <https://herdr.dev/docs/socket-api/>.
- xterm.js, **Terminal API** (`onData`, `onBinary`, `onResize`, `dispose`, and related lifecycle): <https://xtermjs.org/docs/api/terminal/classes/terminal/>.
- Tauri v2, **Calling Rust from the Frontend** (commands, errors, and channels): <https://v2.tauri.app/develop/calling-rust/#channels>.
