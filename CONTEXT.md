# Cockpit Architecture Context

Status: architecture reference. The Herdr client, workspace setup, Cockpit-owned tab placement, virtual Files/Review viewers, per-tab disposable Browser sessions, comments/paste, and Library/provider flows are implemented. This file describes ownership and behavior, not a claim that every acceptance scenario has been verified; current rules are recorded in `DECISIONS.md`.

All filesystem roots, executable locations, Herdr endpoints, and provider settings are configurable. Example absolute paths are intentionally omitted.

## 1. Product vision

Cockpit is a personal, local-first developer cockpit for supervising persistent coding-agent sessions and organizing the context used to work on bounded engineering tasks.

The product is optimized for one developer on a trusted workstation. It is not a multi-tenant service and does not make coding agents autonomous background operators.

Cockpit follows conventional engineering workflows:

- discover a local repository;
- create a task worktree or open an existing directory;
- attach a companion context directory;
- supervise persistent Herdr sessions and agent terminals;
- gather static issue, review, wiki, and telemetry context;
- inspect, test, and review changes locally;
- destroy owned workspace resources when the task is complete.

Cockpit is intended to become the developer's primary way of engaging with local projects. Maintainable code and easy behavior changes matter more than distribution or product generality. Distribution to other users, formal accessibility compliance, and remote access are elective later concerns.

## 2. Product domains

Cockpit brings together several provider-neutral product domains:

- **Forge** — repositories, branches, pull/merge requests, review material, and worktree provenance.
- **Issue Tracker** — issues, comments, labels, parent relationships, references, and freshness checks.
- **Wiki** — pages and related documentation fetched as static context.
- **Telemetry** — optional static log and trace context.
- **Herdr Client** — a graphical client for Herdr sessions, following Herdr semantics and authority.

Forge, Issue Tracker, and Wiki are domain contracts served by configured provider adapters. Source import, refresh, and copied local-folder Library items are implemented; the companion repository-snapshot action has been replaced by the Library folder flow.

## 3. Runtime architecture

### 3.1 Shared core

The reusable Cockpit core and CLI are implemented in Rust. Package boundaries are:

1. **Protocol** — versioned request, response, error, and event types shared by clients and servers.
2. **Application core** — workspace lifecycle, provider ingestion, freshness, context discovery, authorization rules, and idempotency. It must not depend on Tauri, HTTP, WebSocket, or socket framing.
3. **Herdr adapter** — Herdr protocol, session selection, snapshots, events, terminal attachment, reconnect, and Herdr-specific identifiers.
4. **Provider adapters** — configured external CLI integrations and normalization into Cockpit snapshots.
5. **OS vault adapter** — `cockpit-secrets` implements the core's credential-vault trait over Linux Secret Service (macOS Keychain compiled only); hosts compose it and the core stays free of OS dependencies.
6. **CLI and gateway hosts** — user-facing commands and `cockpit serve`.
7. **Client adapters** — Tauri IPC/channels for native use and HTTP/WebSocket for browser use.

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

#### Subscription limits

The existing 28 px work-area bottom strip shows compact Codex, Claude, and Copilot balances without adding height. Hover previews every reported window, reset time, source age, and failure state without moving keyboard focus; leaving the strip and popup dismisses the preview. Click, Tab/Enter, or **Subscription limits** in Commands pins the details open until dismissal. Meters show remaining quota, with amber above 80% used and red above 95% used; exactly 80% remains normal and exactly 95% remains amber. The most constrained reported window determines each provider's chip; narrow layouts show one provider and a count. Accounts are anonymous. Missing windows are not inferred, and unavailable or unsupported values are not zero balances.

The shared core quota service reads `omp usage --json --redact --no-extensions` for Codex/Claude and `gh api --method GET copilot_internal/user` for Copilot. It reuses those CLIs' authentication; Cockpit neither reads credentials nor signs in, switches accounts, or redeems credits. OMP may refresh its own tokens while collecting. Copilot requires explicit token-billing flags and credit used/remaining/entitlement fields; legacy premium-request counters are never relabelled as AI credits. This GitHub endpoint is internal and may change.

Requests return immediately and schedule background work. Native and browser hosts sharing `cache_root` reuse a private, allowlisted snapshot under `quota/v1`, guarded by a cross-process lock and a persisted pre-command lease. Each source runs at most once per five minutes, with independent 5/10/20/40/60-minute failure backoff. No OMP cache invalidation is performed. Hidden clients stop requesting updates; there is no active-agent-triggered polling or host timer. Source observation timestamps, not command completion, determine freshness: errors or age over 15 minutes mark retained data stale, and values older than 24 hours are unavailable. Unsafe/unwritable cache paths fail closed rather than starting an independent collector.

Optional `[quota] omp_executable` and `gh_executable` settings default to `omp` and `gh`; `COCKPIT_OMP_EXECUTABLE` and `COCKPIT_GH_EXECUTABLE` override them. Existing `PI_CODING_AGENT_DIR` and `GH_CONFIG_DIR` authentication overrides are inherited by the CLI children.

### 3.3 Herdr authority

Herdr-server is the authoritative state machine for:

- named sessions;
- Spaces/workspaces;
- tabs and real terminal existence/membership;
- PTYs and processes;
- real terminal focus identity and Herdr's own TUI layout;
- agent detection and agent state;
- Herdr-owned metadata (agent-list ordering is client presentation; see `DECISIONS.md`).

