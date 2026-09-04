# Cockpit Architecture Context

Status: refined proof-of-concept architecture. This document is the implementation reference for the current design.

All filesystem roots, executable locations, Herdr endpoints, and provider settings are configurable. Example absolute paths are intentionally omitted.

## 1. Product vision

Cockpit is a personal, local-first developer cockpit for supervising persistent coding-agent sessions and organizing the context used to work on bounded engineering tasks.

The product is optimized for one developer on a trusted workstation. It is not a multi-tenant service and does not make coding agents autonomous background operators.

Cockpit follows conventional engineering workflows:

- discover a local repository;
- create or open a task worktree;
- attach a companion context directory;
- supervise persistent Herdr sessions and agent terminals;
- gather static issue, review, wiki, and telemetry context;
- inspect, test, and review changes locally;
- destroy owned workspace resources when the task is complete.

The current deliverable is a sophisticated proof of concept for the Herdr client and reusable Cockpit core. Distribution to other users, formal accessibility compliance, remote access, and credential management are later concerns.

## 2. Product domains

Cockpit remains one umbrella product with provider-neutral domains:

- **Forge** — repositories, branches, pull/merge requests, review material, and worktree provenance.
- **Issue Tracker** — issues, comments, labels, parent relationships, references, and freshness checks.
- **Wiki** — pages and related documentation fetched as static context.
- **Telemetry** — optional static log and trace context when a provider adapter is added.
- **Herdr Client** — a richer graphical client for Herdr sessions, intended to feel like the native Herdr TUI.

Forge, Issue Tracker, and Wiki are domain contracts, not separate products. Provider implementations are staged behind capability-based interfaces.

## 3. Runtime architecture

### 3.1 Shared core

The reusable Cockpit core and CLI are implemented in Rust. Package boundaries are:

1. **Protocol** — versioned request, response, error, and event types shared by clients and servers.
2. **Application core** — workspace lifecycle, provider ingestion, freshness, context discovery, authorization rules, and idempotency. It must not depend on Tauri, HTTP, WebSocket, or socket framing.
3. **Herdr adapter** — Herdr protocol, session selection, snapshots, events, terminal attachment, reconnect, and Herdr-specific identifiers.
4. **Provider adapters** — configured external CLI integrations and normalization into Cockpit snapshots.
5. **CLI and gateway hosts** — user-facing commands and `cockpit serve`.
6. **Client adapters** — Tauri IPC/channels for native use and HTTP/WebSocket for browser use.

The Tauri application is a host and presentation layer. Business rules do not live in Tauri handlers.

### 3.2 Native and browser clients

The frontend is shared between native and browser builds.

- The native client packages the static frontend in a Tauri v2 Linux application.
- The browser client receives the same frontend through `cockpit serve`.
- The browser build is not a Tauri target. Tauri is a native application host, not a general-purpose web server.
- Both clients consume one versioned `CockpitClient` contract.
- Native request/response calls use Tauri commands; ordered streams use Tauri channels.
- Browser requests use HTTP; ordered terminal/status streams use WebSocket.
- UI components do not branch on transport details. Adapter selection happens at startup.

`cockpit serve` runs as an explicit foreground process. It serves the static browser frontend and the versioned HTTP/WebSocket API from one loopback origin. The initial browser path is for local verification and automation; remote access and multi-user authorization are out of scope.

Native access is restricted through Tauri capability configuration. The browser path does not initially add a separate authentication mechanism; it remains loopback-only and must not be exposed remotely.

### 3.3 Herdr authority

Herdr-server is the authoritative state machine for:

- named sessions;
- Spaces/workspaces;
- tabs and panes;
- PTYs and processes;
- terminal focus and layout;
- agent detection and agent state;
- Herdr-owned metadata and inbox ordering.

Cockpit does not create a competing session registry or duplicate Herdr lifecycle state. Its local state is a cache of authoritative Herdr data.

## 4. Herdr integration

### 4.1 Transport policy

Cockpit uses Herdr’s documented newline-delimited JSON socket protocol for persistent runtime interaction and long-lived subscriptions.

The Herdr CLI remains useful for:

- one-shot commands;
- human debugging;
- setup/bootstrap operations;
- portability-sensitive operations;
- operations for which a CLI wrapper is the documented interface.

The adapter validates the installed Herdr schema/version and exposes only capabilities supported by that schema. Compatibility is tested with captured schema and representative snapshot/event fixtures for each supported Herdr release, plus a real smoke test against the installed binary.

Herdr’s documented socket API provides the required model:

- `session.snapshot` bootstraps the local cache;
- `events.subscribe` supplies lifecycle and state changes;
- workspace, tab, pane, layout, and agent methods perform mutations;
- terminal attach/read/input operations connect UI panes to server-owned terminals.

The socket transport is newline-delimited JSON. The client must handle request IDs, ordered events, reconnect, stale state, and explicit unsupported-capability errors.

### 4.2 Session selection

The client selects one Herdr named session at a time.

- Startup selects Herdr’s default session when available and provides a session selector.
- Switching sessions detaches old subscriptions/renderers, connects to the new session, loads `session.snapshot`, subscribes to events, and clears stale selection state.
- Multiple Cockpit windows may connect, but Herdr remains the authority for focus and writable terminal ownership.

### 4.3 Workspace and worktree operations

Cockpit uses Herdr’s worktree API for worktree creation, opening, and removal. Herdr worktree provenance is the mapping between a Herdr workspace and its worktree.

Repository discovery enumerates supported repositories beneath a configured default root. The root is not hard-coded.

Branch and workspace-location templates are configurable. Task artifact metadata may contribute to the derived branch or label:

- an issue can provide the task type and identifier;
- a review artifact can already identify its source branch;
- explicit user values override derived values.

Workspace creation eventually performs this sequence:

1. enumerate and select a repository;
2. resolve a typed task/artifact;
3. ask Herdr to create or open the worktree workspace;
4. create the Cockpit-owned companion context resource;
5. attach the companion context through the Herdr session environment and supported workspace association;
6. optionally hydrate explicitly requested context;
7. return the selected Herdr session/resource.

Partial artifacts are allowed. Destruction requires confirmation and removes the owned Herdr worktree/workspace and its associated Cockpit companion context. Central caches and unrelated resources are never removed by workspace destruction.

CoW cloning is preferred where available. A normal copy is a correctness-preserving fallback and emits a performance warning.

## 5. Herdr client UI

### 5.1 Initial foundation

The first usable Cockpit foundation consists of:

- a real Herdr session mirror;
- schema-gated socket connectivity;
- read and attach behavior for terminal panes;
- Herdr-semantic focus and layout operations;
- terminal input/output through Herdr;
- a working `cockpit serve` browser client path;
- protocol contract tests and a real native smoke test.

Workspace creation, context hydration, and concrete provider ingestion follow the Herdr mirror and administration work.

### 5.2 Information architecture

The first screen uses the native Herdr TUI as a behavioral baseline:

- a top-level Herdr session selector;
- a scrollable hierarchical **Spaces** section in the sidebar;
- an **Agents** attention queue below Spaces;
- a main view containing tabs for the selected Space;
- terminal panes arranged according to the selected tab’s Herdr layout.

This is baseline parity, not a permanent imitation target. Cockpit preserves Herdr semantics and authority while deliberately evolving the presentation toward a dense graphical operations workbench.

The UI uses Herdr-native labels such as Spaces, Agents, tabs, and panes. The Herdr API’s workspace terminology remains an internal mapping detail. Agent ordering follows Herdr priority mode—blocked, done, working, idle, then unknown—with newest state transition first inside a priority.

### 5.3 State and interaction

Herdr is authoritative for agent state. The client renders Herdr’s state categories, including blocked, working, done, idle, and other supported states, with transition detail and freshness when available.

User-initiated selection of a Space, tab, pane, or agent:

- records local intent;
- sends the corresponding Herdr focus operation;
- updates confirmed selection from Herdr’s response/event;
- focuses the owning resource;
- attaches the terminal renderer when terminal content is selected and visible.

Semantic focus, DOM keyboard focus, and writable terminal ownership remain separate. An external authoritative focus change supersedes local intent.

The client supports Herdr-semantic hierarchy operations:

- expand/collapse;
- create, rename, reparent, reorder, and close Spaces where supported;
- create, close, rename, focus, split, resize, reorder, and move tabs/panes where supported;
- direct tree controls, context menus, and drag/drop where Herdr supports them.

The behavioral authority is Herdr; the presentation is graphical. Pane/layout actions are primarily shortcut-driven, while native Herdr mouse behavior remains available. GUI controls are discoverable. The Herdr magic escape key has priority over GUI shortcuts.

### 5.4 Terminal attachment and scalability

Herdr owns every PTY, process, current terminal state, and writable-owner arbitration. xterm.js owns rendering and browser-side input capture only.

- Visible panes attach to Herdr’s server-owned terminal streams.
- Fit and WebGL rendering initialize before attachment. The initial attachment uses fitted rows and columns rather than xterm's fallback dimensions. If WebGL is unavailable or lost, the pane keeps working with xterm's built-in text renderer.
- Herdr sends the current rendered terminal state followed by live ANSI frames where supported.
- Normal terminal input goes to the focused xterm and then Herdr only after writable ownership is confirmed.
- Pointer events use Herdr’s structured protocol-20 input path with authoritative pane coordinates. Herdr decides whether a click focuses/selects terminal content or becomes an application mouse report; Cockpit does not synthesize unconditional SGR input.
- `Shift+Enter` sends a bare line-feed. The Herdr magic escape key retains higher priority.
- Kitty keyboard negotiation is enabled. Applications must request enhanced reporting; applications that do not request it keep legacy keyboard encoding.
- WebGL-backed panes load the image addon with Kitty graphics enabled. The current xterm.js implementation accepts direct inline image transmission but not animations, file transfer, or shared-memory transfer.
- Only panes visible in the selected tab keep active xterm renderers/subscriptions.
- Hidden tabs detach UI renderers without stopping Herdr processes.
- Herdr remains authoritative for scrollback/current screen state.
- On reconnect or stale state, the client requests an authoritative view and resubscribes.
- The initially focused pane and explicit local selection/click may request takeover. If another client takes focus or ownership afterward, Cockpit falls back to observation and does not request control again until another local user action.
- Ownership loss is not process closure. Keep the terminal visible, continue observing, and preserve the last frame.
- Attach failure leaves the pane visible with stale/disconnected state, retry, and resync. It never silently closes the Herdr process.

The client loads hierarchy metadata for all resources. It does not require an xterm.js DOM instance or live output subscription for every pane.

### 5.5 Context surface

Context is a first-class read-only surface associated with the selected Space. It can occupy a dedicated Context tab or a pane in a Herdr-supported split layout.

The initial context browser:

- enumerates the companion context tree;
- recognizes frontmatter and resource identity;
- renders Markdown and frontmatter;
- renders bounded plain text and log files;
- renders images when safely supported;
- discovers user-created files automatically;
- refreshes when files change;
- searches through a core-mediated `ripgrep` operation;
- offers external opening for unsafe, binary, executable, or oversized files.

Preview limits are configurable and generous, but finite. Cockpit does not implement a custom search engine or expose arbitrary shell commands to the UI.

There is no generated index or scratchpad in the current scope. Human-created files are discoverable but not managed by Cockpit.

### 5.6 Errors, settings, and accessibility

Errors are inline on the affected Space, tab, pane, or operation. The UI preserves last-known state, explains the failed operation, and offers retry/resync without disabling unrelated resources. Toasts may supplement inline errors.

There is no settings UI initially. Durable configuration uses a config file; environment variables and one-off command options override it.

Accessibility is best effort for this personal proof of concept, not an acceptance gate for broader distribution. The UI should still preserve keyboard operation, visible focus, meaningful labels, non-color-only state, and actionable error text where practical.

## 6. Terminal environment

New Herdr-created terminals receive Cockpit context/workspace variables through the Herdr session environment. Existing Herdr-provided environment metadata is inherited.

Cockpit does not automatically launch or configure OMP. The developer starts agents manually. Herdr’s own integrations report agent state to the presentation layer.

Cockpit does not persist credentials, place secrets in snapshots, or export provider secrets through generated environment values. Provider wrappers retain responsibility for current local credential handling. A future passkey/key-store mechanism is outside this proof of concept.

