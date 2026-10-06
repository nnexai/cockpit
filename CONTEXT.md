# Cockpit Architecture Context

Status: architecture reference. The Herdr client, workspace setup, supervisor orchestration, Cockpit-owned tab placement, virtual Files/Review viewers, per-tab disposable Browser sessions, comments/paste, and Library/provider flows are implemented. This file describes ownership and behavior, not a claim that every acceptance scenario has been verified; current rules are recorded in `DECISIONS.md`.

All filesystem roots, executable locations, Herdr endpoints, and provider settings are configurable. Example absolute paths are intentionally omitted.

## 1. Product vision

Cockpit is a personal, local-first developer cockpit for supervising persistent coding-agent sessions and organizing the context used to work on bounded engineering tasks.

The product is optimized for one developer on a trusted workstation. It is not a multi-tenant service. Coding work can run in supervised workers, with separate, explicit operator authorization for preparation and execution.

Cockpit follows conventional engineering workflows:

- discover a local repository;
- create a task worktree or open an existing directory;
- select relevant Library items and existing repositories for a Space;
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

Forge, Issue Tracker, and Wiki are domain contracts served by configured provider adapters. Source import, refresh, and explicit copied local-folder Library items are implemented. Additional Space repositories are existing local checkout paths, not snapshots.

## 3. Runtime architecture

### 3.1 Shared core

The reusable Cockpit core and CLI are implemented in Rust. Package boundaries are:

1. **Protocol** — versioned request, response, error, and event types shared by clients and servers.
2. **Application core** — workspace lifecycle, durable task/run orchestration, provider ingestion, freshness, context discovery, authorization rules, and idempotency. It must not depend on Tauri, HTTP, WebSocket, or socket framing.
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

The existing 28 px work-area bottom strip shows compact Codex, Claude, and Copilot usage without adding height. Each reported window is visible, including both 5h and weekly limits; duplicate tier/window limits across anonymous accounts show the highest usage. Narrow layouts show one provider's windows and a count of other providers. Percentages and meters show used quota, including Copilot percentages with up to two decimal places. Amber applies above 80% used and red above 95% used; exactly 80% remains normal and exactly 95% remains amber. Hover previews every account's reported windows, used values, reset times, source ages, and failure states without moving keyboard focus; leaving the strip and popup dismisses the preview. Click, Tab/Enter, or **Subscription limits** in Commands pins details open until dismissal. Missing windows are not inferred, and unavailable or unsupported values are not zero usage.

The shared core quota service reads one `omp usage --json --redact --no-extensions` report for Codex, Claude, and Copilot, including Copilot Business. It reuses OMP authentication; Cockpit neither reads credentials nor signs in, switches accounts, or redeems credits. OMP may refresh its own tokens while collecting. Copilot's `copilot:premium` quantities are treated as AI credits without scaling, including OMP's legacy `requests` unit label; unrelated Copilot counters are ignored. OMP is the accounting authority: its JSON does not expose the billing flags needed to distinguish legacy request-billed accounts. Cockpit makes no separate GitHub CLI quota request.

Requests return immediately and schedule background work. Native and browser hosts sharing `cache_root` reuse a private, allowlisted snapshot under `quota/v1`, guarded by a cross-process lock and a persisted pre-command lease. Snapshot schema 2 contains one OMP source and rejects obsolete split-source snapshots while retaining the shared lock. Collection is demand-driven: at most once per five minutes while idle, or once per minute after a completed success while a visible client reports a live working agent in its selected Herdr session. Clients request status every 60 seconds while idle, every 15 seconds while working, and every 3 seconds during collection. Working demand never shortens the five-minute pre-command crash lease or 5/10/20/40/60-minute failure backoff; older snapshots without positive success evidence retain their existing deadline. Hidden clients stop requesting updates, and there is no host timer. OMP's own five-minute report cache (with ±25% jitter in OMP 18.5.0) still bounds source freshness; Cockpit never invalidates it. Source observation timestamps, not command completion, determine freshness: errors or age over 15 minutes mark retained data stale, and values older than 24 hours are unavailable. Unsafe/unwritable cache paths fail closed rather than starting an independent collector.

Optional `[quota] omp_executable` and the higher-priority `COCKPIT_OMP_EXECUTABLE` override remain authoritative. Without either, Cockpit discovers executable `omp` in `PATH`, then `~/.local/bin`, `~/.bun/bin`, `/opt/homebrew/bin`, `/usr/local/bin`, and `/home/linuxbrew/.linuxbrew/bin`, falling back to bare `omp` if absent. This also supports macOS app launches with a restricted inherited `PATH`; no shell startup files are executed. Existing `PI_CODING_AGENT_DIR` authentication overrides are inherited by the CLI child. The obsolete `gh_executable` quota setting and `COCKPIT_GH_EXECUTABLE` override are removed; old TOML quota keys must be removed.

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
4. record exact worktree ownership and the Herdr creation receipt;
5. optionally save prevalidated primary and linked context into the Library and select it for the Space;
6. pass Library and Space discovery environment to explicitly created new Cockpit tabs/panes;
7. return the selected Herdr session/resource.

Configured repository actions run without a per-operation consent checkbox. Cockpit does not set Herdr's Git `trust_repository` override. Setup retains its operation identity after uncertain dispatch and reconciles before any further mutation.

The durable global Context Library, not a cache, survives workspace destruction. Destruction removes only proven owned resources; Library items and unrelated resources are never removed with a workspace. Library operations do not inspect, import, or modify the legacy `<state_root>/sources` cache.

Spaces do not copy context files or create companion directories. Agents read the live Library and existing checkout paths. The Library is Cockpit-managed and read-only by convention, not a sandbox; writable Space Notes have their own durable Markdown store.

