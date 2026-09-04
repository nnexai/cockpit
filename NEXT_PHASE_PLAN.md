# Next phase plan: Herdr session mirror

Status: complete. The browser session mirror is implemented and verified end to end against a disposable named Herdr session. Native command/channel parity is implemented, covered by shared contract tests, compiled against the Fedora Tauri stack, and smoke-tested in the real Tauri app against a separate pinned session.

## Goal

Turn the verified status path into the first useful Cockpit screen: one selected Herdr session, an authoritative Spaces and Agents mirror, Herdr tabs and pane layout, and one real visible terminal attachment. Native and browser clients must observe the same `CockpitClient` behavior.

## Entry criteria

- Bootstrap Rust, frontend, gateway, and protocol gates pass.
- Normal status reports Herdr 0.8.2, protocol 20, schema 1 as compatible.
- Browser status smoke passes through `cockpit serve`.
- Fedora D-Bus, WebKitGTK, GTK3, libsoup3, XDo, Ayatana AppIndicator, and librsvg development packages are provisioned for native builds.

## Previous read-only increment (superseded)

- Typed, sanitized `session.snapshot` projection for Spaces, tabs, panes, layouts, agents, and authoritative focus IDs.
- Typed visible-pane output through documented `herdr pane read`, with ANSI/control sanitization and no raw Herdr metadata exposure.
- Equivalent browser and Tauri snapshot/output operations behind `CockpitClient`.
- Full-window workbench with real Space, agent, tab, pane, and terminal data.
- Non-overlapping two-second snapshot polling and one-second selected-pane polling; local browsing is explicitly distinct from Herdr focus.
- Compatibility gating, relational snapshot validation, structured error preservation, strict runtime client parsing, and regression coverage.

This baseline was replaced by the event-driven session and terminal streams in this phase. The pane-read polling routes were removed in the clean cutover.

## Current contracts

- Rust protocol source: `cockpit_protocol::v1`, with generated TypeScript at `src/protocol/generated/v1.ts`.
- Application entry point: async `CockpitService` operations behind the async `HerdrAdapter` seam.
- Herdr configuration and transport: `HerdrCliConfig` and `HerdrCliAdapter`.
- Browser host: the existing `cockpit serve` Axum router and root `dist/` static output.
- Frontend seam: `CockpitClient`, selected once at startup, with browser and native adapters.
- Native host: bounded status, session, focus, subscription, terminal, command, and cancellation commands/channels with an explicit capability allowlist.

## Scope

### Protocol

Add only the operations exercised by this slice:

- list/select named sessions;
- session snapshot and compatibility state;
- ordered session events with session identity and sequence/cursor metadata where Herdr supplies it;
- focus Space, tab, pane, and agent;
- visible-pane terminal observe/control attachment;
- terminal frame, ownership, takeover, resize, input, release, closed, stale, and disconnected states;
- resync and explicit unsupported/error results.

Generate TypeScript from the Rust source and keep the drift gate. Do not add workspace lifecycle, context, or provider DTOs yet.

### Herdr adapter

Implement the documented Unix newline-delimited JSON path for `ping`, `session.snapshot`, `events.subscribe`, and focus operations.

The adapter owns:

- configurable named-session socket resolution;
- request IDs and response/event demultiplexing;
- installed schema and required-method checks;
- snapshot then subscription ordering;
- session-tagged event delivery;
- disconnect detection and resnapshot/resubscribe;
- unknown fields/events without loss of diagnostics;
- stale state when ordering cannot be proven.

Use the documented `herdr terminal session observe/control` subprocess stream for terminals. Do not reimplement the undocumented binary handshake in this phase. Decode newline JSON terminal frames, base64 ANSI bytes, size, sequence, full/incremental state, closed reason, input, resize, scroll, release, one-controller ownership, and explicit takeover conflicts.

### Core

Add one session-mirror module behind a small interface. It owns the cache and transitions, not Herdr lifecycle.