## 7. Context ingestion and snapshots

### 7.1 Provider boundaries

Cockpit pulls and validates freshness. Developers and agents perform remote writes through the provider CLIs or their normal tools. Cockpit does not silently mutate remote tickets, issues, reviews, or wiki pages.

Provider interfaces are capability-based. Issue Tracker and Wiki adapters primarily provide:

- fetch/pull;
- normalization into Markdown/frontmatter;
- freshness/version comparison;
- explicit unsupported-capability errors.

Forge adapters are initially intended for review-assistant context, not complete forge administration.

Adapters use configured executables and safe argument construction. A missing or unsupported executable creates an explicit unavailable capability rather than silently substituting another provider.

### 7.2 Hydration

Creation hydration is explicit. Adapters decide what to download and how to transform it.

Bounded typed traversal may automatically include:

- the immediate parent issue when available;
- recognized local/provider references in descriptions and comments;
- deduplicated resources within configured depth/count limits;
- cycle detection and explicit per-resource failure status.

Provider failures leave successful assets in place, record retryable per-asset status, and allow the workspace/session to proceed when the primary resource is available.

### 7.3 Cache and companion replication

A configurable central cache stores normalized assets once. Selected assets are copied or reflinked into each Cockpit-owned companion context.

When a cached asset changes, synchronization:

1. fetches and normalizes into a temporary cache object;
2. compares adapter-provided source version/ETag/revision/hash metadata;
3. falls back to canonical content hashing when needed;
4. atomically replaces the central asset;
5. atomically updates companion copies.

Central caches survive workspace destruction. Companion resources do not.

### 7.4 Markdown snapshot format

Every normalized resource is Markdown with a versioned frontmatter envelope.

Mandatory identity/provenance/freshness fields include:

- schema version;
- provider;
- resource type;
- canonical identifier;
- source URL when available;
- fetched timestamp;
- source revision, ETag, version, or hash when available.

The body is normalized provider data intended for human and agent reading. Raw provider payloads are not retained as the canonical snapshot.

### 7.5 First concrete provider

After the Herdr client foundation, the first concrete adapter is Gitea issue context through a configurable Gitea CLI, defaulting to `tea` when it is available.

The first adapter covers:

- issue identity, title, state, and timestamps;
- description, comments, labels, milestones, and assignees;
- source identifiers and freshness metadata;
- recognized local/provider references within traversal limits.

If the configured CLI is missing or lacks the required capability, Cockpit reports the capability as unavailable while keeping Herdr/client workflows usable.

## 8. CLI surface

The initial CLI and core surface provide:

- repository discovery beneath the configured default root;
- Herdr session connectivity and mirroring;
- workspace create/destroy through Herdr worktree operations;
- context hydration hooks;
- `cockpit serve` for the browser frontend and HTTP/WebSocket API.

Provider-specific mutation commands, automatic OMP setup, and broad provider implementations are deferred.

## 9. Verification strategy

Verification uses the actual changed surface:

- protocol contract tests run equivalent behavior cases against native and browser client adapters;
- Herdr schema fixtures cover each supported Herdr release;
- a real native smoke test connects to an installed Herdr-server, mirrors Spaces/Agents/tabs/panes, attaches a visible terminal, sends input, and observes state updates;
- browser verification runs through `cockpit serve` in explicit test mode;
- context/provider tests cover normalization, frontmatter identity, freshness unchanged/changed cases, bounded reference traversal, and partial failures;
- workspace lifecycle tests cover configured repository discovery, Herdr worktree provenance, companion ownership, warning-producing copy fallback, and coupled destruction.

## 10. Deferred scope

- automatic OMP launch/configuration;
- agent history and resumable conversations;
- separate inbox popup and advanced inbox views;
- complete Forge review-assistant ingestion for every provider;
- full Issue Tracker and Wiki provider matrices;
- Cockpit-managed credentials, passkeys, and key-store injection;
- remote browser access, TLS, and multi-user authorization;
- a Cockpit settings screen;
- arbitrary shell execution from the UI;
- raw socket forwarding to browsers;
- unbounded file previews;
- formal WCAG 2.2 AA release compliance.