Cockpit does not create a competing session registry or duplicate Herdr lifecycle state. Its session mirror caches authoritative snapshots and ordered events. Cockpit separately owns in-memory placement and viewer selection inside each tab: those are presentation state, not Herdr terminal membership or focus authority.

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
- workspace, tab, pane, and agent methods perform lifecycle/focus mutations; Cockpit does not use Herdr geometry mutations;
- terminal attach/read/input operations connect UI panes to server-owned terminals.

The socket transport is newline-delimited JSON. The client must handle request IDs, ordered events, reconnect, stale state, and explicit unsupported-capability errors.

### 4.2 Session selection

The client selects one Herdr named session at a time.

- Startup selects Herdr’s default session when available and provides a session selector.
- Switching sessions detaches old subscriptions/renderers, connects to the new session, loads `session.snapshot`, subscribes to events, and clears stale selection state. Tab placement is keyed by session, server instance and tab id above the workbench; same-instance resync preserves it, while a changed server instance starts fresh.
- Multiple Cockpit windows may connect, but Herdr remains the authority for focus and writable terminal ownership.

### 4.3 Workspace and worktree operations

Cockpit uses Herdr’s worktree API to create and remove owned task worktrees. Opening an existing directory uses `workspace.create` with the exact validated path. It needs no repository selection, accepts plain and nested directories, and never initializes Git or switches branches. Optional Git discovery supplies metadata only. Opened directories are borrowed and cannot be removed by Cockpit teardown.

Repository discovery enumerates supported repositories beneath a configured default root. The root is not hard-coded.

Branch and workspace-location templates are configurable. Task artifact metadata may contribute to the derived branch or label:

- an issue can provide the task type and identifier;
- a review artifact can already identify its source branch;
- explicit user values override derived values.

New worktree setup performs this sequence:

1. enumerate and select a repository;
2. resolve a typed task/artifact;
3. ask Herdr to create the worktree workspace;
4. create the Cockpit-owned companion context resource;
5. record the companion association in a Cockpit-owned provenance manifest and pass context environment to explicitly created new Cockpit tabs/panes;
6. optionally hydrate explicitly requested context;
7. return the selected Herdr session/resource.

Configured repository actions run without a per-operation consent checkbox. Cockpit does not set Herdr's Git `trust_repository` override. Setup retains its operation identity after uncertain dispatch and reconciles before any further mutation.

The durable global Context Library, not a cache, survives workspace destruction. Destruction removes only proven owned resources; Library items and unrelated resources are never removed with a workspace. Library operations do not inspect, import, or modify the legacy `<state_root>/sources` cache.

Independent reflink snapshots are preferred where available. Normal copies preserve correctness and report the fallback. Hardlinks and Git alternates must not couple writable context files to their originals.

## 5. Herdr client UI

### 5.1 Implemented foundation

The implemented Cockpit foundation includes:

- a real Herdr session mirror with schema-gated socket connectivity;
- terminal pane read, attach, input, and output through Herdr;
- Herdr-semantic hierarchy and focus operations with Cockpit-local terminal/viewer placement;
- workspace creation and context hydration;
- provider-backed source import and refresh;
- the `cockpit serve` browser client path.

### 5.2 Information architecture

The first screen uses the native Herdr TUI as a behavioral baseline:

- a top-level Herdr session selector;
- a scrollable hierarchical **Spaces** section in the sidebar;
- an **Agents** attention queue below Spaces;
- a main view containing tabs for the selected Space;
- real terminals and local Files/Review/Browser leaves arranged in the selected tab's Cockpit-owned split tree.

This is baseline parity, not a permanent imitation target. Cockpit preserves Herdr semantics and authority while deliberately evolving the presentation toward a dense graphical operations workbench.

The UI uses Herdr-native labels such as Spaces, Agents, tabs, and panes. The Herdr API’s workspace terminology remains an internal mapping detail. Agent ordering is blocked, done, working, idle, unknown; newest state change first.

### 5.3 State and interaction

Herdr is authoritative for agent state. The client renders Herdr’s state categories, including blocked, working, done, idle, and other supported states, with transition detail and freshness when available.

Selecting a real terminal records local intent and uses the ordered Herdr focus acknowledgement before enabling input. Selecting a viewer changes local selection and DOM focus only; Herdr remains focused on the tab's last real terminal. Layout selection, Herdr focus identity, attachment/input ownership and DOM keyboard focus are separate state.

A changed external Herdr focus triple selects its real terminal, tab and Space, restoring local zoom if necessary. A local request echo completes only still-current intent; an unchanged focus in a repeated snapshot never steals selection from a viewer.

The client supports Herdr-semantic hierarchy operations: create, rename, reparent, reorder and close Spaces/tabs where supported, and create, rename, close and move real terminals. Cockpit owns positioning within each tab: header dragging swaps at the centre or places at an edge, live dividers resize sibling shares, and zoom is local. These actions never send `pane_resize`, `pane_swap` or `pane_zoom`; Herdr geometry and zoom hints are ignored. Fitting a control-attached terminal still updates its PTY grid.

Layouts are never persisted. First load is a balanced grid in stable pane-id order. Externally added terminals arrive at the full-height right edge; Cockpit splits place the terminal beside the selected leaf using the validated creation receipt. Runtime source selection is separate from placement: selected real terminal, last real terminal, Herdr-focused terminal, then first real terminal.