### 4.4 Durable Space Notes

Notes is a Cockpit-local workarea, opened from the header or Commands without creating a Herdr pane or changing Herdr focus/layout. Scratchpad, Todos, Kanban and Decisions share the same core operations through the browser, native client and `cockpit-cli`; board-item comments are durable Markdown records, separate from ephemeral Files/Review comment batches.

The default Linux root is `$XDG_DATA_HOME/cockpit/notes` (`$HOME/.local/share/cockpit/notes` when unset), configurable with `notes_root` or `COCKPIT_NOTES_ROOT`. Keep it outside disposable checkouts, the Library and Cockpit's ephemeral pane-state roots. Each Notes UUID owns `scratchpad.md`, `todos.md`, `decisions/<decisionId>.md` and `comments/<todoId>/<commentId>.md`; registry and lock metadata live under `.cockpit/`. Opening an unbound Space never creates files. Creation and attaching an existing UUID are explicit.

Space bindings use the live endpoint boot identity, session and Space ID, never labels or checkout paths. A Herdr restart leaves old content intact but requires explicit attachment; transferring a binding is confirmed. Pinned `--notes UUID` content operations need neither Herdr nor a running Cockpit owner, so agents resolve once and retain that identity throughout a task. The UI's command examples also pin the configured Notes root.

Todos are source Markdown, not a second database. Stable hidden IDs and optional open-lane metadata connect the board to the same tasks. Done derives from the checkbox; reopening restores the remembered open lane. Moves preserve file order rather than inventing ranks, and unboarding keeps the todo and its comments. Decision replacement creates a new record without rewriting its predecessor. Decision recorded timestamps and comment creation/author metadata are preserved during edits.

Writes use bounded no-follow reads, stable advisory lock inodes, revision checks and atomic publication. Stable task IDs use item revisions; unadopted/ambiguous tasks use line references tied to the full document revision. Surgical Markdown edits preserve unrelated bytes and refuse unsafe boundaries. External editors that ignore advisory locks can still race after the final revision check; this is not a filesystem-wide compare-and-swap guarantee.

Editors keep drafts separately by Notes UUID and record, with bounded best-effort browser storage and visible storage failures. Tab/Space changes and closing Notes do not silently discard those drafts. Conflicts require explicit resolution. An unknown write outcome requires reading saved state and acknowledging the possible duplicate before another write; it is never automatically replayed. Pointer and keyboard board gestures capture their source revision and cancel on source changes, focus loss, Notes closure or Space changes.


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
- real terminals and local Files/Review/Browser leaves and widget docks arranged in the selected tab's Cockpit-owned split tree.

This is baseline parity, not a permanent imitation target. Cockpit preserves Herdr semantics and authority while deliberately evolving the presentation toward a dense graphical operations workbench.

The UI uses Herdr-native labels such as Spaces, Agents, tabs, and panes. The Herdr API’s workspace terminology remains an internal mapping detail. Agent ordering is blocked, done, working, idle, unknown; newest state change first.

Sidebar Git status is read by Cockpit from the Space's checkout, with a pane folder used only as a read-only fallback. Primary rows keep branch and ahead/behind counters together on the second line; worktree children keep borderless actions immediately after their name. Counts describe local remote-tracking refs since the last fetch, not current remote state. Detached HEAD, absent upstream, missing tracking refs and failed reads remain distinct.

Pull (fast-forward only) and Push to the tracked upstream are Space-scoped actions in the row, its menu and Commands, with no assigned key bindings. Row actions do not select that Space. Only a fresh Herdr-reported checkout grants write authority; a pane-folder fallback does not. The shared core revalidates checkout identity, branch, upstream and effective remote destination after acquiring the checkout's action reservation.

Pull fetches the configured upstream without allowing configured fetch mappings to overwrite local branches, then fast-forwards only; it never creates a merge commit, rebases or autostashes. Push uses an explicit branch-to-upstream refspec and disables mirror/force behavior while retaining ordinary Git credentials and URL aliases. Results refresh status immediately. A confirmed result shows only a brief check in the row's action; refusals, not-run and unknown results stay inline under the affected row until dismissed and are echoed in the workbench indicator only when that row is not visible. An unknown result keeps the row's Pull/Push off until dismissed. Ambiguous spawned or transport failures require inspection in a terminal, never an automatic retry or a claim that nothing changed.

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

Command/popup metadata and invocation share the native/browser `CockpitClient` session-stream and resource-mutation contract; popup rendering uses the same direct terminal stream with an explicit popup target. Tauri IPC/channels and browser HTTP/WebSocket hosts compose the same core and Herdr adapter. The client-shell decoder reads generation-1 full surfaces (`shell.snapshot.v1`, `shell.surface.v1`) and negotiates Herdr 0.9.3's optional `endpoint.surface-delta.v1` only when advertised. Full surfaces establish the baseline and remain supported for peers without the optional codec. Deltas require matching boot identity, baseline projection/surface revisions, grid dimensions and popup provenance before metadata advances. Unsupported generation/codecs produce `shell_unsupported`, endpoint identity mismatch fails closed, and connection/handshake failures remain explicit shell status errors. Invocation and popup attach reject retired advertisements or targets with `command_not_available` or `popup_not_open`.

The existing client-shell API requires an active surface subscription at 120×40 cells to obtain popup metadata. This is not a passive metadata observer: it can resize unattached Herdr panes, including a concurrently visible Herdr TUI. That side effect is an accepted constraint of the chosen API, not a claim that Cockpit leaves every Herdr PTY untouched. Direct control-attached Cockpit panes retain their own fitted PTY sizes; Cockpit's split rectangles remain unchanged.