Invariants:

- one selected session at a time;
- session switch detaches old subscriptions and terminal renderers before loading the new snapshot;
- late events from a prior session are rejected;
- local clicks are pending intent until Herdr confirms focus;
- an event gap or malformed transition marks state stale and triggers resync;
- attach failure never closes a pane or process;
- only panes visible in the selected tab may hold terminal streams;
- automatic takeover remains the accepted proof-of-concept behavior and is visible as pending/success/failure.

### Hosts

Extend the existing status transport rather than creating new hosts.

- Browser: bounded HTTP commands plus WebSocket ordered event and terminal streams through the existing loopback `cockpit serve` origin.
- Native: thin Tauri commands plus channels with the same protocol events and cancellation semantics.
- Both transports expose task-level operations. Neither forwards raw Herdr sockets, arbitrary subprocesses, or shell access.
- Subscription disposal must be explicit and idempotent.

### Frontend

Implement the first screen from `research/ui-design-direction.md` and the invariants in `research/ui-implementation-constraints.md`.

Initial visible UI:

- 272 px default resizable sidebar;
- session selector;
- hierarchical Spaces section;
- Herdr-ordered Agents attention queue;
- selected Space tab strip;
- exact Herdr pane layout;
- 26 px pane headers;
- xterm.js only for panes visible in the selected tab;
- inline pending, stale, disconnected, unsupported, takeover, and failed states.

Use `@xterm/xterm` and `@xterm/addon-fit` only after the terminal stream contract exists. Herdr remains PTY, process, focus, ownership, and scrollback authority. The magic escape path must run before xterm input or Cockpit shortcuts.

## Implementation order

1. Capture redacted snapshot/event fixtures and synthetic terminal frame/control fixtures for Herdr 0.8.2.
2. Extend Rust protocol and generated TypeScript; freeze operation names and state discriminants.
3. Implement raw socket request/snapshot/subscription and core cache transitions in parallel.
4. Implement terminal observe/control subprocess handling and ownership transitions.
5. Add browser HTTP/WebSocket and native command/channel adapters against the same core interface.
6. Build the UI state store, session/Spaces/Agents chrome, layout renderer, then visible terminal lifecycle.
7. Run contract, reconnect, stale-state, ownership, browser, and dedicated native smoke gates.

Steps 3 and 4 can run in parallel after the protocol contract. Browser/native adapters can run in parallel after core stream interfaces are fixed. UI chrome and terminal renderer can run in parallel after TypeScript protocol generation, with separate file ownership.

## Tests and smoke proof

Tests must cover:

- schema gate and missing capability;
- request correlation with interleaved events;
- snapshot-before-subscribe order;
- reconnect and resnapshot;
- session switch rejecting late events;
- focus intent versus authoritative confirmation;
- unknown event and malformed frame handling;
- visible-pane attach/dispose/remount;
- one-controller conflict and automatic takeover transitions;
- terminal input/resize/release only after ownership;
- equivalent browser and native adapter mapping;
- inline stale/disconnected/error state reducers.

Browser smoke uses a disposable, dedicated Herdr named session. It mirrors real resources, focuses a real pane, attaches, receives an initial frame, sends harmless input, observes output, hides and restores the pane renderer without stopping the process, then cleans up only resources created by the smoke.

Never take over or mutate a user's existing session during automated smoke. Native smoke starts the real Tauri app pinned to another disposable named session and verifies rendered session state, a stable event subscription, and initial plus subsequent terminal frames. The browser smoke and shared contract tests cover the mutating interaction paths.

## Non-goals

- Workspace/worktree creation or destruction.
- Companion context, context viewer, hydration, or search.
- Gitea or other provider adapters.
- Agent history, separate inbox UI, settings UI, remote access, or credentials.
- Undocumented terminal handshake implementation.
- Multiple simultaneously selected sessions.

## Known blockers

- None for this phase.
