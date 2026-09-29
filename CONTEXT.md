# Cockpit Architecture Context

Status: architecture reference. The Herdr client, workspace setup, Context browsing and media, comments/paste, Library provider imports, copied folders, followed Confluence spaces and followed Jira queries, graphical Review snapshots, and inline browser are implemented. Verified delivery boundaries are recorded in `DECISIONS.md`.

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

Cockpit is intended to become the developer's primary way of engaging with local projects. Maintainable code and easy behavior changes matter more than distribution or product generality. Distribution to other users, formal accessibility compliance, remote access, and credential management are elective later concerns.

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
- Herdr-owned metadata (agent-list ordering is client presentation; see `DECISIONS.md`).

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
- Herdr-semantic focus and layout operations;
- workspace creation and context hydration;
- provider-backed source import and refresh;
- the `cockpit serve` browser client path.

### 5.2 Information architecture

The first screen uses the native Herdr TUI as a behavioral baseline:

- a top-level Herdr session selector;
- a scrollable hierarchical **Spaces** section in the sidebar;
- an **Agents** attention queue below Spaces;
- a main view containing tabs for the selected Space;
- terminal panes arranged according to the selected tab’s Herdr layout.

This is baseline parity, not a permanent imitation target. Cockpit preserves Herdr semantics and authority while deliberately evolving the presentation toward a dense graphical operations workbench.

The UI uses Herdr-native labels such as Spaces, Agents, tabs, and panes. The Herdr API’s workspace terminology remains an internal mapping detail. Agent ordering is blocked, done, working, idle, unknown; newest state change first.

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