### 5.5 Virtual Files, Review and Browser viewers

Each tab has at most one Files, one Review and one Browser leaf. Opening an existing Files/Review leaf focuses it and changes its requested source; per-source view state, including unsaved comment editor text, survives switching sources and unmounting. Viewers do not launch addons, inspect addon-private state or replace real pane renderers; existing addon panes remain ordinary terminals with no graphical/terminal toggle.

The core's `ViewerService` opens Files/Review contexts from fresh evidence of a real terminal in the same tab. It pins only the requested root and records source cwd, endpoint, tab and Space. Requests use `viewer_id` and `binding_id`, revalidate current tab/endpoint/Space and root filesystem identity, and reject replaced bindings. A later `cd` or source-pane close does not retarget a viewer. Missing contexts require explicit Reopen; closing a viewer or retiring its tab releases the context without deleting durable batches.

Additional selected repositories must still belong to the current configured repository catalog before Files grants access. Removing a configured root revokes that authority without deleting the persisted selection; it remains diagnosed for recovery. The terminal's own freshly verified checkout is a separate source of authority.

The bound Context Library retains its issued root and supports comments, search and media through that binding. The standalone global Library remains unbound and comment-free. Context comment messages identify the authorized absolute file path, so identical relative filenames in the Library, an additional repository and the working checkout cannot be confused.

The Context browser:

- enumerates the live Library and explicitly authorized local checkout roots;
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
The owned Playwright CLI daemon's `browser.initPage` policy derives its UA from that running browser's `Browser.getVersion.userAgent`, changing only `HeadlessChrome/` to `Chrome/`; it pins no version. The policy gates the initial page, later CLI-created pages/popups, and embedded child frames including nested cross-site frames before navigation, and remains active while the inline view is hidden because it belongs to the daemon, not the capture helper. Browser executable/headless mode, launch/security/viewport settings, JPEG capture, and fresh-open versus existing-session attach semantics remain unchanged. This does not establish general anti-bot acceptance, video/audio/DRM correctness, or performance parity.

Leaf creation/reopening uses `OpenFresh`: a surviving tab session is stopped and cleaned, never adopted, before starting at `[browser] default_url` / `COCKPIT_BROWSER_DEFAULT_URL` (default `about:blank`). The configured URL is validated at load. CLI `Open` and an existing leaf's Reconnect preserve attach/new-page behavior rather than restoring a closed leaf's navigation.

Pane-local state is ephemeral. After acquiring the exclusive browser owner lock, and before publishing its owner socket or constructing Review/comment services, the owning runtime stops proven leftover managed sessions and clears `browser/` except `owner.lock` and `owner.sock`, then clears `comments/` and `review/`. Observers joining the same owner never reset state. Reset preserves the lock inode, does not follow symlinks or cross devices, and fails startup without wiping browser state when process shutdown cannot be confirmed. Library content, credentials, configuration, project operations and real Herdr sessions/terminals are untouched.

Closing a Browser, reopening it fresh, retiring its tab or shutting down its owner discards that association's annotation drafts, pending captures, feedback and delivery receipts along with its disposable profile. There is no saved-tab/archive recovery API, saved-work close guard, or Saved-before-tabs / Saved-browser-work / Review-items cleanup panel. Current-run annotations and comment drafting remain available; Git Review remains a normal viewer. Cleanup failures stay inline and retryable.

The CLI uses `cockpit browser open|status|close|feedback --tab <tab-id>` with explicit Herdr session/socket, or `--current` to resolve the calling real pane's tab. Feedback and exact-ID acknowledgement address current tab work only; detached `--legacy` addressing is removed.

#### Trusted agent widgets

Widgets are run-local visual companions to a terminal conversation, not Library items, Herdr resources or Browser sessions. The shared frontend renders trusted HTML in an active, opaque-origin iframe inside a Cockpit-owned dock leaf; multiple widget IDs share one dock per tab. Publishing opens or updates the dock without changing source/tab selection, Herdr focus or terminal input. Widgets and user-removal tombstones live only in the owning runtime's memory, never on disk, and retire with confirmed tab/final-terminal loss or owner shutdown.

The private owner runtime (`BrowserRuntime`, shared with the existing Browser service) owns `WidgetService`. CLI publication connects to its socket rather than starting another owner. Browser HTTP/WebSocket and native Tauri commands/channels expose equivalent content, select, remove and ordered snapshot/event paths. Fresh Herdr endpoint/session/tab/Space evidence resolves the destination; a Herdr caller's current real pane supplies source identity even when explicitly targeting another tab. An external caller must specify `--pane`, `--tab` or `--space` with `--herdr-session` and `--herdr-socket`; explicit targeting does not invent an agent source.

Showing the same `(session, tab, id)` replaces content in place; distinct IDs coexist (at most eight live per tab). Identical bytes and content kind return `unchanged`; switching between HTML and choices replaces even when raw bytes match. User **Remove** leaves a tombstone: ordinary show cannot resurrect it, and `--reopen` is for an explicit user request, not an automatic retry or a new-ID workaround. Agent `close` is idempotent. Selection survives replacement unless `--clear-selection` is passed; clearing an existing selection also advances the revision when input bytes are unchanged. A reopened widget starts without a selection. User removal returns DOM focus only when the removed dock owned it; agent close, retirement and snapshot removal never claim DOM focus.