Only live authoritative membership can prune leaves. Loss of a tab or its final real terminal releases its viewer contexts and stops/cleans its Browser session; stale, disconnected or loading snapshots never imply loss. Durable comments and browser work remain recoverable. Viewers cannot move across tabs or Spaces. Opening the Library leaves layouts unchanged while membership and focus reconciliation continue.

Commands act on the selected leaf; terminal creation and cross-tab terminal moves remain Herdr operations, while cycle, directional focus, swap, resize and zoom use local geometry. Header controls and Commands make actions discoverable. `Ctrl+B` routing is shared across native and browser clients; Esc belongs to content surfaces except when cancelling a drag or restoring zoom from layout chrome.

Herdr-advertised custom commands are discovered from the identity-checked client-shell projection, not from plugin-specific integrations or Cockpit's configuration. Opaque command IDs, direct and `prefix+` binding aliases, descriptions and configured Herdr prefix chords update with the server projection, including config reloads. Supported actions are shell, pane, popup and plugin action; unknown actions are not invokable. Commands lists these runtime actions alongside Cockpit's static registry. Invocation uses `command.invoke` with the confirmed Herdr Space/tab/pane context, revalidating membership and the current advertisement before dispatch.

Cockpit keeps its own `Ctrl+B` prefix. A different configured Herdr prefix routes advertised custom bindings without replacing Cockpit's shortcuts; when Herdr also uses `Ctrl+B`, advertised custom bindings take precedence over colliding Cockpit bindings. Direct custom chords similarly precede local viewer chords, outside text editors and local dialogs. Literal double-prefix passthrough is reserved: `Ctrl+B Ctrl+B` sends a literal `Ctrl+B`, and a Herdr-armed prefix followed by a configured Herdr prefix passes that chord to the focused surface.

Herdr's singleton popup is a centered floating terminal above the unchanged Cockpit split layout, not a pane inserted into that tree. Its server title and cell/percentage size hints determine presentation within the work area. The underlay is inert, underlying terminal input is gated (also while a popup command is pending), and painted panes keep their attachments and geometry. The popup program receives Esc, Enter, Tab and prefix chords; Cockpit does not force-close it or run workbench shortcuts behind it. Authoritative server closure restores DOM focus to the opener if still available, otherwise selected-tab chrome, without a Herdr focus mutation. Disconnection retains the last open popup with disabled input, status and retry rather than interpreting stale state as closure.

### 5.4 Terminal attachment and scalability

Herdr owns every PTY, process, terminal model, and terminal stream. xterm.js owns rendering and input capture only.

- Compatibility requires Herdr protocol 22, schema 1, and the adapter's required methods, not an exact display-version patch. Every painted terminal leaf opens a direct ANSI stream using `TerminalHello` and `ControlTerminal`; no observe attachment or downgrade path remains.
- Herdr's JSON API owns hierarchy, real terminal membership and focus. Cockpit owns placement; public snapshots contain no layout rectangles. Stable `TerminalFrame` messages supply sequence numbers, dimensions, and ANSI bytes for each attached pane.
- Attachment uses fitted per-pane dimensions and measured cell pixels. The first frame must be full; every later sequence must be consecutive, including full repaints.
- A full frame is an ANSI baseline, not permission to reset xterm. Socket framing has one uninterrupted reader with bounded buffering and deterministic shutdown.
- Terminal graphics are parked. Known auxiliary messages are consumed without exposing graphics payloads or disconnecting an otherwise usable text terminal. The image addon is not loaded.
- Text and binary input use stable raw `Input`; wheel/page scrolling uses `AttachScroll`, gated by local control intent and attachment state. Normal xterm.js panes attached to Herdr are observed to receive wheel/scroll events.
- Herdr's per-attachment `MouseCapture` signal enables application mouse handling automatically. Cockpit sends structured `AttachMouse` cell coordinates, and Herdr chooses the application's encoding and rejects reports when tracking is disabled. Mode-off and Shift-drag retain xterm text selection. Idle hover reports and exact pixel coordinates are not forwarded.
- `Shift+Enter` sends a bare line-feed.
- Local xterm enables Kitty keyboard support; stable end-to-end enhanced-reporting behavior still requires TERM-03 evidence.
- Only painted terminals in the active tab keep xterm renderers/subscriptions. Zoom-hidden terminals, inactive tabs and the Library detach without stopping Herdr processes. Files/Review state is retained per source in the layout store; hidden Browser views release captures but keep their managed session.
- Herdr remains authoritative for scrollback and screen state. Reconnect requires a fresh full baseline before consecutive updates.
- Control attachment and permission to send input are distinct: input requires local terminal DOM focus, Herdr-confirmed focus and owned control. Tab switches prepare the incoming painted terminal grid before Herdr focus (first focused-terminal frame or 300 ms) and retain outgoing attachments until the swap is painted.
- Attach failure leaves the pane visible with stale/disconnected state, retry, and resync. It never silently closes the Herdr process.

The client loads hierarchy metadata for all resources. It does not require an xterm.js DOM instance or live output subscription for every pane.

Command/popup metadata and invocation share the native/browser `CockpitClient` session-stream and resource-mutation contract; popup rendering uses the same direct terminal stream with an explicit popup target. Tauri IPC/channels and browser HTTP/WebSocket hosts compose the same core and Herdr adapter. The client-shell decoder targets Herdr 0.9.2's generation-1 surface (`shell.snapshot.v1`, `shell.surface.v1`) in addition to the protocol/schema requirements above; unsupported generation/codecs produce `shell_unsupported`, endpoint identity mismatch fails closed, and connection/handshake failures remain explicit shell status errors. Invocation and popup attach reject retired advertisements or targets with `command_not_available` or `popup_not_open`.

