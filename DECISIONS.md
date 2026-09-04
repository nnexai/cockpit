# Cockpit Architecture Decision Snapshot

Status: confirmed working snapshot. `CONTEXT.md` has been rewritten as the current implementation reference.

This records the decisions made during the architecture refinement interview. It intentionally avoids example filesystem locations; all roots and endpoints are configurable.

## Product shape

- Cockpit remains the umbrella product.
- Forge, Issue Tracker, and Wiki are provider-neutral domains inside Cockpit, not separate products.
- The repository contains at least two major client-side/backend concerns:
  - a Rust Cockpit core and CLI;
  - a Tauri v2 Herdr client with a shared web frontend.
- Provider implementations are staged. Broad capability-based interfaces come first; unsupported capabilities must be explicit rather than silently approximated.

## Runtime and platform

- Linux is the first native target.
- Native distribution uses Tauri v2.
- Browser distribution is a separate deployment of the same static frontend, not a Tauri browser target.
- Native and browser clients share a versioned protocol and injected client interface.
- The native client uses Tauri IPC/channels.
- The browser client uses HTTP/WebSocket through a standalone local gateway.
- Browser/native behavior is verified through protocol contract tests plus a real native smoke test.
- Browser/test operation has an explicit deterministic test mode; normal startup uses Herdr’s default session with a selector.

## Shared core and package boundaries

- The core is reusable and written in Rust.
- Package boundaries are:
  - versioned protocol types;
  - application/core services;
  - Herdr and provider adapters;
  - CLI and standalone gateway hosts;
  - Tauri/native and browser client adapters.
- Tauri handlers remain thin transport adapters; business rules do not live in the UI host.
- The browser gateway is provided by a dedicated `cockpit serve` process, binds to loopback, and initially has no separate browser authentication mechanism. Remote access is out of scope.
- The gateway exposes task-level operations, not raw shell or raw socket forwarding.

## Herdr authority and transport

- Herdr-server is authoritative for sessions, Spaces/workspaces, tabs, panes, PTYs, processes, agent state, and Herdr-owned metadata.
- Cockpit has no separate session/workspace registry. Herdr metadata and provenance are authoritative.
- Cockpit uses Herdr’s documented newline-delimited JSON socket protocol for persistent runtime interaction and long-lived event subscriptions.
- Herdr CLI wrappers remain available for simple one-shot operations, human debugging, setup, and portability-sensitive cases.
- The installed Herdr schema/version is checked at startup. The adapter is schema-gated and maintains explicit capability/compatibility handling.
- Blind undocumented protocol probing is not the design. Runtime discovery means reading the supported schema/capability surface and validating required operations.
- The documented setup command for native OMP integration is a separate bootstrap concern, not part of each workspace creation.
- Herdr named sessions are supported through a top-level session selector. Only one session is selected at a time.
- Switching sessions detaches old streams, connects to the new session, loads an authoritative snapshot, subscribes to events, and discards stale selection state.
- Client state uses an authoritative snapshot plus ordered events. Sequence gaps, reconnects, or stale state trigger resnapshot/resubscription.

## Herdr client UI

### Primary surface

- The first screen uses the Herdr TUI as its semantic and operational baseline, not as a skin to reproduce indefinitely.
- Preserve Herdr’s resource model, hierarchy, focus semantics, attention ordering, terminal behavior, and supported operations before introducing intentional Cockpit departures.
- Cockpit may depart from the TUI where a graphical desktop workbench improves supervision, discoverability, direct manipulation, or ownership clarity. A departure must not create a second authority for Herdr-owned state.
- The sidebar has:
  - scrollable hierarchical Spaces at the top;
  - an Agents section below, acting as Herdr’s attention queue.
- The main area shows tabs for the selected Space and fills available space.
- Spaces and agent/process entries use Herdr-native terminology and identifiers at the UI level where practical.
- The initial Cockpit foundation is the real Herdr mirror plus terminal attach/input, Herdr-semantic focus/layout controls, schema-gated socket access, and a working `cockpit serve` browser gateway.

### State and inbox

- Spaces and agents mirror Herdr entities directly.
- Agent state is Herdr-authoritative: blocked, working, done, idle, and related Herdr states where available.
- State indicators include transition detail/freshness where available and do not rely on color alone.
- The persistent Agents sidebar follows Herdr priority ordering: blocked, done, working, idle, then unknown; equal-priority entries use newest state transition first.
- There is no separate inbox popup in the initial scope.
- Agent history/resumable closed conversations are deferred.
- Inbox/triage mutations are performed through Herdr; Cockpit does not persist local acknowledgement state.