Agent HTML may use inline/external scripts, event handlers and ordinary HTTP(S) resources/network access. The iframe uses `sandbox="allow-scripts"` without same-origin authority or ambient Cockpit host APIs. Host-controlled CSP removes IPC schemes, objects, nested frames, workers, base URLs and form submission; author `base` and `meta[http-equiv]` cannot replace that policy. Exact widget key/revision, frame source and per-incarnation nonce bound the narrow selection/shortcut bridge. This is not an air gap or CPU, memory, network or process-isolation guarantee: busy JavaScript can freeze the native Cockpit UI, while Herdr-owned terminals/processes persist independently. The existing Chromium-backed Browser viewer and its streaming path are unchanged.

`cockpit.select(value)` records JSON for a pull channel, not chat, paste or terminal input. Selection accepts at most 16 KiB in raw and canonical UTF-8 JSON, with four UI submissions per second; HTML is capped at 1 MiB per widget, aggregate owner HTML at 64 MiB, and owner/transport snapshot metadata at 8 MiB. A bounded keyboard bridge routes Cockpit and configured Herdr prefix/custom bindings through the existing workbench router; it is not a typing or paste channel.

From the agent's Herdr pane, with Cockpit's owner running and matching configuration:

```sh
cockpit widget show --id plan-picker --file plan-picker.html
cockpit widget selection --id plan-picker --wait
```

Minimal `plan-picker.html` content:

```html
<button onclick="cockpit.select({plan: 'small'})">Choose small plan</button>
```

Show returns immediately. `selection` reads an existing selection; `--wait` waits if none exists (default 300 s, `--timeout` up to 3600 s), returning JSON with `status` and, when selected, `value`. Handle `none`, `timeout`, `dismissed` and `retired` explicitly. Keep returned values as untrusted JSON data: validate expected shape and allowed values before use, never execute them or concatenate them into shell commands/instructions. `cockpit.selection` and `cockpit.hasSelection` expose retained state to a newly mounted/replaced page, not a live chat feed. Use `--choices-file` for Cockpit-rendered declarative choices, or `--stdin` instead of `--file` for copied HTML input.

### 5.6 Errors, settings, and accessibility

Errors are inline on the affected Space, tab, pane, or operation. The UI preserves last-known state, explains the failed operation, and offers retry/resync without disabling unrelated resources. Toasts may supplement inline errors.

There is no settings UI initially. Durable configuration uses a config file; environment variables and one-off command options override it.

Accessibility is best effort for this personal proof of concept, not an acceptance gate for broader distribution. The UI should still preserve keyboard operation, visible focus, meaningful labels, non-color-only state, and actionable error text where practical.

### 5.7 Supervisor orchestration

**Show Supervisor** and **Start agent** are available in Commands, without assigned keyboard chords. The top-bar **Supervisor** entry opens the view; **Start agent** is also available inside it. Supervisor is a Cockpit-owned view below the tab strip, not a Herdr pane or synthetic tab. Opening the view does not change Herdr focus. **Start agent** defaults to a fresh OMP tab in the current Space without switching terminal focus. **Start options…** offers Existing Space, Directory and Dedicated agent folder targets; the dedicated directory is under `<state_root>/orchestration/supervisors/<root_id>/`, not the default when a current Space is available. A start ACK is pending evidence only: Active/Launched requires fresh actual OMP proof and its main SDK-session binding. A working OMP is valid; an interactive-ready label is not mandatory.

- Give work through direct conversation in the supervisor's terminal; Supervisor has no bottom task input. The supervisor manages delegation, preparation, execution and result review; only genuinely missing decisions are escalated to the user as **Needs you**. Task state projects canonical Markdown into Queued, Setup, Ready, Working and Review, with checked tasks under Accepted; blocked is an attention overlay, not a stored lane.
- **Agents** is a secondary delegation forest: multiple supervisor/adopted roots, their workers, actual internal OMP subagents and separately identified unmanaged Herdr agents. Parentage is independent of Space/tab location. Internal subagents have lifecycle telemetry and no separate Herdr pane.
- **History** separates durable reports/messages, authorization provenance and delivery receipts from fresh Herdr observations. **Diagnostics** exposes exact plans, bindings and receipts. Recovery and advanced operator intervention remain available without making routine approval the normal task flow.

The supervisor creates tasks and proposes workers without blocking on worker completion. A proposal targets an explicit repository worktree, an Open path or an existing Space; cwd/current Space is not a repository routing rule. Preparation reuses the existing project plan, operation identity, generation and ownership receipts. The authorized root reviews the exact setup plan and issues a single-use **Prepare** grant for that revision. This authorizes setup, worker launch in a new tab without stealing focus, and bounded read-only initialization—not coding execution. The worker pulls its brief and reports **Ready** with an exact work plan. The root reviews that plan and issues **Execute** bound to its exact revision, retaining the initialization receipt. Changed plans require fresh review; normal preparation and execution do not require operator confirmations.

Supervisor authorization is limited to the actual main OMP session of a freshly bound, active, top-level Supervisor or Adopted root and only its strict descendant Worker runs. Workers and internal subagents cannot self-authorize or control siblings, ancestors or unrelated roots. Grants record `origin=supervisor` and the actual authorizing root/session separately from browser/native operator provenance; advanced operator grants remain supported, and widget selections grant no authority. The OMP extension checks fresh authority before tools and blocks mutating tools, shell/eval and delegation until Execute. Caller/run/attempt, endpoint, terminal and bound native OMP-session evidence fence stale or mismatched actors. These are same-UID accident-prevention controls, not an OS sandbox.