The existing client-shell API requires an active surface subscription at 120×40 cells to obtain popup metadata. This is not a passive metadata observer: it can resize unattached Herdr panes, including a concurrently visible Herdr TUI. That side effect is an accepted constraint of the chosen API, not a claim that Cockpit leaves every Herdr PTY untouched. Direct control-attached Cockpit panes retain their own fitted PTY sizes; Cockpit's split rectangles remain unchanged.

### 5.5 Virtual Files, Review and Browser viewers

Each tab has at most one Files, one Review and one Browser leaf. Opening an existing Files/Review leaf focuses it and changes its requested source; per-source view state, including unsaved comment editor text, survives switching sources and unmounting. Viewers do not launch addons, inspect addon-private state or replace real pane renderers; existing addon panes remain ordinary terminals with no graphical/terminal toggle.

The core's `ViewerService` opens Files/Review contexts from fresh evidence of a real terminal in the same tab. It pins only the requested root and records source cwd, endpoint, tab and Space. Requests use `viewer_id` and `binding_id`, revalidate current tab/endpoint/Space and root filesystem identity, and reject replaced bindings. A later `cd` or source-pane close does not retarget a viewer. Missing contexts require explicit Reopen; closing a viewer or retiring its tab releases the context without deleting durable batches.

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

Context and Library file pickers request one sorted server-side file index rather than walking directories over many UI requests. Git roots use tracked plus non-ignored untracked paths; ordinary folders use a bounded no-follow walk. The browser keeps a small stale-while-revalidate candidate list; the gateway also persists path-only hints under the configured cache root (`COCKPIT_CACHE_ROOT`, `cache_root`, then `$XDG_CACHE_HOME/cockpit`). Every open re-enumerates the authorized root, and cached paths never grant access or supply file contents.

Repository catalog scans are cached only for read-path authorization, with a mutation generation and stale-while-refill window; setup and teardown continue to perform fresh checks. Review reuses an in-memory snapshot only when its Git revision tokens and comparison identity still match, and revision tokens stream Git output into a digest rather than retaining large diff output. Persisted Review snapshots/files use a versioned, viewer-bound cache namespace; recognized pre-viewer cache files are removed without deserializing their incompatible payloads. Malformed current-format entries still report errors.

Cockpit owns run-local GUI comment batches and a read-only local Git Review model with staged, unstaged, branch and untracked scopes, explicit revisions and side-aware anchors. It does not mutate Git or post provider comments. Comment owners are tagged `viewer` or `legacy_pane`; source identity and authorization remain mandatory for saves, reattachment and delivery. Accepted batches and paste receipts survive view changes during the owning runtime's run, not its next startup.

Browser is a tab-local leaf with an independently managed Chromium session per Herdr tab. Views, input, captures, drafts and feedback are isolated by the tab association key. Hiding by tab switch, zoom or Library releases capture resources but does not close the browser. Explicit close, tab/final-terminal retirement and owning-runtime shutdown stop the process and remove only identity-proven profile/workspace/config artifacts; cookies, logins and site storage are disposable. Failed cleanup stays visible with retry, and cannot delete unrelated or unproven resources.

Leaf creation/reopening uses `OpenFresh`: a surviving tab session is stopped and cleaned, never adopted, before starting at `[browser] default_url` / `COCKPIT_BROWSER_DEFAULT_URL` (default `about:blank`). The configured URL is validated at load. CLI `Open` and an existing leaf's Reconnect preserve attach/new-page behavior rather than restoring a closed leaf's navigation.

Pane-local state is ephemeral. After acquiring the exclusive browser owner lock, and before publishing its owner socket or constructing Review/comment services, the owning runtime stops proven leftover managed sessions and clears `browser/` except `owner.lock` and `owner.sock`, then clears `comments/` and `review/`. Observers joining the same owner never reset state. Reset preserves the lock inode, does not follow symlinks or cross devices, and fails startup without wiping browser state when process shutdown cannot be confirmed. Library content, credentials, configuration, project operations and real Herdr sessions/terminals are untouched.

Closing a Browser, reopening it fresh, retiring its tab or shutting down its owner discards that association's annotation drafts, pending captures, feedback and delivery receipts along with its disposable profile. There is no saved-tab/archive recovery API, saved-work close guard, or Saved-before-tabs / Saved-browser-work / Review-items cleanup panel. Current-run annotations and comment drafting remain available; Git Review remains a normal viewer. Cleanup failures stay inline and retryable.

The CLI uses `cockpit browser open|status|close|feedback --tab <tab-id>` with explicit Herdr session/socket, or `--current` to resolve the calling real pane's tab. Feedback and exact-ID acknowledgement address current tab work only; detached `--legacy` addressing is removed.

### 5.6 Errors, settings, and accessibility

Errors are inline on the affected Space, tab, pane, or operation. The UI preserves last-known state, explains the failed operation, and offers retry/resync without disabling unrelated resources. Toasts may supplement inline errors.

There is no settings UI initially. Durable configuration uses a config file; environment variables and one-off command options override it.