The behavioral authority is Herdr; the presentation is graphical. Pane/layout actions are primarily shortcut-driven, while native Herdr mouse behavior remains available. GUI controls are discoverable. The Herdr magic escape key should have priority over GUI shortcuts (not yet implemented; the current keymap handles only Cockpit's `Ctrl+B` prefix and Escape).

### 5.4 Terminal attachment and scalability

Herdr owns every PTY, process, terminal model, and terminal stream. xterm.js owns rendering and input capture only.

- Compatibility requires Herdr protocol 22, schema 1, and the adapter's required methods, not an exact display-version patch. Each visible pane opens a direct ANSI terminal stream using `TerminalHello` and `ControlTerminal` or `ObserveTerminal`. This does not restore the historical client-shell/graphics implementation.
- Herdr's JSON API owns hierarchy, focus, and layout. Stable `TerminalFrame` messages supply sequence numbers, dimensions, and ANSI bytes for each attached pane.
- Attachment uses fitted per-pane dimensions and measured cell pixels. The first frame must be full; every later sequence must be consecutive, including full repaints.
- A full frame is an ANSI baseline, not permission to reset xterm. Socket framing has one uninterrupted reader with bounded buffering and deterministic shutdown.
- Terminal graphics are parked. Known auxiliary messages are consumed without exposing graphics payloads or disconnecting an otherwise usable text terminal. The image addon is not loaded.
- Text and binary input use stable raw `Input`; wheel/page scrolling uses `AttachScroll`, gated by local control intent and attachment state. Normal xterm.js panes attached to Herdr are observed to receive wheel/scroll events.
- Herdr's per-attachment `MouseCapture` signal enables application mouse handling automatically. Cockpit sends structured `AttachMouse` cell coordinates, and Herdr chooses the application's encoding and rejects reports when tracking is disabled. Mode-off and Shift-drag retain xterm text selection. Idle hover reports and exact pixel coordinates are not forwarded.
- `Shift+Enter` sends a bare line-feed.
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

Context and Library file pickers request one sorted server-side file index rather than walking directories over many UI requests. Git roots use tracked plus non-ignored untracked paths; ordinary folders use a bounded no-follow walk. The browser keeps a small stale-while-revalidate candidate list; the gateway also persists path-only hints under the configured cache root (`COCKPIT_CACHE_ROOT`, `cache_root`, then `$XDG_CACHE_HOME/cockpit`). Every open re-enumerates the authorized root, and cached paths never grant access or supply file contents.

Repository catalog scans are cached only for read-path authorization, with a mutation generation and stale-while-refill window; setup and teardown continue to perform fresh checks. Review reuses an in-memory snapshot only when its Git revision tokens and comparison identity still match, and revision tokens stream Git output into a digest rather than retaining large diff output.

Cockpit owns durable GUI drafts and a local Git review model for the complete Reviewr replacement. Local review includes staged, unstaged, branch, and untracked scopes with explicit revisions and side-aware anchors. It does not mutate Git or post provider comments.

The inline browser displays a supervised Chromium tab beside the selected Space. Cockpit owns browser interaction, durable drafts, and feedback; hiding the view releases capture resources without closing the browser.

### 5.6 Errors, settings, and accessibility

Errors are inline on the affected Space, tab, pane, or operation. The UI preserves last-known state, explains the failed operation, and offers retry/resync without disabling unrelated resources. Toasts may supplement inline errors.

There is no settings UI initially. Durable configuration uses a config file; environment variables and one-off command options override it.

Accessibility is best effort for this personal proof of concept, not an acceptance gate for broader distribution. The UI should still preserve keyboard operation, visible focus, meaningful labels, non-color-only state, and actionable error text where practical.

## 6. Terminal environment

Existing Herdr-provided metadata is inherited. Worktree create/open cannot accept environment variables for the initial root pane in the inspected Herdr version. Cockpit passes context/workspace variables explicitly when it creates subsequent tabs/panes through supported env parameters. Existing terminals and panes launched directly from the Herdr TUI cannot be retrofitted or assumed to inherit them. The setup result states this limitation and never silently closes the initial pane.

Cockpit does not automatically launch or configure OMP. The developer starts agents manually. Herdr’s own integrations report agent state to the presentation layer.

Cockpit does not persist credentials, place secrets in snapshots, or export provider secrets through generated environment values. Provider wrappers retain responsibility for current local credential handling. A future passkey/key-store mechanism is outside this proof of concept.

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

### 7.2 Hydration

Creation hydration is explicit. Adapters decide what to download and how to transform it.

Bounded typed traversal may automatically include:

- the immediate parent issue when available;
- recognized local/provider references in descriptions and comments;
- deduplicated resources within configured depth/count limits;
- cycle detection and explicit per-resource failure status.

Provider failures leave successful assets in place, record retryable per-asset status, and allow the workspace/session to proceed when the primary resource is available.

### 7.3 Durable Library and Space copies

The global Context Library is the durable, Cockpit-owned source for provider snapshots. It is stored under the configured `library_root`, independently of any Herdr session, Space, or companion. Adding or refreshing an item updates the Library; it does not automatically fan out changes to companion copies.

Space adds save the Library item first, then copy or reflink it into a freshly verified companion. Setup passes its prevalidated primary and linked assets into one Library operation without fetching them again; every item and pending attempt becomes durable before any companion copy starts. A companion failure leaves all saved items and their durable Space-add attempts; setup reports `source_sync_conflict` and can resume by copying saved content without asking the provider again. A later Library refresh never silently updates a Space copy. In the UI, a live target Space (the Library view's selected Space, or a Context pane's own Space) adds `Add to <Space>` to Library item headers and menus and a `Library and <Space>` destination to the Add dialog, whose progress shows the Library and Space phases separately. A companion's `Resources` lists that Space's Library copies with read-only states, failed adds first with `Retry adding to <Space>` (copying the saved item, never fetching again) and `Dismiss`; its toolbar button reads `Resources · N behind` when copies are behind. `Update` and `Restore from Library` act only on selected `Library newer` or `Missing in Space` rows in that Space; `Update all (N)` counts those rows and skips edited copies. `Replace with Library version…` and `Remove from this Space…` require confirmation bound to the listed path hashes; a stale confirmation conflicts without changing the file. Copies marked `Removed at source`, `Not in Library`, or `Not linked` are not updated; removing a Library item leaves its Space files intact.

`SourceService` is fetch-only: provider lookup, bounded metadata/fetch, setup's short-lived `RecentReads`, and optional hydration. `LibraryService` is the persistence authority. The legacy pane-scoped source import/list/refresh transports are removed in favor of Library operations and the explicit Space list/add/attempt-dismiss transports.

The old `<state_root>/sources` cache is inert: Cockpit never reads, imports, reports, modifies, or deletes it. There is no migration. It remains on disk for manual user removal after confirmation.

Local folders are copied into the Library from a typed absolute or `~` path; there is no live link, two-way sync, or native folder picker. Capture rejects a directory that overlaps the Library, a Space companion, state, or worktree root. Git roots use tracked and untracked non-ignored files; plain directories use regular files. Nested `.git`, symlinks, special files, hardlinks, and native executables are excluded and counted. Fixed build/dependency exclusions apply. Paths are byte-wise sorted, and file/byte limits keep a sorted prefix with a `partial` result. The source is not changed. Refresh is an explicit re-copy: Library bytes change only after confirmation if a Library file was edited; existing Space copies remain untouched and become `Library newer`.

The Library is a human- and agent-readable tree that mirrors each source's own hierarchy; no Cockpit knowledge is needed to traverse it. Provider items live at `<provider>/<host>/<source hierarchy>/<leaf>/<Title>.md`: Confluence `confluence/<host>/<KEY - Space Name>/<ancestor titles>/<Title>/<Title>.md` with child pages nested in the parent's directory; Jira `jira/<host>/<PROJ>/<PROJ-123>/<Title>.md`; forges `<provider>/<host>/<owner>/<repo>/issues|merge-requests|pulls/<n>/<Title>.md`. Downloaded attachments sit in the item's `_files/`. Folder copies live under `folders/<Folder Name>`. Segments keep real titles and replace only unsafe characters; a sibling collision appends ` [<id>]`. All machine state (index, journal, staging, trash, locks, operations) is under `.cockpit/`, and a generated root `README.md` explains the layout. A provider item owns only its document and `_files/`; child items and user files in its directory are never touched by its refresh, replacement or removal. A title or ancestor change moves the item by renaming its directory (children move with it) and rewrites descendant paths in one journaled index commit. The flat layout (index schema 1) has no migration: such a Library fails with `library_layout_outdated` and must be deleted and re-added. Index schema 2 is upgraded to schema 3 in place when the Library opens (see the reference model below); that upgrade is one-way, and an older binary reports the schema 3 index as corrupt, so copy the Library before trying a new build on real data.
Issue-like provider snapshots use generated Markdown plus frontmatter fields for known type, status, priority, author, assignee, timestamps, and comment count. The body starts with a title and one-line summary, then optional `## Description` and `## Comments (n)` sections. Each comment is a `### Author · YYYY-MM-DD HH:MM` card with an optional edited time/location, one permalink paragraph, and its body; provider headings are demoted so descriptions and comments cannot collide with document structure. Partial comment lists state shown and total counts.


Folder items retain each file at its relative path. Adding any item to a Space mirrors its Library-relative paths in the companion, so links resolve the same way there, with one manifest entry and content hash per file. Explicit Space update copies new or changed Library files, restores missing files, and removes unedited files no longer in the Library. Edited files are skipped until their listed path hashes are explicitly confirmed; edited Space-only files are preserved and reported. Removal applies the same per-file compare-and-swap protection. A Library move reaches a Space only through its explicit update, which writes the new paths and removes unedited files at the old ones.
Markdown links navigate within their current Library or Space root; links to a Library source URL open the corresponding Library item or its Space copy, and file actions copy Library-relative or absolute paths. Newly created context terminals receive `COCKPIT_LIBRARY_ROOT` alongside `COCKPIT_CONTEXT_PATH`.

A followed Confluence space includes every page the profile can read across its top-level trees, not only the homepage tree; Cloud folders appear as ancestor-only nodes in the hierarchy. Its durable follow record, page count, partial state, and excluded page ids live in the Library index. `Refresh space` pages the space once and fetches a page only when new or when its version, title, or ancestor chain changed. Page metadata, labels, body, and attachment lookups run concurrently; changed pages are fetched with a bounded concurrency of eight, then published in deterministic parent-first order, each as its own crash-safe durable Library transaction. Ancestor metadata drives hierarchy and parent links; a moved or renamed page's directory is renamed to its new place. Pages absent from a complete enumeration are identified as removed at source, not deleted; previously imported snapshots are preserved.

Every Library item carries a sorted set of references. `Manual` covers a manual add, the "Keep in Library" action and setup imports; `Follow{follow_id}` means a followed Confluence space or Jira query holds it; `Space{companion_root_id}` is written when the item is copied into a Space and dropped when that copy is removed. Refs change only through index mutations under the exclusive lock, so a publish or refresh of an existing item keeps its current refs and two follows cannot overwrite each other's membership. An item that loses its last reference is not deleted: it gets a `purge_after` timestamp (tombstone) and shows an `Unreferenced` pill. A live follow drop sets it 14 days ahead; removing the last Space copy or a follow with its items marks it due at once. Adding any reference clears the tombstone. The sweep at the end of every add or refresh purges due tombstones, unless the item is being fetched or has local Library edits (kept and reported). Because membership is a ref, a page that was added manually and later falls inside a followed space now survives removing that space; only items held by nothing else are removed.

A followed Jira query is added from Add to Library by typing JQL, or a bare project key such as `SCRUM`, which means `project = SCRUM`. The query is normalized (whitespace collapsed, a trailing `ORDER BY` removed) and every read is sent as `project IS NOT EMPTY AND (<jql>)`, so jira-cli's implicit default-project filter never applies. Each matching issue is one Library item at its usual `jira/<host>/<PROJ>/<KEY>/<Title>.md`, shared by every follow that matches it. Resolve shows the query, the issue count (`100+ issues` when the first window was full) and a suggested mode. A follow is `live`, whose members mirror the query, or `accumulate`, which only adds and never drops; queries with relative dates (`-7d`, `now()`, `startOfDay()`) suggest `accumulate`. A refresh lists metadata only (key, updated, status, type, assignee through jira-cli plain columns with a unit-separator delimiter) and fetches an issue's content only when its `updated` value differs from the one stored at the last fetch, or when the item is removed, failed or unknown. Listings are windowed by `updated`, 100 rows per call, because jira-cli's offset paging is unusable on Cloud; the total is capped by `library_space_pages`. An accumulate refresh probes from a watermark derived from the members' newest `source_revision` and key-checks members the probe did not return. A live refresh drops a member only after a complete listing: a failed, truncated, cancelled or empty listing never drops anyone (an empty listing with members is reported as partial and kept). A dropped member loses the follow ref and is tombstoned if nothing else holds it. `Stop following` keeps the issues (exclusive members become `Manual`); `Remove query and its items` deletes the members held by that follow alone, refused if any of them has local edits; removing one issue excludes it from every follow that references it. Jira follows are not Space follows: Add to Space rejects them, while individual Jira items can still be copied into a Space. The plain `updated` value follows jira-cli's `timezone` setting, which is only significant for window bounds above 100 results; leave `timezone` unset.

A Space holding follows shows one aggregate row per follow, with new, changed, edited, and removed-at-source page counts rather than one row per page. Its explicit `Update` can select follows and copies only the selected follow's new or changed pages for that Space; other follows, standalone items, and other Spaces are untouched. Edited Space copies are reported and skipped unless replacement is explicitly confirmed against their current hashes. The Space manifest tracks known pages so removing a page copy does not cause it to be re-added by a later update.

Confluence attachment metadata is captured with page snapshots, but binary downloads are opt-in: individual page actions or an explicit followed-space option. Downloads use the configured CLI's read-only attachment command, a private staging directory, predicted exact-name matching, per-file and periodically monitored aggregate staging budgets (not hard filesystem quotas), regular single-link file checks, safe stored names, and atomic page-plus-attachment publication; overmatched CLI results are discarded. `Remove downloaded` removes only stored binaries. A confirmed Library replacement re-downloads edited attachment bytes only after matching the exact current file hashes; an opted-in follow refresh retries failed downloads. Page body links retain their original relative paths. Companion copies place `document.md` and `attachments/<safe-name>` together; updates remove obsolete unedited files and preserve/report edited files. PNG and JPEG render through the bounded raster-media reader; PDF has no active renderer, and SVG/HTML are never executed as media. Jira issues (single adds and followed queries) capture `fields.attachment[]` metadata (name, size, media type, content link) the same way and show it read-only in the same attachments panel and item frontmatter; a refresh re-lists it because a changed attachment set changes the issue's `updated` time and content revision. Jira attachment bytes are never downloaded: jira-cli 1.7.0 has no attachment command, and Cockpit does not read its credentials to do authenticated HTTP itself, so `download_attachments`, a Jira follow's attachment opt-in and a per-attachment download are refused with `source_capability_unavailable`. Attachment listing on Jira is verified by fixtures only; the Jira Cloud test site's issues have none, and self-hosted Data Center is unverified.

The former companion repository-snapshot action and its HTTP/Tauri/client transports are removed. Existing `repos/` companion entries remain readable as `Not linked`; Cockpit does not migrate them or inspect the legacy `<state_root>/sources` cache.

### 7.4 Library Markdown format

Provider snapshots in the Library are Markdown with a versioned frontmatter envelope. The fixed envelope includes `schema_version`, `provider`, `resource_type`, `canonical_id`, `provider_instance`, `source_url`, `original_url`, `complete`, `source_revision`, `content_hash`, and `generated`; provider-specific fields and attachment metadata may follow.

`content_hash` is the Library content revision (`sha256:`), not a hash of provenance or presentation metadata. For provider items it covers identity (provider instance, resource type and canonical identifier), title, source revision, body, completeness, and non-empty extra fields and attachment metadata. It excludes URLs, container presentation, diagnostics, and fetch time. The body is normalized provider data for human and agent reading; raw provider payloads are not retained as the canonical snapshot.

### 7.5 Available provider adapters

The Library currently supports GitHub issues and pull requests, GitLab, Gitea through Tea, Jira issues and followed Jira queries, and Confluence pages and followed spaces. Confluence accepts Cloud page IDs/links and Data Center display links from a configured `confluence` CLI profile. A page snapshot includes page body and bounded metadata (space, ancestors, version, editor display name, labels, and attachment metadata); attachment binaries remain not downloaded unless explicitly requested for a page or follow. Confluence URL authority and canonical page identity are checked against the configured instance; a checkout's Git origin does not select or constrain it. Provider credentials remain with the provider CLI or external credential setup; Cockpit does not store tokens or secrets.

Self-hosted provider behavior is fixture/contract verified, not live validated. `glab`'s host selector cannot express a port, and Jira Data Center wiki-markup descriptions/comments are converted to Markdown by a conservative converter (`jira_wiki.rs`: headings, emphasis, code/noformat, quotes, lists, links, images, tables, rules; unrecognised constructs stay as text). Jira query follows read through jira-cli 1.7.0 `issue list --plain` and are live-verified on Jira Cloud for listing, resolve, add and refresh only; Data Center list and date formats, windowing above 100 results and the real-time 14-day purge are fixture-verified. Confluence Cloud and Data Center protocol paths are covered by synthetic CLI fixtures. The existing read-only Confluence profile exposed no page or attachment in the inspected SD space, so there is no live page/attachment validation. Provider executables, logins, and supported operations vary; unsupported capabilities are reported rather than silently substituted.


## 8. CLI surface

The `cockpit` CLI provides:

- `cockpit status` to inspect the configured Herdr installation;
- `cockpit serve` to serve the browser client and HTTP API in the foreground;
- `cockpit configuration` to inspect effective non-secret project configuration;
- `cockpit browser` to control the browser associated with a Herdr Space.

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
- Cockpit-managed credentials, passkeys, and key-store injection;
- remote browser access, TLS, and multi-user authorization;
- a Cockpit settings screen;
- arbitrary shell execution from the UI;
- raw socket forwarding to browsers;
- unbounded file previews;
- formal WCAG 2.2 AA release compliance.