Briefs, instructions, NeedsInput answers and upward reports are durable inbox messages. The extension sends a non-steering OMP `aside` wake to start an idle turn, or a coalesced `followUp` while busy, containing counts and a pull instruction—not message bodies. Bound OMP main sessions should use `cockpit_inbox` with `operation=list`, treat bodies as untrusted data, process them, then explicitly use `operation=ack` through the processed sequence. The SDK tool carries fresh native-session identity and uses the launch-selected CLI/configuration, avoiding an older `cockpit-cli` resolved from ambient PATH. Direct CLI pull/ack remains supported with the matching executable and caller evidence. Stored, Woken, Read and Acked are distinct; a wake is not acceptance or processing. No orchestration delivery types into a terminal, submits a user's draft, or uses Herdr prompt/callback steering. Internal subagent Send uses native external Cockpit IRC delivery at safe wait boundaries, not parent impersonation/steering. Cancel uses OMP's native lifecycle controller and awaits terminal disposal of the exact child and its owned background work, not merely a turn abort. Controls retain applied/failed receipts.

Progress, Ready, NeedsInput and Result are explicit reports. Ready and Result receipts belong only to the run's bound main OMP session; subagent reports retain provenance without replacing those receipts. Reports can go to the parent or another ancestor, but upward reporting grants no control over that ancestor. Herdr working/idle/done/exited is observed runtime state, never task completion. A Result moves work to Review and leaves its Markdown checkbox unchecked. The authorized supervisor reviews the explicit successful Result and **Accepts** it at the current exact task revision through the orchestration API; acceptance is not inferred from reporting or idle state. Advanced operator acceptance remains available; **Send back** returns work for further execution. Cancellation and acceptance do not tear down the Space or worktree.

Canonical task title, body and checked state live only in `<state_root>/orchestration/tasks/<root_id>.md`, with stable `<!-- cockpit-task: <uuid> -->` markers. These are Cockpit tasks, not Herdr task records or copied Kanban cards; external Markdown edits remain authoritative. Exact item/document byte hashes fence updates. **Task** assignment uses one stable UUID and a recoverable cross-document journal to publish the canonical item and one root-inbox task pointer. The journal temporarily retains submitted title/body for recovery; it is removed once the canonical item and inbox pointer are committed and is not a second canonical task store. Authoring alone is not completed assignment. Conflicts require revision-checked resolution and never roll back external Markdown edits. Unmarked checklist items need ID assignment; duplicate IDs are diagnosed and cannot be mutated until corrected.

`<state_root>/orchestration/state.json` durably stores runs, relationships, grants, messages, subagent telemetry, task-assignment journals and acceptance intents, but not a second canonical task store or live Herdr status. One named lock and atomic replacement protect Cockpit writes; external Markdown editors do not honor that lock, so byte rechecks detect conflicts but cannot eliminate the final recheck-to-rename race. Assignment and Accept use recoverable intents across the machine document and Markdown, with exact-revision conflict checks. Only the private runtime owner dispatches/reconciles work, with per-run execution leases; owner restart does not clear orchestration state or stop Herdr agents. Ambiguous setup/launch outcomes require exact-receipt reconciliation or explicit guarded recovery, not automatic re-launch. Delivery receipts do not promise exactly-once execution of external effects.

**Check status/Reconcile** queues a read-only review of an already launched run's exact launch receipt. Matching fresh actual OMP proof and binding restore dispatch state to Launched while preserving current lifecycle, Ready/Result receipts, grants, location and inbox progress, including legitimate changes during review. Missing terminals, stale bindings or conflicting proof remain visible as missing/Unknown/NeedsReview; an unavailable observation does not prove absence. Explicit guarded **Restart/Retry launch** rechecks the prior launch before creating a new tab, retains run/root identity and recorded setup, increments the launch attempt and requires a new main SDK binding. It does not recreate the checkout or implicitly duplicate a possibly live agent. **Close tracking** retains tasks, accepted results and history; it does not guarantee that the agent or its descendants stop.

Workers read selected Library item paths and existing repository paths directly through Space context discovery. Setup, launch and teardown do not create, copy or manage companion folders; existing companion content is left untouched. Canonical supervisor tasks live outside checkouts; implementation notes remain in the worker checkout, not the managed Library.

## 6. Terminal environment

Existing Herdr-provided metadata is inherited. Worktree create/open cannot accept environment variables for the initial root pane in the inspected Herdr version. Cockpit passes context/workspace variables explicitly when it creates subsequent tabs/panes through supported env parameters. Existing terminals and panes launched directly from the Herdr TUI cannot be retrofitted or assumed to inherit them. The setup result states this limitation and never silently closes the initial pane.

Supervisor start and authorized worker preparation launch OMP through Herdr's typed tab creation and `agent.start` methods, with the Cockpit extension loaded by per-process `-e`. No shell typing, task-prompt injection or global OMP configuration mutation is involved. Existing OMP authentication is reused; Cockpit does not sign in or manage it. Launch completion requires fresh actual OMP proof plus its main SDK binding, not merely `agent.start` acknowledgement. Manually launched agents remain supported and are shown as unmanaged unless explicitly adopted and bound. CLI selection has an explicit host/native process role: a host may use itself, but the installed native GUI named `cockpit` selects its sibling `cockpit-cli`, never itself; debug `cockpit-tauri` may use the distinct sibling `cockpit` host. No override is required for the installed pair.

Cockpit places no secrets in snapshots, generated environment values, Library files or configuration. A pasted provider API token or PAT (Jira, Confluence) is stored in the OS vault, one item per configured provider id and instance. The UI can set, replace and remove it but never read it back. Jira and Confluence use that token exclusively in Cockpit's GET-only HTTP client: no CLI processes, environment injection, initialization files or profiles are used. Missing stored tokens fail with `source_credential_required`; an unavailable vault fails with `credential_vault_unavailable`, with no external-login fallback. GitHub, GitLab and Gitea retain their CLI credentials. Passkeys/WebAuthn and OAuth sign-in are not storable secrets.