Accessibility is best effort for this personal proof of concept, not an acceptance gate for broader distribution. The UI should still preserve keyboard operation, visible focus, meaningful labels, non-color-only state, and actionable error text where practical.

## 6. Terminal environment

Existing Herdr-provided metadata is inherited. Worktree create/open cannot accept environment variables for the initial root pane in the inspected Herdr version. Cockpit passes context/workspace variables explicitly when it creates subsequent tabs/panes through supported env parameters. Existing terminals and panes launched directly from the Herdr TUI cannot be retrofitted or assumed to inherit them. The setup result states this limitation and never silently closes the initial pane.

Cockpit does not automatically launch or configure OMP. The developer starts agents manually. Herdr’s own integrations report agent state to the presentation layer.

Cockpit places no secrets in snapshots, generated environment values, Library files or configuration. A pasted provider API token or PAT (Jira, Confluence) may be stored in the OS vault, one item per configured provider instance. The UI can set, replace and remove it but never read it back; a stored token is injected only into that provider CLI's child environment, pinned to the configured site, and is used for Cockpit's own authenticated HTTP (Jira attachment downloads). With no stored token, or an unavailable vault, the provider CLI's own login applies. Passkeys/WebAuthn and OAuth sign-in are not storable secrets.

## 7. Context ingestion and Library

### 7.1 Provider boundaries

Cockpit pulls and validates freshness. Developers and agents perform remote writes through the provider CLIs or their normal tools. Cockpit does not silently mutate remote tickets, issues, reviews, or wiki pages.

Provider interfaces are capability-based. Issue Tracker and Wiki adapters primarily provide:

- fetch/pull;
- normalization into Markdown/frontmatter;
- freshness/version comparison;
- explicit unsupported-capability errors.

Forge adapters are initially intended for review-assistant context, not complete forge administration.

Adapters use configured executables and safe argument construction. A missing or unsupported executable creates an explicit unavailable capability rather than silently substituting another provider.

### 7.2 Hydration and reference depth

Creation hydration is explicit. Adapters decide what to download and how to transform it. Setup validates and imports only the requested primary artifacts; it does not follow references.

Related items are controlled by one number, `reference_depth` (`LibraryAddRequest.reference_depth`, replacing the former same-repository `hydrate_references` boolean and its 32-asset/2-depth crawl; no alias remains). The API accepts 0 to 5 and rejects anything else with `library_reference_depth_invalid`; the Add dialog offers `Follow references`: Off, 1, 2 or 3 steps. `0` saves only the requested item, `1` adds the items its seeds reference, and each further step repeats one hop. The default is 1 for a single Jira issue and 0 for a GitHub/GitLab/Gitea issue or PR and for a new Jira query; a stored item or follow without the field (everything saved before this feature) reads as 0, so there is no schema bump, and Resolve reports the stored depth of an existing item or follow so re-adding does not silently reset it. A Confluence page add and a Confluence space follow reject a depth above 0 with `source_capability_unavailable`; folder copies ignore it.

References come from three places. Jira issues emit structured `parent`, `subtasks` and `links` frontmatter fields (a link is `<relation text> <KEY>`, for example `blocks OPS-2`). Description and comment text contribute Jira keys (recognized only in Jira content and resolved against the same configured site, with word-boundary rules so `OPS-12-fix` or `xOPS-1` are not keys) and URLs; other providers contribute URLs from their body. A URL is followed only when it resolves under a configured provider instance: Jira issues, Confluence pages, and GitHub/GitLab/Gitea issues and pull/merge requests, with authority and canonical identity validated exactly like a direct import. Confluence space URLs, unsupported hosts and unconfigured instances stay plain links. Every reached item is expanded at the next step by the same rules whatever its provider, so a Jira issue can lead to a Confluence page that leads back to another issue within one depth budget. At most 64 references are kept per asset.

Traversal is breadth-first with eight fetches in flight. Items are deduplicated by canonical Library identity (provider, instance, resource type, canonical id), so cycles and self-references collapse and each item is fetched once; the shallowest route is stored as the item's inclusion reason (one stored per holder: the referencing item, the relation such as `parent`, `comment` or `description`, and the step). Safety caps are internal, not configurable: a single import follows at most 32 related items, 8 MiB and 60 s; a query follow at most 100 related items, 16 MiB and 180 s. The seeds are saved before traversal starts. A provider failure, a cap, or cancellation yields a Partial report row naming what was not saved, never fails or removes the seeds or previously saved related items, and never authorizes a drop (see below).

A single import stores its depth on the seed item (`LibraryItemSummary.reference_depth`) and re-traverses at that depth on each explicit refresh of that item, using its stored references. A refreshing add can change or clear the depth; `Keep in Library` at depth 0 keeps it. Related items of a single import are saved with a `Manual` ref plus an `included_by` reason held by the seed: they remain in the Library when the seed is removed, and when a complete refresh no longer reaches one it loses only that reason, never its `Manual` ref. Storage keeps one reason per holder; Library Details shows each distinct rendered reason once as `Included via KEY · relation · step N` (identical rendered reasons are deduplicated in the UI), and a seed shows `Related depth`.

Provider failures leave successful assets in place, record a per-item status, and allow the workspace/session to proceed when the primary resource is available.

### 7.3 Durable Library and Space copies

The global Context Library is the durable, Cockpit-owned source for provider snapshots. It is stored under the configured `library_root`, independently of any Herdr session, Space, or companion. Adding or refreshing an item updates the Library; it does not automatically fan out changes to companion copies.

