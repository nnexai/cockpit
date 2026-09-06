# Cockpit Architecture Context

Status: architecture reference. The Herdr client, reviewed worktree setup, companion association, and bounded graphical Context source/Markdown browsing are implemented. Media rendering, reference comments/paste, repository snapshots, source ingestion, and graphical review remain planned in `planning/next-level/README.md`. Verified delivery boundaries are recorded in `DECISIONS.md`.

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

Cockpit is intended to become the developer's primary way of engaging with local projects. Maintainable code and easy behavior changes matter more than distribution or product generality. Distribution to other users, formal accessibility compliance, remote access, and credential management are elective later concerns.

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
5. record the companion association in a Cockpit-owned provenance manifest and pass context environment to explicitly created new Cockpit tabs/panes;
6. optionally hydrate explicitly requested context;
7. return the selected Herdr session/resource.

Partial artifacts are allowed. Destruction requires confirmation and removes the owned Herdr worktree/workspace and its associated Cockpit companion context. Central caches and unrelated resources are never removed by workspace destruction.

Independent reflink snapshots are preferred where available. Normal copies preserve correctness and report the fallback. Hardlinks and Git alternates must not couple writable context files to their originals.

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

Herdr owns every PTY, process, terminal model, and terminal stream. xterm.js owns rendering and input capture only.

- The active target is Homebrew Herdr 0.8.2, protocol 20, schema 1. Each visible pane opens its own direct terminal stream with `TerminalAnsi` and `TerminalAttach`; protocol-22 client-shell work remains in committed history.
- Herdr's JSON API owns hierarchy, focus, and layout. Stable `TerminalFrame` messages supply sequence numbers, dimensions, and ANSI bytes for each attached pane.
- Attachment uses fitted per-pane dimensions and measured cell pixels. The first frame must be full; every later sequence must be consecutive, including full repaints.
- A full frame is an ANSI baseline, not permission to reset xterm. Socket framing has one uninterrupted reader with bounded buffering and deterministic shutdown.
- Terminal graphics are parked. Known auxiliary messages are consumed without exposing graphics payloads or disconnecting an otherwise usable text terminal. The image addon is not loaded.
- Text and binary input use stable raw `Input`; wheel/page scrolling uses `AttachScroll`, gated by local control intent and attachment state. Normal xterm.js panes attached to Herdr are observed to receive wheel/scroll events.
- Click-to-focus remains available. The prior physical X11 direct-attach result is scoped to that host-input path; it does not establish that Herdr 0.8.2 cannot carry SGR. Herdr's one-shot `pane.send-keys`/`pane.send-text` APIs can inject SGR into a pane PTY. Cockpit must trace physical pointer interception, focus, ownership, and coordinate routing before advertising structured application-mouse support, and must not inject SGR unconditionally at shell prompts.
- `Shift+Enter` sends a bare line-feed. The Herdr magic escape key retains higher priority.
- Local xterm enables Kitty keyboard support; stable end-to-end enhanced-reporting behavior still requires TERM-03 evidence.
- Only panes visible in the selected tab keep active xterm renderers/subscriptions. Hidden tabs detach UI renderers without stopping Herdr processes.
- Herdr remains authoritative for scrollback and screen state. Reconnect requires a fresh full baseline before consecutive updates.
- Control and observe requests use stable attachment modes. Semantic focus, local control intent, attachment state, and terminal process lifetime remain distinct.
- Attach failure leaves the pane visible with stale/disconnected state, retry, and resync. It never silently closes the Herdr process.

The client loads hierarchy metadata for all resources. It does not require an xterm.js DOM instance or live output subscription for every pane.

### 5.5 Context and graphical review panes

Cockpit detects supported Herdr extension panes and replaces their renderer with a complete graphical implementation. The initial targets are `herdr-file-viewer` for Context/files and `persiyanov.reviewr` for local review. These remain real Herdr panes with normal layout, move, resize, focus, and close behavior. There are no Build/Review workbench modes or synthetic Context tabs.

Detection uses supported Herdr plugin launch provenance and bounded `pane.process_info` evidence, with explicit per-pane renderer selection for ambiguous cases. It does not communicate with the extension's internal state, scrape terminal output, or require extension/Herdr-server changes. The original TUI continues independently; switching to terminal view does not synchronize its comments with Cockpit's own GUI drafts.

The Context browser:

- enumerates the companion context tree and optionally an authorized repository root;
- recognizes frontmatter/resource identity and discovers user-created files;
- renders Markdown with Mermaid, bounded source/plain text/logs, and safe images;
- preserves exact original source line numbers, including frontmatter;
- refreshes on file changes and searches through bounded core-mediated ripgrep;
- offers safe external opening/refusal for unsupported content;
- collects whole-file and selected-line comments across files;
- previews a batch containing real file paths, comments, and selected original lines with line numbers;
- pastes that batch into an explicitly selected same-tab agent without submitting it.

Cockpit owns durable GUI drafts and a local Git review model for the complete Reviewr replacement. Local review includes staged, unstaged, branch, and untracked scopes with explicit revisions and side-aware anchors. It does not mutate Git or post provider comments. A future TUI backport can reuse Cockpit core logic as a separate story.

Preview/search/paste limits are finite and configurable. No generated index or scratchpad is required. Human-created files are discovered but are never treated as managed merely because their frontmatter claims ownership.

See `planning/next-level/04-viewer-and-reference-comments.md`, `07-extension-panes.md`, and `08-ui-design.md` for the proposed implementation and UI contracts.

### 5.6 Errors, settings, and accessibility

Errors are inline on the affected Space, tab, pane, or operation. The UI preserves last-known state, explains the failed operation, and offers retry/resync without disabling unrelated resources. Toasts may supplement inline errors.

There is no settings UI initially. Durable configuration uses a config file; environment variables and one-off command options override it.

Accessibility is best effort for this personal proof of concept, not an acceptance gate for broader distribution. The UI should still preserve keyboard operation, visible focus, meaningful labels, non-color-only state, and actionable error text where practical.

## 6. Terminal environment

Existing Herdr-provided metadata is inherited. Worktree create/open cannot accept environment variables for the initial root pane in the inspected Herdr version. Cockpit passes context/workspace variables explicitly when it creates subsequent tabs/panes through supported env parameters. Existing terminals and panes launched directly from the Herdr TUI cannot be retrofitted or assumed to inherit them. The setup result states this limitation and never silently closes the initial pane.

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
- browser runtime verification uses `cockpit serve` against an explicit run-owned Herdr session; `--test-mode` is only an unavailable-state fixture, not live compatibility proof;
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

## 11. Next-level delivery and code maintenance

The full plan is `planning/next-level/README.md`. Begin with a separately scoped maintainability increment, then implement configuration/contracts, real extension-pane detection/rendering, local worktree/companion setup, context reading/comments/paste, and source downloads. Full graphical review and previously deferred capabilities remain separately selectable stories with dependencies and tests.

Keep this a personal tool that is easy to change in code. Prefer clear owning modules and small interfaces over a generic plugin/workflow framework. The cleanup plan names concrete seams and a code-tweaking guide; it must preserve current Herdr runtime behavior before feature additions.

Prime the cleanup with deterministic quality infrastructure (CLEAN-05): pinned test/coverage/complexity tools, per-function CRAP with a proposed new-code ceiling of 8 and preferred target of 6, changed-code mutation tests, and machine-readable failures for implementation agents. Establish a reviewed legacy baseline rather than requiring a rewrite. Coverage mapping and tool failures must be reported as incomplete, not passed. Detailed policy and adoption probes live in `planning/next-level/11-quality-gates.md`.

## 12. Stability before expansion

The reported terminal redraw flicker reopens the custom protocol-22 and renderer choice. Establish a temporal reproduction and a reliable daily-use build before cleanup or feature expansion; Kitty support remains desired but must pass the same stability gate. `planning/next-level/13-terminal-stability.md` compares repair and stable-compatible alternatives without presuming a downgrade is already verified. `research/next-level-existing-code-review.md` records additional current-code failures and planned corrections. No feature, renderer repair, or installed-server change is part of this planning task.

The subsequent user decision selects stable Herdr as the default and parks protocol 22, while preserving mouse click/input/scroll support. Current evidence separates physical direct-attach pointer delivery, CLI-injected SGR, normal xterm.js wheel delivery, and structured app-mode pointer routing; do not collapse these into a single “mouse unsupported” result. Follow `planning/next-level/13-terminal-stability.md`; do not silently disable mouse. The final implementation order and verifiable goals are in plans 15 and 16.

The user downgrades Herdr to stable before implementation begins. Astra runs inside protected session `default`; it must leave that server/session and its focus/layout untouched. BOOT-01 restores actual frontend transport compatibility before frontend-dependent smoke tests. All runtime test effects target explicit run-owned disposable sessions. Start implementation from `IMPLEMENTATION_HANDOFF.md`.