## 7. Context ingestion and Library

### 7.1 Provider boundaries

Cockpit pulls and validates freshness. Developers and agents perform remote writes through the provider CLIs or their normal tools. Cockpit does not silently mutate remote tickets, issues, reviews, or wiki pages.

Each configured provider has an explicit `kind` (`github`, `gitlab`, `gitea`, `jira`, `confluence`). The forge kinds require `executable`; Jira/Confluence reject `executable` and `login`. Their optional TOML `deployment` (`cloud`, `data_center`) resolves to Cloud for case-insensitive `*.atlassian.net` hosts and Data Center elsewhere unless explicitly overridden. Auth kind never chooses deployment. Confluence Cloud requires `/wiki`; Data Center keeps the configured context path. Jira uses Cloud REST v3 or Data Center v2; Confluence uses Cloud v2 or Data Center v1. See `CODE_GUIDE.md` for the migration snippet; the user's configuration is never automatically edited.

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

### 7.3 Durable Library and Space selections

The global Context Library is the durable, Cockpit-owned source for provider snapshots. It is stored under the configured `library_root`, independently of Herdr sessions and Spaces. Spaces hold relevance selections, not separate files or pinned versions. Library refreshes are visible the next time an agent or viewer reads the item.

Space adds save to the Library first, then select the saved item IDs for a freshly verified Herdr Space. Setup reuses its prevalidated primary and linked assets without fetching them again. A failed selection leaves the saved Library items intact; there is no companion copy, copy retry journal, or Space update phase. The Library UI offers `Add to <Space>` and `Library and <Space>`. Context `Resources` lists selected items and additional repository paths; removing an item or repository from a Space only removes its selection.

`SourceService` is fetch-only: provider lookup, bounded metadata/fetch, setup's short-lived `RecentReads`, and bounded reference traversal (`collect_related`, which returns assets without persisting them). `LibraryService` is the persistence authority for Library content and Space selections.

The old `<state_root>/sources` cache is inert: Cockpit never reads, imports, reports, modifies, or deletes it. There is no migration. It remains on disk for manual user removal after confirmation.

Local folders are explicitly copied into the Library from a typed absolute or `~` path; this remains separate from selecting an existing repository for a Space. Capture has no live link, two-way sync, or native folder picker. It rejects overlaps with Cockpit-owned roots. Git roots use tracked and untracked non-ignored files; plain directories use regular files. Nested `.git`, symlinks, special files, hardlinks, and native executables are excluded and counted. Fixed build/dependency exclusions apply. Paths are byte-wise sorted, and file/byte limits keep a sorted prefix with a `partial` result. The source is not changed. Refresh is an explicit re-copy, with confirmation protecting edited Library bytes; all Spaces read the resulting Library version directly.

The Library is a human- and agent-readable tree that mirrors each source's own hierarchy; no Cockpit knowledge is needed to traverse it. Provider items live at `<provider>/<host>/<source hierarchy>/<leaf>/<Title>.md`: Confluence `confluence/<host>/<KEY - Space Name>/<ancestor titles>/<Title>/<Title>.md` with child pages nested in the parent's directory; Jira `jira/<host>/<PROJ>/<PROJ-123>/<Title>.md`; forges `<provider>/<host>/<owner>/<repo>/issues|merge-requests|pulls/<n>/<Title>.md`. Downloaded attachments sit in the item's `_files/`. Folder copies live under `folders/<Folder Name>`. Segments keep real titles and replace only unsafe characters; a sibling collision appends ` [<id>]`. All machine state (index, journal, staging, trash, locks, operations) is under `.cockpit/`, and a generated root `README.md` explains the layout. A provider item owns only its document and `_files/`; child items and user files in its directory are never touched by its refresh, replacement or removal. A title or ancestor change moves the item by renaming its directory (children move with it) and rewrites descendant paths in one journaled index commit. The flat layout (index schema 1) has no migration: such a Library fails with `library_layout_outdated` and must be deleted and re-added. Index schema 2 is upgraded to schema 3 in place when the Library opens (see the reference model below); that upgrade is one-way, and an older binary reports the schema 3 index as corrupt, so copy the Library before trying a new build on real data.
Issue-like provider snapshots use generated Markdown plus frontmatter fields for known type, status, priority, author, assignee, timestamps, and comment count. The body starts with a title and one-line summary, then optional `## Description` and `## Comments (n)` sections. Each comment is a `### Author · YYYY-MM-DD HH:MM` card with an optional edited time/location, one permalink paragraph, and its body; provider headings are demoted so descriptions and comments cannot collide with document structure. Partial comment lists state shown and total counts.


Folder items retain each file at its relative Library path. Selecting an item never materializes a Space copy. Additional repositories are selected from the configured local catalog and read in place; the Space's own checkout remains available. Repository paths are revalidated with fresh filesystem and Git evidence before becoming viewer roots.
Markdown links navigate within the current authorized root; source URLs open the corresponding Library item, and file actions copy Library-relative or absolute paths. New context terminals receive `COCKPIT_LIBRARY_ROOT`, not `COCKPIT_CONTEXT_PATH`. Existing terminals discover relevant files with the read-only `cockpit context --current` command, which reports the originating Space, Library root, selected item paths, checkout and extra repository paths.