Space adds save the Library item first, then copy or reflink it into a freshly verified companion. Setup passes its prevalidated primary and linked assets into one Library operation without fetching them again; every item and pending attempt becomes durable before any companion copy starts. A companion failure leaves all saved items and their durable Space-add attempts; setup reports `source_sync_conflict` and can resume by copying saved content without asking the provider again. A later Library refresh never silently updates a Space copy. In the UI, a live target Space (the Library view's selected Space, or a Context pane's own Space) adds `Add to <Space>` to Library item headers and menus and a `Library and <Space>` destination to the Add dialog, whose progress shows the Library and Space phases separately. A companion's `Resources` lists that Space's Library copies with read-only states, failed adds first with `Retry adding to <Space>` (copying the saved item, never fetching again) and `Dismiss`; its toolbar button reads `Resources · N behind` when copies are behind. `Update` and `Restore from Library` act only on selected `Library newer` or `Missing in Space` rows in that Space; `Update all (N)` counts those rows and skips edited copies. `Replace with Library version…` and `Remove from this Space…` require confirmation bound to the listed path hashes; a stale confirmation conflicts without changing the file. Copies marked `Removed at source`, `Not in Library`, or `Not linked` are not updated; removing a Library item leaves its Space files intact.

`SourceService` is fetch-only: provider lookup, bounded metadata/fetch, setup's short-lived `RecentReads`, and bounded reference traversal (`collect_related`, which returns assets without persisting them). `LibraryService` is the persistence authority. The legacy pane-scoped source import/list/refresh transports are removed in favor of Library operations and the explicit Space list/add/attempt-dismiss transports.

The old `<state_root>/sources` cache is inert: Cockpit never reads, imports, reports, modifies, or deletes it. There is no migration. It remains on disk for manual user removal after confirmation.

Local folders are copied into the Library from a typed absolute or `~` path; there is no live link, two-way sync, or native folder picker. Capture rejects a directory that overlaps the Library, a Space companion, state, or worktree root. Git roots use tracked and untracked non-ignored files; plain directories use regular files. Nested `.git`, symlinks, special files, hardlinks, and native executables are excluded and counted. Fixed build/dependency exclusions apply. Paths are byte-wise sorted, and file/byte limits keep a sorted prefix with a `partial` result. The source is not changed. Refresh is an explicit re-copy: Library bytes change only after confirmation if a Library file was edited; existing Space copies remain untouched and become `Library newer`.

The Library is a human- and agent-readable tree that mirrors each source's own hierarchy; no Cockpit knowledge is needed to traverse it. Provider items live at `<provider>/<host>/<source hierarchy>/<leaf>/<Title>.md`: Confluence `confluence/<host>/<KEY - Space Name>/<ancestor titles>/<Title>/<Title>.md` with child pages nested in the parent's directory; Jira `jira/<host>/<PROJ>/<PROJ-123>/<Title>.md`; forges `<provider>/<host>/<owner>/<repo>/issues|merge-requests|pulls/<n>/<Title>.md`. Downloaded attachments sit in the item's `_files/`. Folder copies live under `folders/<Folder Name>`. Segments keep real titles and replace only unsafe characters; a sibling collision appends ` [<id>]`. All machine state (index, journal, staging, trash, locks, operations) is under `.cockpit/`, and a generated root `README.md` explains the layout. A provider item owns only its document and `_files/`; child items and user files in its directory are never touched by its refresh, replacement or removal. A title or ancestor change moves the item by renaming its directory (children move with it) and rewrites descendant paths in one journaled index commit. The flat layout (index schema 1) has no migration: such a Library fails with `library_layout_outdated` and must be deleted and re-added. Index schema 2 is upgraded to schema 3 in place when the Library opens (see the reference model below); that upgrade is one-way, and an older binary reports the schema 3 index as corrupt, so copy the Library before trying a new build on real data.
Issue-like provider snapshots use generated Markdown plus frontmatter fields for known type, status, priority, author, assignee, timestamps, and comment count. The body starts with a title and one-line summary, then optional `## Description` and `## Comments (n)` sections. Each comment is a `### Author · YYYY-MM-DD HH:MM` card with an optional edited time/location, one permalink paragraph, and its body; provider headings are demoted so descriptions and comments cannot collide with document structure. Partial comment lists state shown and total counts.


Folder items retain each file at its relative path. Adding any item to a Space mirrors its Library-relative paths in the companion, so links resolve the same way there, with one manifest entry and content hash per file. Explicit Space update copies new or changed Library files, restores missing files, and removes unedited files no longer in the Library. Edited files are skipped until their listed path hashes are explicitly confirmed; edited Space-only files are preserved and reported. Removal applies the same per-file compare-and-swap protection. A Library move reaches a Space only through its explicit update, which writes the new paths and removes unedited files at the old ones.
Markdown links navigate within their current Library or Space root; links to a Library source URL open the corresponding Library item or its Space copy, and file actions copy Library-relative or absolute paths. Newly created context terminals receive `COCKPIT_LIBRARY_ROOT` alongside `COCKPIT_CONTEXT_PATH`.