### Navigation and interaction

- Semantic focus, DOM keyboard focus, and writable terminal ownership are distinct state. Cockpit synchronizes them deliberately rather than inferring one from another.
- Clicking a Space, tab, pane, or agent sends the corresponding Herdr focus operation. Confirmed selection follows Herdr’s response/event; agent selection resolves to its owning pane.
- A local pane selection or terminal click records local control intent. An authoritative focus change or ownership loss from another client clears that intent.
- Space hierarchy supports direct tree operations, including selection, expand/collapse, create, rename, reparent, reorder, close, context menus, and drag/drop where Herdr supports them.
- Tab/pane layout editing supports Herdr-backed core operations.
- The interaction authority is Herdr semantics with GUI presentation. Space tree operations support native Herdr mouse behavior where available; pane/layout actions are primarily shortcut-driven.
- GUI controls remain discoverable, but the first shortcut set is limited mainly to Herdr-style navigation. The Herdr magic escape key has priority over GUI shortcuts.

### Terminal attachment

- Herdr owns the PTY, process, terminal model, and client-shell surface. xterm.js owns browser rendering and input capture only.
- Terminal attachment requires Herdr protocol 22 and the generation-1 client-shell endpoint. Cockpit does not use the protocol-20 direct terminal socket.
- One endpoint is shared by every visible pane in a Cockpit surface. `PaneSurface` and `Patch` messages provide authoritative cells, styles, cursor state, pane rectangles, and graphics; Cockpit slices the surface into per-pane xterm renderers.
- The JSON API remains authoritative for hierarchy discovery and resource mutations. Surface geometry and targeted terminal input travel through the client-shell endpoint.
- xterm.js renderers are mounted only for panes visible in the selected tab. Hidden tabs detach renderers/subscriptions while Herdr processes continue running.
- Cockpit fits the shared surface before attachment and sends both character dimensions and measured cell pixels in the endpoint hello.
- Focused xterm forwards text, pointer, and scroll input as targeted Herdr pane events only when local control intent allows it. Herdr performs pane hit-testing and application mouse-mode routing; Cockpit does not force xterm mouse modes or send raw SGR reports at shell prompts.
- Kitty keyboard negotiation remains enabled. A terminal application still controls whether enhanced key reporting is active.
- Cockpit re-encodes Herdr graphics assets and placements as Kitty direct-transmission commands for xterm's image addon. The built-in xterm renderer is intentional: the WebGL addon obscures the Kitty image layer.
- Client-shell endpoints do not expose the previous exclusive writable takeover/loss transitions. Local control intent, semantic focus, and terminal process closure remain independent.
- Attach/reconnect failure leaves the resource visible with stale/disconnected status, retry, and resync; the Herdr process is not silently closed.
- Errors are inline on the affected resource. Toasts may supplement but are not the only error surface.

### Context surface

- Context is a first-class read-only surface associated with the selected Space.
- It can be represented as a dedicated Context tab and may also occupy a pane in a Herdr-supported split layout.
- The browser renders Markdown/frontmatter, navigates the companion tree, searches, and refreshes on file changes.
- Search is delegated to `ripgrep` through a narrow core-mediated allowlisted operation; Cockpit does not implement a custom search engine.
- Safe type-based viewers support Markdown, bounded plain text/log files, and images. Unsafe, binary, executable, or oversized files are refused or opened externally.
- User-created files found in the companion tree are rendered under the same discovery and safety rules.
- Preview bounds are configurable and intentionally generous, not unlimited.

### Accessibility and settings

- No settings UI is planned initially. Configuration is file/environment/launch-option driven.
- Accessibility is currently a best-effort goal for this personal proof of concept rather than an explicit release gate for broader distribution.
- The design should still preserve keyboard access, visible focus, usable labels, non-color-only state, and actionable inline errors where practical.

## Workspace and filesystem lifecycle

- Workspace creation starts by enumerating repositories under a configured default root.
- Creation resolves a typed task/artifact, asks Herdr’s worktree API to create/open the worktree workspace, creates the Cockpit-owned companion context, attaches that context through Herdr metadata/environment, and returns the selected session/resource.
- Workspace lifecycle uses Herdr’s worktree API for creation/open/removal and Herdr worktree provenance for mapping.
- The companion context resource is Cockpit-owned and linked through Herdr metadata; it is not tracked by a separate Cockpit registry.
- Partial provisioning artifacts are allowed. Destroying a newly created space/workspace must also remove its owned companion context.
- Branch and workspace-location templates are configurable and use task artifact metadata where available. A review artifact may already identify its source branch.
- Configuration uses a durable config file with environment-variable overrides and one-off command options.
- CoW cloning is preferred. Normal copying is a correctness-preserving fallback and emits a performance warning.
- Central normalized cache assets are replicated into companion contexts. Sync replaces changed copies atomically.