A followed Confluence space includes every page the stored token can read across its top-level trees, not only the homepage tree; Cloud folders appear as ancestor-only nodes in the hierarchy. Its durable follow record, page count, partial state, and excluded page ids live in the Library index. `Refresh space` pages the space once and fetches a page only when new or when its version, title, or ancestor chain changed. Page metadata, labels, storage body, and attachment lookups run concurrently; changed pages are fetched with a bounded concurrency of eight, then published in deterministic parent-first order, each as its own crash-safe durable Library transaction. Ancestor metadata drives hierarchy and parent links; a moved or renamed page's directory is renamed to its new place. Pages absent from a complete enumeration are identified as removed at source, not deleted; previously imported snapshots are preserved. Storage XML is converted to Markdown locally on both deployments; explicit page refresh may change the historical CLI-rendered body, while unchanged follow members are not mass-rewritten.

Every Library item carries a sorted set of references. `Manual` covers a manual add, the "Keep in Library" action and setup imports; `Follow{follow_id}` means a followed Confluence space or Jira query holds it; `Space{space_context_id}` records that a Space selects the item. Refs change only through index mutations under the exclusive lock, so a publish or refresh of an existing item keeps its current refs and two follows cannot overwrite each other's membership. An item that loses its last reference is not deleted: it gets a `purge_after` timestamp (tombstone) and shows an `Unreferenced` pill. A live follow drop sets it 14 days ahead; removing the last reference marks it due at once. Adding any reference clears the tombstone. The sweep at the end of every add or refresh purges due tombstones, unless the item is being fetched or has local Library edits (kept and reported). Because membership is a ref, a page that was added manually and later falls inside a followed space now survives removing that space; only items held by nothing else are removed.

A followed Jira query is added from Add to Library by typing JQL, or a bare project key such as `SCRUM`, which means `project = SCRUM`. The query is normalized (whitespace collapsed, a trailing `ORDER BY` removed); listing adds `ORDER BY updated DESC` directly through the REST API, with no CLI default-project filter or guard prefix. Each matching issue is one Library item at its usual `jira/<host>/<PROJ>/<KEY>/<Title>.md`, shared by every follow that matches it. Resolve shows the query, the issue count (`100+ issues` when the bounded first page is full) and a suggested mode. A follow is `live`, whose members mirror the query, or `accumulate`, which only adds and never drops; queries with relative dates (`-7d`, `now()`, `startOfDay()`) suggest `accumulate`. A refresh lists key, updated, status, type and assignee and fetches content only when `updated` differs from the last fetch, or the item is removed, failed or unknown. Cloud listings use enhanced-search tokens; Data Center uses offsets, 100 results per page, capped by `library_space_pages`. Listed `updated` preserves the returned wall time as `YYYY-MM-DD HH:MM:SS`, matching stored follow comparisons without mass refetches. An accumulate refresh probes from members' newest `source_revision` and key-checks members the probe did not return. A live refresh drops a member only after a complete listing: failed, truncated, cancelled or empty listings never drop anyone (an empty listing with members is reported as partial and kept). A dropped member loses the follow ref and is tombstoned if nothing else holds it. `Stop following` keeps issues (exclusive members become `Manual`); `Remove query and its items` deletes members held by that follow alone, refused if any has local edits; removing one issue excludes it from every follow that references it. Jira follows remain global Library queries, not Space follows; individual Jira items can be selected directly.

A Jira query follow carries the same `reference_depth` (default 0, stored on the follow), applied after each add or refresh. An item the query lists is a seed; an item reached only through references is related and is held by `Follow{follow_id}` plus an inclusion reason held by that follow, so it counts toward the follow's items and is shared like any member. If the query later lists a related item it becomes a seed and loses the reason. A live follow drops related items no longer reached only after a complete related pass and a complete seed listing; an accumulate follow never drops related items. Any failed, capped, cancelled or incomplete pass keeps every member and adds one Partial row instead of marking the follow partial. Seeds whose content is unchanged are traversed from their stored references, so a refresh fetches only changed seeds plus the related items; a seed stored before depth was enabled is fetched once to record its references. Removing a related item excludes it by Library id and it stays out. Space selection is independent: Library refresh never fans out to selected Spaces, and Jira follow collections cannot target a Space.

Adding a followed Confluence space to a Space selects the items saved by that add. Global follow refresh remains a Library operation, not per-Space synchronization. Existing selections read refreshed Library content directly.

Confluence attachment metadata is captured with page snapshots, but binary downloads are opt-in: individual page actions or an explicit followed-space option. Jira captures bounded `fields.attachment[]` metadata in the same panel and frontmatter; refresh re-lists it because attachment changes affect the issue's `updated` time and content revision. Both providers download through Cockpit's shared HTTP client using the stored token, ID-based private no-follow staging, enforced per-file/aggregate byte budgets, safe stored names and atomic publication; failed downloads remove their partial file. Authorization stays on the configured origin; cross-origin media redirects carry none, and HTTPS-to-HTTP is refused. JSON redirects stay same-origin with final on-site path validation. Both redirect paths cap at three. `Remove downloaded` removes only stored binaries. A confirmed Library replacement re-downloads edited bytes only after matching current file hashes; an opted-in follow refresh retries failed downloads. PNG/JPEG render through the bounded raster-media reader; PDF has no active renderer and SVG/HTML are never executed. Missing tokens prevent all Jira/Confluence reads with `source_credential_required`; vault failures use `credential_vault_unavailable`. A Jira follow's attachment opt-in reports per-row download failures without discarding successfully saved issues. Historical attachment fake-server checks do not establish a new live HTTP adapter run.

Existing companion directories and their `repos/` entries are preserved untouched on disk. Known Library-backed legacy association metadata is migrated to Space selections; unknown files and user notes are never imported, adopted, or deleted. Setup and teardown no longer manage companion directories. The legacy `<state_root>/sources` cache remains inert.