A followed Confluence space includes every page the profile can read across its top-level trees, not only the homepage tree; Cloud folders appear as ancestor-only nodes in the hierarchy. Its durable follow record, page count, partial state, and excluded page ids live in the Library index. `Refresh space` pages the space once and fetches a page only when new or when its version, title, or ancestor chain changed. Page metadata, labels, body, and attachment lookups run concurrently; changed pages are fetched with a bounded concurrency of eight, then published in deterministic parent-first order, each as its own crash-safe durable Library transaction. Ancestor metadata drives hierarchy and parent links; a moved or renamed page's directory is renamed to its new place. Pages absent from a complete enumeration are identified as removed at source, not deleted; previously imported snapshots are preserved.

Every Library item carries a sorted set of references. `Manual` covers a manual add, the "Keep in Library" action and setup imports; `Follow{follow_id}` means a followed Confluence space or Jira query holds it; `Space{companion_root_id}` is written when the item is copied into a Space and dropped when that copy is removed. Refs change only through index mutations under the exclusive lock, so a publish or refresh of an existing item keeps its current refs and two follows cannot overwrite each other's membership. An item that loses its last reference is not deleted: it gets a `purge_after` timestamp (tombstone) and shows an `Unreferenced` pill. A live follow drop sets it 14 days ahead; removing the last Space copy or a follow with its items marks it due at once. Adding any reference clears the tombstone. The sweep at the end of every add or refresh purges due tombstones, unless the item is being fetched or has local Library edits (kept and reported). Because membership is a ref, a page that was added manually and later falls inside a followed space now survives removing that space; only items held by nothing else are removed.

A followed Jira query is added from Add to Library by typing JQL, or a bare project key such as `SCRUM`, which means `project = SCRUM`. The query is normalized (whitespace collapsed, a trailing `ORDER BY` removed) and every read is sent as `project IS NOT EMPTY AND (<jql>)`, so jira-cli's implicit default-project filter never applies. Each matching issue is one Library item at its usual `jira/<host>/<PROJ>/<KEY>/<Title>.md`, shared by every follow that matches it. Resolve shows the query, the issue count (`100+ issues` when the first window was full) and a suggested mode. A follow is `live`, whose members mirror the query, or `accumulate`, which only adds and never drops; queries with relative dates (`-7d`, `now()`, `startOfDay()`) suggest `accumulate`. A refresh lists metadata only (key, updated, status, type, assignee through jira-cli plain columns with a unit-separator delimiter) and fetches an issue's content only when its `updated` value differs from the one stored at the last fetch, or when the item is removed, failed or unknown. Listings are windowed by `updated`, 100 rows per call, because jira-cli's offset paging is unusable on Cloud; the total is capped by `library_space_pages`. An accumulate refresh probes from a watermark derived from the members' newest `source_revision` and key-checks members the probe did not return. A live refresh drops a member only after a complete listing: a failed, truncated, cancelled or empty listing never drops anyone (an empty listing with members is reported as partial and kept). A dropped member loses the follow ref and is tombstoned if nothing else holds it. `Stop following` keeps the issues (exclusive members become `Manual`); `Remove query and its items` deletes the members held by that follow alone, refused if any of them has local edits; removing one issue excludes it from every follow that references it. Jira follows are not Space follows: Add to Space rejects them, while individual Jira items can still be copied into a Space. The plain `updated` value follows jira-cli's `timezone` setting, which is only significant for window bounds above 100 results; leave `timezone` unset.

A Jira query follow carries the same `reference_depth` (default 0, stored on the follow), applied after each add or refresh. An item the query lists is a seed; an item reached only through references is related and is held by `Follow{follow_id}` plus an inclusion reason held by that follow, so it counts toward the follow's items and is shared like any member. If the query later lists a related item it becomes a seed and loses the reason. A live follow drops related items no longer reached only after a complete related pass and a complete seed listing; an accumulate follow never drops related items. Any failed, capped, cancelled or incomplete pass keeps every member and adds one Partial row instead of marking the follow partial. Seeds whose content is unchanged are traversed from their stored references, so a refresh fetches only changed seeds plus the related items; a seed stored before depth was enabled is fetched once to record its references. Removing a related item excludes it by Library id and it stays out. Space independence is unchanged: Library refresh never updates a Space copy, Jira follows cannot target a Space, and a Space add of a single item copies the items that add saved.

A Space holding follows shows one aggregate row per follow, with new, changed, edited, and removed-at-source page counts rather than one row per page. Its explicit `Update` can select follows and copies only the selected follow's new or changed pages for that Space; other follows, standalone items, and other Spaces are untouched. Edited Space copies are reported and skipped unless replacement is explicitly confirmed against their current hashes. The Space manifest tracks known pages so removing a page copy does not cause it to be re-added by a later update.

Confluence attachment metadata is captured with page snapshots, but binary downloads are opt-in: individual page actions or an explicit followed-space option. Downloads use the configured CLI's read-only attachment command, a private staging directory, predicted exact-name matching, per-file and periodically monitored aggregate staging budgets (not hard filesystem quotas), regular single-link file checks, safe stored names, and atomic page-plus-attachment publication; overmatched CLI results are discarded. `Remove downloaded` removes only stored binaries. A confirmed Library replacement re-downloads edited attachment bytes only after matching the exact current file hashes; an opted-in follow refresh retries failed downloads. Page body links retain their original relative paths. Companion copies place `document.md` and `attachments/<safe-name>` together; updates remove obsolete unedited files and preserve/report edited files. PNG and JPEG render through the bounded raster-media reader; PDF has no active renderer, and SVG/HTML are never executed as media. Jira issues (single adds and followed queries) capture `fields.attachment[]` metadata (name, size, media type, content link) the same way and show it read-only in the same attachments panel and item frontmatter; a refresh re-lists it because a changed attachment set changes the issue's `updated` time and content revision. Jira attachment bytes are downloadable only with a token stored in Cockpit: jira-cli 1.7.0 has no attachment command, so Cockpit's own same-site HTTP client fetches them (Authorization only to the configured origin; cross-origin redirects are followed without it) into the same staging, budget and publish path as Confluence. Without a stored token, a per-attachment download or an add with downloads fails with `source_credential_required` (`credential_vault_unavailable` if the vault fails) and the UI offers the token dialog; a Jira follow's attachment opt-in still saves the issues and reports a note per row. Attachment listing and download are verified by fixtures and a fake HTTP server only: the Jira Cloud test site's issues have no attachment, so no live download has run, and self-hosted Data Center (Bearer PAT) is unverified.