## Terminal environment

- New Herdr-created terminals receive context/workspace environment through the Herdr session environment.
- Automatic OMP setup/launch is out of scope for now.
- Initial workspace setup creates/registers a Herdr session only; it does not require a predefined three-surface layout.
- Cockpit does not mutate Herdr metadata for its own bookkeeping. Existing Herdr-provided metadata is inherited; Cockpit adds only the context/workspace environment needed for its operations.
- Credentials are not abstracted or managed by the client initially. External wrappers/tools retain responsibility for credential handling. Cockpit must not persist or export secrets through snapshots or generated environment values.

## Context ingestion and snapshots

- Creation hydration is explicit, with bounded typed dependency expansion.
- Automatic expansion may include an immediate parent and recognized local/provider references in descriptions/comments, with deduplication, cycle detection, and depth/count limits.
- Cockpit pulls and validates freshness. Users and agents perform remote writes through provider CLIs or their normal tools.
- Provider adapters decide what to download and how to transform it.
- Normalized snapshots use a versioned canonical Markdown/frontmatter envelope.
- Required frontmatter covers schema version, provider, resource type, canonical identifier, source URL when available, fetched time, and source revision/hash metadata when available.
- Freshness prefers adapter-provided source version/ETag/revision/hash metadata, then canonical content hash as appropriate.
- The current context browser discovers the companion tree plus frontmatter. There is no generated `INDEX.md` or scratchpad in the current scope.
- The first concrete adapter is Gitea issue context via a configurable CLI, with `tea` as the default when available. It includes issue metadata, description, labels, milestones, assignees, comments, timestamps, source identifiers, and recognized bounded references.
- Gitea provider interfaces are capability-detected and return explicit unsupported-operation errors.
- Provider failures leave successful assets in place, record per-asset failure status/retry information, and allow the workspace/session to proceed when the primary resource is available.

### CLI surface

- The initial user-facing CLI provides repository discovery, Herdr session connectivity/mirroring, workspace create/destroy, context hydration hooks, and `serve` for the browser gateway.
- `cockpit serve` is an explicit foreground process serving the static browser frontend plus the versioned HTTP/WebSocket API from one loopback origin, with explicit test mode. Native Tauri access uses Tauri capabilities; separate browser authentication is deferred.
- Provider-specific mutation commands are deferred.

## Deferred scope

- Automatic OMP setup and launch.
- Provider implementations beyond the first Gitea issue pull/freshness slice.
- Full Forge review-assistant ingestion for GitLab/GitHub/Gitea.
- Full Issue Tracker and Wiki provider matrix implementations.
- Agent history and resumable conversations.
- Separate inbox popup/views and advanced inbox title/history behavior.
- Cockpit-managed secrets, passkey integration, and key-store injection.
- Remote browser access and multi-user authorization.
- A Cockpit-owned settings screen.
- Unbounded file previews, arbitrary shell execution, raw socket forwarding, and custom search implementation.

## Known risks

1. **Client-shell compatibility is a hard runtime boundary.** Cockpit requires protocol 22 plus generation-1 endpoint negotiation, fixture-tests the schema, and rejects older Herdr servers before attachment.
2. **Terminal input is endpoint-targeted, not exclusively leased.** Cockpit gates forwarding with local control intent, but a second client-shell endpoint can still send input to the same pane.
3. **Accessibility is intentionally best effort for this personal proof of concept, not a gate for broader distribution.**
4. **No index/scratchpad means context membership is derived from the companion tree and frontmatter; human-created files are displayed but not managed by Cockpit.**
5. **Herdr “Space” UI labels map to Herdr API “workspace” resources.** The adapter must keep this translation explicit.

## Evidence used during refinement

- `CONTEXT.md` in this repository: initial architecture and workflow proposal.
- Tauri v2 documentation: https://v2.tauri.app/start/ and related architecture/configuration/security pages.
- Herdr socket API: https://herdr.dev/docs/socket-api/
- Herdr source repository: https://github.com/SuperCodeAgents/herdr-terminal
- Herdr Agent Inbox source: https://github.com/douglascorrea/herdr-agent-inbox