### 7.4 Library Markdown format

Provider snapshots in the Library are Markdown with a versioned frontmatter envelope. The fixed envelope includes `schema_version`, `provider`, `resource_type`, `canonical_id`, `provider_instance`, `source_url`, `original_url`, `complete`, `source_revision`, `content_hash`, and `generated`; provider-specific fields and attachment metadata may follow.

`content_hash` is the Library content revision (`sha256:`), not a hash of provenance or presentation metadata. For provider items it covers identity (provider instance, resource type and canonical identifier), title, source revision, body, completeness, and non-empty extra fields and attachment metadata. It excludes URLs, container presentation, diagnostics, and fetch time. The body is normalized provider data for human and agent reading; raw provider payloads are not retained as the canonical snapshot.

### 7.5 Available provider adapters

The Library currently supports GitHub issues and pull requests, GitLab, Gitea through Tea, Jira issues and followed Jira queries, and Confluence pages and followed spaces. Explicit provider `kind` selects the adapter, independent of ids or executable names. Confluence accepts Cloud page IDs/links and Data Center display links under its configured instance. A page snapshot includes locally converted storage Markdown and bounded metadata (space, ancestors, version, editor display name, labels and attachment metadata); attachment binaries stay not downloaded unless explicitly requested for a page or follow. Confluence URL authority and canonical page identity are checked against the configured instance; a checkout's Git origin does not select or constrain it. Jira and Confluence require OS-vault credentials (section 6), with no CLI login/profile fallback.

Live verification (2026-10-06): Cockpit's GET-only adapters imported and refreshed Jira Cloud issues/comments and Confluence Cloud pages using stored Basic tokens in a private Linux keyring and disposable browser Library. JQL follow/refresh, spaces enumeration, large storage-to-Markdown conversion, and Confluence attachment download/PNG preview/removal succeeded. A clean Confluence space follow imported all 66 readable pages; refresh reported 66 unchanged without partial/failed results. Clearing either token refused new reads without removing saved items. Live Data Center behavior, including Jira Bearer PATs, a live Jira attachment download, and macOS Keychain remain unverified; local HTTP fixtures exercise those API contracts, not live deployment behavior. The native host type-checked but was not exercised in a native window. `glab`'s host selector still cannot express a port. Jira Data Center wiki-markup descriptions/comments use the conservative `jira_wiki.rs` converter (headings, emphasis, code/noformat, quotes, lists, links, images, tables, rules; unrecognised constructs stay as text). Vault calls may take 20 s to report `vault_unavailable` when no keyring daemon runs. glab/gh/tea do not take Cockpit-stored tokens. Unsupported capabilities are reported rather than silently substituted.

Internal HTTP reference traversal was live-verified in a disposable Library against the cycle SCRUM-7 → SCRUM-8 by structured link, SCRUM-7's comment → a Confluence page, that page → SCRUM-9, and SCRUM-9 → SCRUM-7. Importing SCRUM-7 at depth 2 saved exactly SCRUM-7, SCRUM-8, the page and SCRUM-9 once each. The JQL `key = SCRUM-7` follow at depth 1 retained exactly the two issues and page; refresh kept all three without duplicates or failures. GitHub, GitLab and Gitea URL expansion is fixture/contract verified only, not live; do not read the Jira/Confluence run as evidence for every forge.


## 8. CLI surface

The `cockpit` CLI provides:

- `cockpit status` to inspect the configured Herdr installation;
- `cockpit serve` to serve the browser client and HTTP API in the foreground;
- `cockpit configuration` to inspect effective non-secret project configuration;
- `cockpit browser` to control a Herdr tab's managed browser via `--tab` or `--current`, and read/acknowledge archived pre-tab feedback via `--legacy`.
- `cockpit widget show|close|list|selection` to publish run-local trusted HTML or declarative choices through the private owner, refine an existing ID, and pull/wait for untrusted selection JSON.
- `cockpit task list|show|create|update|assign-ids`, `run list|show|propose|prepare|execute|accept|send-back|cancel|report|bind-session|message|annotate|adopt`, `inbox list|wait|woken|ack`, `subagent update|controls|control-done|send|cancel`, and `route resolve` expose agent-facing orchestration and inspection. Installed agent-facing examples use `cockpit-cli`, the same host binary. Prepare/Execute/Accept are supervisor actions, not CLI operator grants: fresh actual main-session authority from an active, bound, top-level Supervisor/Adopted root permits only strict descendant Workers. Prepare/Execute require exact current plan revisions; Accept requires an explicit successful Result and the current exact task revision. Workers cannot self-authorize. Browser/native operator intervention remains separate, with its own provenance.

Workspace lifecycle and context operations are provided through core and host services, not separate CLI subcommands.

## 9. Verification strategy

Verification uses the actual changed surface:

- protocol contract tests run equivalent behavior cases against native and browser client adapters;
- Herdr schema fixtures cover each supported Herdr release;
- a real native smoke test connects to an installed Herdr-server, mirrors Spaces/Agents/tabs/panes, attaches a visible terminal, sends input, and observes state updates;
- browser runtime verification uses `cockpit serve` against an explicit run-owned Herdr session; `--test-mode` is only an unavailable-state fixture, not live compatibility proof;
- context/provider tests cover normalization, frontmatter identity, freshness unchanged/changed cases, bounded reference traversal, and partial failures;
- workspace lifecycle tests cover configured repository discovery, exact Herdr worktree provenance, borrowed-directory safety, direct Space selections, and preservation of legacy companion files.

## 10. Deferred scope

- global OMP configuration/authentication management (supervisor and worker launch with a per-process extension are supported);
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