The former companion repository-snapshot action and its HTTP/Tauri/client transports are removed. Existing `repos/` companion entries remain readable as `Not linked`; Cockpit does not migrate them or inspect the legacy `<state_root>/sources` cache.

### 7.4 Library Markdown format

Provider snapshots in the Library are Markdown with a versioned frontmatter envelope. The fixed envelope includes `schema_version`, `provider`, `resource_type`, `canonical_id`, `provider_instance`, `source_url`, `original_url`, `complete`, `source_revision`, `content_hash`, and `generated`; provider-specific fields and attachment metadata may follow.

`content_hash` is the Library content revision (`sha256:`), not a hash of provenance or presentation metadata. For provider items it covers identity (provider instance, resource type and canonical identifier), title, source revision, body, completeness, and non-empty extra fields and attachment metadata. It excludes URLs, container presentation, diagnostics, and fetch time. The body is normalized provider data for human and agent reading; raw provider payloads are not retained as the canonical snapshot.

### 7.5 Available provider adapters

The Library currently supports GitHub issues and pull requests, GitLab, Gitea through Tea, Jira issues and followed Jira queries, and Confluence pages and followed spaces. Confluence accepts Cloud page IDs/links and Data Center display links from a configured `confluence` CLI profile. A page snapshot includes page body and bounded metadata (space, ancestors, version, editor display name, labels, and attachment metadata); attachment binaries remain not downloaded unless explicitly requested for a page or follow. Confluence URL authority and canonical page identity are checked against the configured instance; a checkout's Git origin does not select or constrain it. Jira and Confluence credentials may be stored in the OS vault (section 6); otherwise the provider CLI's own login or profile applies.

Self-hosted provider behavior is fixture/contract verified, not live validated. `glab`'s host selector cannot express a port, and Jira Data Center wiki-markup descriptions/comments are converted to Markdown by a conservative converter (`jira_wiki.rs`: headings, emphasis, code/noformat, quotes, lists, links, images, tables, rules; unrecognised constructs stay as text). Jira query follows read through jira-cli 1.7.0 `issue list --plain` and are live-verified on Jira Cloud for listing, resolve, add and refresh only; Data Center list and date formats, windowing above 100 results and the real-time 14-day purge are fixture-verified. Confluence Cloud and Data Center protocol paths are covered by synthetic CLI fixtures. The existing read-only Confluence profile exposed no page or attachment in the inspected SD space, so there is no live page/attachment validation. Stored provider tokens are live-verified on Jira Cloud (Basic token through jira-cli import and follow, failing without a token and after removal) and Confluence Cloud (env mode with an empty profile) against a private gnome-keyring, and through the native Tauri commands and capability ACL; Jira Data Center Bearer PATs and macOS Keychain are not verified, glab/gh/tea do not take a stored token, and the status may show `vault_unavailable` after 20 s when no keyring daemon runs. Provider executables, logins, and supported operations vary; unsupported capabilities are reported rather than silently substituted.

Reference-depth traversal was live-verified against a Jira site and a Confluence site in a disposable Library with a cycle fixture: SCRUM-7 to SCRUM-8 by structured link, a comment on SCRUM-7 to SCRUM-8 and to a Confluence page, that page to SCRUM-9, and SCRUM-9 back to SCRUM-7. Depth 0 saved exactly one item, depth 1 three and depth 2 four, each once and without duplicating the seed; the JQL `key = SCRUM-7` follow at depth 1 saved the same three, and an explicit refresh kept membership without duplicates or failures. GitHub, GitLab and Gitea URL expansion is fixture/contract verified only, not live; do not read the Jira/Confluence run as evidence for every forge.


## 8. CLI surface

The `cockpit` CLI provides:

- `cockpit status` to inspect the configured Herdr installation;
- `cockpit serve` to serve the browser client and HTTP API in the foreground;
- `cockpit configuration` to inspect effective non-secret project configuration;
- `cockpit browser` to control a Herdr tab's managed browser via `--tab` or `--current`, and read/acknowledge archived pre-tab feedback via `--legacy`.

Workspace lifecycle and context operations are provided through core and host services, not separate CLI subcommands.

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
- Cockpit-driven provider sign-in flows (OAuth, passkeys) and glab/gh/tea token injection;
- remote browser access, TLS, and multi-user authorization;
- a Cockpit settings screen;
- arbitrary user-entered shell execution from the UI (server-advertised configured commands are supported);
- raw socket forwarding to browsers;
- unbounded file previews;
- formal WCAG 2.2 AA release compliance.

