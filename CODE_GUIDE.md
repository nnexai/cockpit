# Code guide

Herdr owns live sessions, Spaces, tabs, real terminal existence/membership, focus identity and PTYs. Cockpit owns in-memory tab placement and local Files/Review/Browser leaves, task setup, Context assets, local Review snapshots and comment batches. An HTTP response arriving later does not make it newer than an ordered stream event.

## Where to change behavior

| Change | Owner | Useful verification |
| --- | --- | --- |
| Public request/response shape | `crates/cockpit-protocol/src/` and its TypeScript exporter | Protocol tests and generated-file check |
| Session ordering | `src/client/streamOrder.ts`, `src/app/session/sessionStore.ts` | Ordering corpus and reducer tests |
| Local placement, membership and focus | `src/app/layout/{tabLayoutStore,reconcile,splitTree,solveLayout}.ts`, `TabCanvas.tsx`, `src/app/session/{focusCoordinator,mutationCoordinator}.ts`, `src/app/input/` | Tree/focus/creation transitions, live membership changes and a disposable-runtime smoke |
| Sidebar presentation, status badges and row navigation | `src/app/sidebar/`, `src/app/sidebar.css` | Herdr ordering, state shapes, roving focus and disposable-session screenshots |
| Shared keyboard bindings and discoverability | `src/app/input/shortcuts.ts`, `herdrBindings.ts`, `keymap.ts`, `modal.ts` | Static registry/doc parity, runtime aliases and reload, prefix collisions and literal passthrough |
| Terminal lifecycle and input | `src/app/TerminalPane.tsx`, `crates/cockpit-herdr/src/terminal_wire.rs` | Lifecycle/race tests and successive runtime frames |
| Herdr methods, server identity and creation receipts | `crates/cockpit-herdr/src/cli/`, `crates/cockpit-protocol/src/v1.rs` | Adapter fixtures against the supported schema and real split/move receipts |
| Generic Herdr commands and singleton popup | `crates/cockpit-protocol/src/herdr_shell.rs`, `crates/cockpit-herdr/src/{shell_wire.rs,cli/shell.rs,cli/operations.rs}`, `src/app/{App,ServerPopup}.tsx` | Live advertised invocation, popup program input/closure, unchanged split geometry, disconnected recovery and DOM focus return |
| Task setup and recovery | `crates/cockpit-core/src/projects.rs`, `project_store.rs`, `project_teardown.rs` | Ownership, idempotency and uncertain-outcome fixtures |
| Virtual viewer source binding and path authorization | `crates/cockpit-core/src/{viewer,context}.rs`, `crates/cockpit-protocol/src/viewer.rs`, `src/app/layout/{FilesLeaf,ReviewLeaf}.tsx`, `viewerLifecycle.ts` | Same-tab source checks, stale bindings/root replacement, pluginless Files/Review and source-state retention |
| Repository discovery cache and setup freshness | `crates/cockpit-core/src/repository_cache.rs`, `projects.rs`, `context.rs` | Mutation-generation invalidation, stale-while-refill, and disposable gateway request counts |
| Context/Library file index and picker ranking | `crates/cockpit-core/src/context.rs`, `file_index_cache.rs`, `config.rs`, `library/reader.rs`, `src/app/input/fileIndexCache.ts`, `fileNavigation.ts` | Git ignore/symlink/cap fixture, persisted restart smoke, mixed Unicode ranking parity |
| Library read-path recovery and index cache | `crates/cockpit-core/src/library/store.rs`, `library/space.rs`, `library.rs` | Journal recovery, cross-Store identity invalidation, and bounded reader latency |
| Review token streaming and snapshot reuse | `crates/cockpit-core/src/process.rs`, `review.rs`, `src/app/review/ReviewPane.tsx` | Large real-Git fixture, stable review identity, changed-token invalidation, and linear duplicate marking |
| Library folder capture and per-file Space copies | `crates/cockpit-core/src/library/folder.rs`, `library/space.rs`, `context_assets.rs` | D15 source boundaries/limits, D22 layout and edited-file transitions |
| Provider authority and fetch | `crates/cockpit-core/src/sources.rs` and `crates/cockpit-providers/src/` | Configured-instance and canonical-identity fixtures |
| Durable Library storage, provider snapshots, reference sets and followed spaces/queries | `crates/cockpit-core/src/library.rs`, `library/{store,refs,follow,jira_follow,space}.rs`, `jira_query.rs`, `crates/cockpit-providers/src/{confluence,jira,jira_wiki}.rs` | Paged enumeration, ancestor/move detection, exclusion and removal safety, per-follow Space updates, v2 to v3 upgrade, listing-failure/empty/truncated never dropping members, tombstone purge |
| Reference-depth traversal and inclusion state | `crates/cockpit-core/src/sources/references.rs` (extraction, resolution, `collect_related`, caps), `library/related.rs` (saving related items, inclusions, single-import lifecycle), `library/{refs,jira_follow}.rs`, `crates/cockpit-providers/src/jira.rs` (`issue_fields`), `crates/cockpit-protocol/src/library.rs`, `src/app/library/{AddContextDialog,LibraryDetails}.tsx`, `src/client/libraryProtocol.ts` | Fake-provider traversal with cycles and caps, live follow never dropping on incomplete pass, old data reads as depth 0; live Jira/Confluence graph in a disposable Library |
| Provider tokens: vault, injection and native Jira download | `crates/cockpit-core/src/credentials.rs`, `crates/cockpit-secrets/`, `crates/cockpit-providers/src/{jira,confluence,jira_attachments}.rs` (and `jira_attachments/download.rs`), `src/app/library/ProviderCredentialsDialog.tsx` | `MemoryVault` service tests, the fake HTTP redirect/downgrade tests, and an isolated `dbus-run-session` gnome-keyring smoke |
| Library HTTP/native transport | `crates/cockpit-host/src/server/library.rs`, `src-tauri/src/library.rs` | Equivalent browser/native DTOs and owned-runtime smoke |
| Tea JSON and CLI behavior | `crates/cockpit-providers/src/tea.rs` | Real Tea against a localhost fixture with a fake login |
| Local diff and frozen sources | `crates/cockpit-core/src/review.rs` | Real Git fixtures; compare index and working files before/after |
| Comment text and delivery | `crates/cockpit-core/src/comments/` | Exact payload bytes, CAS recovery and acknowledged paste tests |
| Markdown and media display | `src/app/context/`, `context_media.rs` | Source mapping, hostile input, byte/pixel caps and browser/native rendering |
| Host request decoding and composition | `crates/cockpit-host/src/server/`, `src-tauri/src/` | Equivalent browser/native DTOs and real native startup |
| Library view, Space copies and Context resources | `src/app/library/`, `src/app/context/ContextViewer.tsx`, `src/app/context/ContextResources.tsx` | Library and Space-copy actions, refresh state, and Context viewer tests |
| Per-tab Browser lifecycle and pane-state reset | `crates/cockpit-core/src/browser.rs`, `browser/cleanup.rs`, `ephemeral.rs`, `crates/cockpit-host/src/browser_runtime.rs`, `src/app/layout/{BrowserLeaf,BrowserCleanupNotices}.tsx`, `browserLifecycle.ts` | Independent tabs, fresh open, discard-on-close, owner-only startup reset and observer isolation |

The core depends on narrow adapter traits. Hosts compose concrete adapters; provider behavior belongs in the provider crate. Keep stable error codes with a useful message at the module owning the failure. Clients decode and match response identities before frontend state accepts them.

## Herdr command projection and popup transport

`SessionSnapshotResponse.herdr_shell` carries status, configured prefix chords, advertised command IDs/binding aliases/action descriptions and the singleton popup terminal/size hints. `shell_wire.rs` reads Herdr 0.9.2's generation-1 client-shell snapshot and matching surface revisions; peer identity and boot identity must agree with the inspected server. The adapter retains the last popup through disconnects and publishes projection invalidations through the existing session stream. Unsupported mandatory codecs/generation report `shell_unsupported`; connection, timeout, malformed and identity errors remain visible rather than synthesizing a usable manifest.

`herdrBindings.ts` derives direct/prefix alias matching and shortcut labels from that projection; `keymap.ts` applies advertised-command precedence and reserved literal double-prefix passthrough. Cockpit's `Ctrl+B` stays independent of any different configured Herdr prefix. When Herdr shares `Ctrl+B`, custom prefix aliases shadow colliding Cockpit actions; direct aliases shadow matching viewer chords. Text editors/local dialogs keep their keys. Keep `shortcuts.ts` and its generated documentation block static: document runtime custom bindings in prose outside the block, and do not manufacture plugin-specific registry entries.

`ResourceMutationRequest::CommandInvoke` maps to Herdr `command.invoke`. Before sending, the adapter rechecks the current advertisement and Space/tab/pane membership; `command_not_available` and `command_target_invalid` are explicit failures. Invoke against confirmed Herdr focus, not a viewer's local DOM selection, and never automatically replay an uncertain command result. Browser HTTP/WebSocket and native Tauri IPC/channels use the same `CockpitClient` session, mutation and terminal contract and the same core/adapter implementation.

`ServerPopup` reuses `TerminalPane` with `target_kind: "popup"`; attach revalidates the current live popup ID (`popup_not_open` on mismatch). Center it over `.workarea-content` using server cell/percentage hints. Do not add it to the split tree, detach/reflow the painted underlay, or request Herdr focus. Keep the underlay inert and underlying terminal input gated, including pending popup invocation. While live, Esc/Enter/Tab/prefix keys go to the program without local close or workbench routing. Server closure restores DOM focus to an available opener or selected-tab chrome. Stale state retains a disconnected popup with disabled input and retry.

The shell subscription currently advertises `surface_active: true` at 120×40 cells. Obtaining popup metadata this way can resize unattached panes or a concurrently visible Herdr TUI; this accepted API constraint must not be described as passive observation. Direct control attachments retain Cockpit panes' fitted PTY dimensions, and Cockpit split rectangles are unaffected.

## Development loop

Run `bun run browser` to build the frontend and serve the browser app at `http://127.0.0.1:4173`. Extra server flags can be appended, for example `bun run browser --herdr-session my-session`. Run `bun run tauri:dev` for the native app.

For a focused frontend change, run `bun run typecheck` and `bun run test -- <affected-test-file>`. For a Rust change, run the affected package/test filter. At an integration boundary:

```sh
cargo run -q -p cockpit-protocol --bin export-typescript -- --write src/protocol/generated/v1.ts
bun run typecheck
bun run test
cargo test --workspace --exclude cockpit-tauri
cargo check -p cockpit-tauri
bun run build
```

Use the repository's Rust toolchain and pinned Bun dependencies. Run `cargo fmt --check` on the final Rust scope. Regenerate TypeScript from Rust DTOs; never maintain a second handwritten wire schema. Runtime failures need a reproduction through the user action and its authoritative result, not only a mocked success response.

## Disposable runtime acceptance

Create a uniquely named Herdr session with isolated XDG config/state, a fixture repository and a resource ledger. `scripts/verify/resource_guard.py` checks executable, session, socket and ownership before fixture Herdr commands run. Never automate the default session or use the user's manual gateway. Browser and Tauri must point to the same owned session and configuration. Preserve evidence before stopping only recorded processes.


## Local provider configuration

A provider selects an executable and an existing CLI login/profile by name. Cockpit performs no remote writes; a token can optionally be stored in the OS vault (see Provider tokens below), and without one the CLI's own login applies. For example, in a task-specific Cockpit TOML configuration:

```toml
[[providers]]
id = "my-forge"
base_url = "https://forge.example/gitea"
executable = "tea"
login = "my-existing-tea-login"
```

GitLab (`glab`) and Jira (`jira`, ankitpokhrel/jira-cli) use the CLI's own login:

```toml
[[providers]]
id = "gitlab"
base_url = "https://gitlab.com"
executable = "glab"

[[providers]]
id = "jira"
base_url = "https://your-site.atlassian.net"
executable = "jira"
```

Confluence pages use the installed `confluence` CLI and an existing profile; the provider's base URL must include the site's context path when present:

```toml
[[providers]]
id = "confluence"
base_url = "https://example.atlassian.net/wiki"
executable = "confluence"
login = "my-existing-confluence-profile"
```

Cockpit invokes the read-operation allowlist and passes `CONFLUENCE_READ_ONLY=true` and `CONFLUENCE_CLI_ANALYTICS=false`. Page links must belong to the configured instance. Confluence imports preserve page body, identity, version, ancestor, label, editor-display-name, and attachment metadata. Attachments stay not downloaded by default; explicit page/per-follow downloads use the CLI's attachment operation in private staging with exact-name prediction, safe stored names, no-follow regular-file checks, per-file limits, and a periodically monitored aggregate staging budget (not a hard filesystem quota). `Remove downloaded` deletes stored binaries only. Companions mirror `document.md` and `attachments/<name>`; unedited obsolete files are removed while edits are preserved and reported. PNG/JPEG preview is supported; PDF has no active renderer and SVG/HTML are never executed. Verified against Confluence CLI 2.25.2; Data Center protocol behavior is fixture-verified, not live-validated. Jira issues list attachments from `fields.attachment[]` (`jira_attachments.rs`: name, size, media type, same-site content link; malformed or beyond-256 entries produce a `source_attachments_partial` diagnostic) and download them only with a token stored in Cockpit, because jira-cli has no attachment command: `jira_attachments/download.rs` fetches `/rest/api/2/attachment/{id}` and its same-origin `content` URL over HTTP into the same staging and budget rules. Without a token the panel shows a `Provider token…` action and downloads fail with `source_credential_required`. Jira download is fake-server verified only, not live-verified.

Provider authority is resolved against the selected configured provider instance; forge owner/repository identity comes from the artifact's canonical identifier, and Jira authority comes from the configured site. API-returned canonical URLs are checked against that authority. A checkout's primary repository origin does not constrain Library imports. Tokens live only in the OS vault when stored through Cockpit; otherwise provider CLIs and the user's external credential setup own tokens and logins. A missing login, unavailable adapter or unsupported artifact returns an explicit failure.

## Provider tokens

Jira and Confluence accept a token stored in the OS vault (Linux Secret Service; the macOS Keychain backend is compiled but unverified). In the Library, open the token dialog from `Provider tokens…` in the toolbar `⋯` menu, the command palette (`Library: Provider tokens…`), or an item header's `⋯` menu (Jira and Confluence items); or from `Provider token…` on a provider row, in the Add dialog's sign-in failure, or in a Jira item's attachments panel. A Jira issue whose files need a token but has none also says so on its header's state line, with the same entry point. Choose `Personal access token (Bearer)` (Data Center) or `Email and API token (Basic)` (Cloud), paste the token and save; it can be replaced or removed but never read back, and the dialog shows only `Token stored` and its kind. One item exists per provider id and `base_url`, so editing `base_url` makes an old token not apply. glab, gh and tea report `unsupported`.

New token forms default to Basic for `*.atlassian.net` and Bearer otherwise; both remain selectable, and replacement forms retain the stored kind. A colon in the Basic email is rejected with a specific inline message before submission, without clearing the entered token. Closing a Library dialog returns focus to its opener, or the nearest surviving ancestor if the opener disappeared (for example, after storing a token from the attachments header). Jira attachment headers make no download-state claim until the token status has loaded.

A stored token is injected only into that provider's CLI child environment; with none stored, or the vault unavailable, the CLI keeps its own login (the `Keyring unavailable` status can take 20 s to appear when no keyring daemon runs). Vault values are cached per process, so edits made in Seahorse/KWallet appear after a restart. Jira also still needs the user's `jira init` config file (installation type lives there).

| Jira env | Value |
| --- | --- |
| `JIRA_SERVER` | `base_url` without a trailing `/` (pins the CLI to the site) |
| `JIRA_AUTH_TYPE` | `bearer` or `basic` |
| `JIRA_LOGIN` | username (basic only) |
| `JIRA_API_TOKEN` | the token |

| Confluence env | Value |
| --- | --- |
| `CONFLUENCE_DOMAIN` | `host[:port]` plus the base path, unless the path is exactly `/wiki` |
| `CONFLUENCE_PROTOCOL` | the `base_url` scheme |
| `CONFLUENCE_API_PATH` | `/wiki/rest/api` for a `/wiki` base path, else `/rest/api` |
| `CONFLUENCE_AUTH_TYPE` | `bearer` or `basic` |
| `CONFLUENCE_EMAIL` | username (basic only) |
| `CONFLUENCE_API_TOKEN` | the token |

Confluence env mode ignores `--profile` (still passed) and removes inherited `CONFLUENCE_COOKIE` and `CONFLUENCE_TLS_*`; `CONFLUENCE_READ_ONLY` stays `true`. Cockpit's own HTTP (Jira attachments) sends `Authorization` only to the configured origin, follows at most three redirects, and drops the header on any cross-origin hop and refuses https to http. Verified: live Jira Cloud and Confluence Cloud through a private gnome-keyring in the browser build and native Tauri commands. Not verified: a live Jira attachment download (fake-server tests only), Jira Data Center Bearer, macOS Keychain.

Jira attachment metadata from `rest/api/2/attachment/{id}` must identify the requested attachment: a supplied `id` must match exactly, including when the content link names the requested id. Data Center responses may omit `id`; only then can a same-site link below the configured base path establish identity through `/secure/attachment/{id}/…` (with a file path) or `/rest/api/{version}/attachment/content/{id}` (with no extra path suffix). Null, malformed and mismatched supplied ids are rejected rather than treated as missing. This does not change redirect, credential, byte-budget or private no-follow staging rules.

The global Context Library is rooted at `library_root` (TOML or `COCKPIT_LIBRARY_ROOT`; default `$XDG_DATA_HOME/cockpit/library`, falling back to `~/.local/share/cockpit/library`). For example:

```toml
library_root = "/data/cockpit/library"

[limits]
library_max_items = 20000
```

The Library root must be absolute, contain no `..`, and must not overlap the state, companion, or worktree roots. The limits shown are defaults; configured values are bounded. The Library stores and browses source snapshots, refreshes providers, and adds saved items into a live Space. S3 adds explicit per-Space update, replace, and remove; S6 adds explicit followed-space refresh and per-follow Space updates; S7 adds explicit attachment downloads/removal and safe raster media. A Confluence follow enumerates every readable page across the space's top-level trees; Cloud folders are ancestor-only nodes. Refresh fetches only new pages or pages whose version, title, or ancestor chain changed, and moved or renamed pages are relocated by directory rename (`library/layout.rs` derives placements; `library/store.rs` journals entry publication and moves). Removals are marked only after a complete enumeration and individual confirmation; partial results, enumeration failures, or cancellation never infer absence. Attachments are excluded by default; explicit opt-in downloads stage exact requested files and mirror them under `attachments/` in matching Space copies. PDF is not rendered; SVG/HTML are never active.
Every Library item carries a `refs` set (`library/refs.rs`: `Manual`, `Follow`, `Space`); change it only through `Store::add_ref`, `mutate_index` or the follow/Space commits, never by writing a caller's summary over the index entry. `Store::open` upgrades an index schema 2 to 3 in place before journal recovery; the upgrader works on raw JSON, is idempotent, and is one-way, so test new builds on a copy of a real Library. An item without refs is tombstoned (`purge_after`) and purged by the sweep after each add/refresh (`TOMBSTONE_GRACE_MS`, 14 days for live-follow drops; items with local edits are kept). Jira query follows live in `library/jira_follow.rs` (resolve, add, refresh) with the pure JQL helpers in `jira_query.rs` and the `list_issues` provider call in `crates/cockpit-providers/src/jira.rs`. The list argv is allowlisted; rows use the `\x1f` delimiter and windowed `updated` paging, capped by `library_space_pages`. jira-cli's `timezone` must stay unset for the plain `updated` column to match the view timestamps. Jira follows are not Space follows (`source_capability_unavailable`). The Add dialog classification is in `src/app/library/AddContextDialog.tsx` and `libraryState.ts`.
Reference depth is one `reference_depth` on `LibraryAddRequest` (0 to `MAX_REFERENCE_DEPTH` = 5), the stored item/follow field, and `included_by` inclusion reasons; there is no boolean or same-repository crawl left. `SourceService::collect_related` in `sources/references.rs` only fetches: it extracts references (`asset_references`), resolves each under a configured provider instance, deduplicates by canonical identity, and stops at the `TraversalBudget` caps (`Single` 32 items/8 MiB/60 s, `Query` 100/16 MiB/180 s). Persistence stays in `LibraryService` (`library/related.rs`): save related items under item leases, write reasons through `mutate_index`, and let `refs::remove_ref` strip a follow's reason with its ref. Never drop or untag on an incomplete pass, and never save over a seed. New reference sources are a provider frontmatter field or an extraction rule plus a resolution rule in `references.rs`; keep Confluence page adds and space follows at depth 0.
Markdown links resolve within the current Library or Space root; source URLs for Library items open the Library item or its Space copy. Library items and viewer documents offer Copy Library path and Copy full path actions, and newly created context terminals receive `COCKPIT_LIBRARY_ROOT`.

Library tree structure lives in `libraryState.ts` (`libraryTree` groups by provider id and instance; `nestUnderParents` nests Jira subtasks) and `LibraryTree.tsx` (rows, roving tab stop, keys). Jira subtask nesting needs the listing to project `parent_item_id` from stored references (`jira_parent_projection` in `crates/cockpit-core/src/library.rs`).

Library presentation lives in `src/app/library/`: `LibraryItemHeader` groups page identity and actions (tile, eyebrow and title on the left, `Refresh`, the Space action, `⋯` and Details at the right edge of the title row; the item's facts as one quiet line, then one state line), `LibraryDetails` discloses source/local metadata, and `StatePill`/`ProviderMark` supply shared status and source indicators. `libraryState.ts` accepts stored epoch-millisecond and ISO timestamps for display. The tree beside an item is resized by `TreeSplitter` (`src/app/viewer/ViewerLayout.tsx`): while dragging it writes the width straight to the layout once per frame and never re-renders React, so a tree of thousands of rows stays smooth; it measures the width the grid actually gave the list (a narrow viewer caps a stored width at 60%) only at drag start and on arrow keys. The Library launcher sits between Browser and Commands in the tab strip; shortcut hints come from `src/app/input/shortcuts.ts`.

Setup passes `Arc<LibraryService>` and validated fetch results through `ProjectService::start` and its execution boundary; `resume` passes the Library dependency without retaining provider results. Do not store LibraryService on ProjectService: LibraryService already owns ProjectService for fresh Space/companion authorization. Both HTTP and Tauri compose the same acyclic graph. Pre-start validation fetches without persistence, and `add_fetched_and_copy` commits the validated primary and linked assets plus their pending attempts in one Library operation before any companion copy starts. A failed copy is durable and resumes through `start_space_add`, without provider refetch. `SourceService` has no disk cache or persisted import/list/refresh routes, and the old `<state_root>/sources` contents must remain untouched.

## Boundaries worth preserving

- Keep application mouse/terminal capability claims tied to the supported Herdr runtime. An unavailable capability is inconclusive.
- Keep source views canonical. Rendered Markdown and diagrams map back to physical source lines; HTML, SVG and remote assets do not execute in the host.
- Keep Files/Review independent of addon TUIs. Existing addon panes are ordinary terminals, not renderer replacements; run-local comment batches retain immutable source identity.
- Paste only after the exact preview and target have been revalidated. An ambiguous dispatch requires receipt reconciliation, not an automatic retry.
- Use path-limited staging and inspect the staged diff before committing. Live installation, publication and user-session changes are separate actions.

## Changed-scope quality gate

Run `bun run quality:probe` to inspect the pinned metric tools, then `bun run quality:report --base <review-base>` for the current staged, unstaged and untracked scope. `bun run quality:gate --base <review-base> --strict` applies the strict gate. Reports are ignored local artifacts under `quality/reports/`. Missing metric providers produce an inconclusive result (exit 2); they never count as passing coverage or complexity. See `quality/README.md` for provider inputs, baseline review and exception rules.

For manual, source-targeted mutation reports only, use `bun run quality:mutation -- rust|ts --file <source>`; see the opt-in workflow and side-effect warnings in `quality/README.md`. Mutation testing is never invoked by ordinary tests, the gate, or CI.

## Inline browser implementation

`browser-runtime/browser-helper.mjs` attaches to the tab's CLI-managed Chromium session and owns CDP input, metadata, inspection, and binary frames. `crates/cockpit-host/src/browser_helper.rs` supervises it. `browser_runtime.rs` routes requests through the single runtime owner; `browser_view.rs` provides web routes and bounded binary relay, and native commands use the same runtime. Core `BrowserTarget` selects a tab, or resolves a real pane to its tab; association keys include endpoint identity, session and tab, never Space-only identity.

`src/app/browser/` owns presentation, input mapping, annotations, and PNG composition through `CockpitClient`. Rust protocol DTOs in `crates/cockpit-protocol/src/{browser,browser_view}.rs` generate the shared frontend contract. `src/app/layout/BrowserLeaf.tsx` integrates one Browser leaf per tab with layout lifecycle; core browser draft, feedback and delivery modules own current-run persisted data. Hiding a leaf releases view/capture resources, not its browser session.

Leaf creation uses `BrowserAction::OpenFresh`: stop/clean a surviving session before starting at `[browser] default_url` or `COCKPIT_BROWSER_DEFAULT_URL`, validated at load and defaulting to `about:blank`. An existing leaf's Reconnect and CLI `Open` retain attach/new-page semantics. Close, tab/final-terminal retirement and owning-runtime shutdown confirm the process stopped before removing only derived, no-follow, identity-proven profile/workspace/config artifacts and that association's drafts, pending captures, feedback and delivery receipts. Cookie/login/site storage is disposable; failures remain visible/retryable. No saved-work close guard or detached recovery surface remains.

`BrowserRuntime::start` invokes `ephemeral::reset_owner_state` only after acquiring the exclusive `browser/owner.lock`, before binding `owner.sock` or constructing Review/comments services. It confirms shutdown of derived leftover CLI sessions, clears `browser/` except the owner lock/socket, and clears `comments/` and `review/`. Deletion is no-follow and same-device; preserve the lock inode and fail closed on uncertain shutdown. An observer joining the existing owner clears nothing. Library, vault, configuration, project-operation state and real Herdr processes are outside this reset.

`BrowserWorkScope` has only live `Tab` work. `BrowserCleanupStatus` exposes current-run failures, presented by `BrowserCleanupNotices.tsx`; saved-tab/legacy archive DTOs, routes, commands and recovery panels are removed. Current-run drafts/captures keep their target, document, viewport and ownership checks, and ambiguous deliveries still require reconciliation rather than automatic replay.

CLI addressing is `cockpit browser open|status|close|feedback --herdr-session <session> --herdr-socket <socket> --tab <tab-id>`, or `--current` inside the caller's Herdr pane. Feedback and exact-ID acknowledgement address current tab work only; detached `--legacy` and obsolete `--space` addressing are removed.

## Tab layout and viewer cutover

`tabLayoutStore.ts` holds run-local state above the workbench, keyed by session/server instance/tab. `reconcile.ts` consumes authoritative live membership and changed focus, not Herdr rectangles. `splitTree.ts` owns first-load balanced grids, local split/swap/edge placement and prune rules; `solveLayout.ts` computes local bounds and nested minimums. `TabCanvas.tsx` keeps leaf DOM identity stable across movement, while divider styles update live and weights commit on release. Do not persist layouts or reintroduce Herdr resize/swap/zoom operations. Local fitting of a painted terminal still resizes its control-attached PTY.

Every painted terminal attaches control-only. Input remains gated separately by DOM focus, confirmed Herdr focus and owned control. Hidden tabs, zoom-hidden leaves and the Library detach terminal renderers without stopping processes. Viewer selection sends no Herdr focus request; only a changed external focus selects a real terminal. A repeated snapshot must not steal selection. Cockpit-created terminals are attributed by the validated creation receipt and placed beside the acted-on leaf; unrelated terminals insert at the right edge. Confirmed final-terminal/tab loss releases viewers and retires Browser, discarding its association work.

Files/Review call `ViewerService` with a real same-tab source terminal, then authorize by `viewer_id`/`binding_id` and the pinned selected root. A source close or `cd` must not retarget the context; each request still checks fresh tab/Space/endpoint, binding and filesystem identity. Source switches retain per-source frontend view state during the run. No plugin launch, process-based renderer detection or terminal-view toggle remains. Comments use tagged viewer owners and retain source authorization; owner startup clears their store. `review.rs` uses versioned viewer-bound snapshot/file cache names and prunes recognized pre-viewer names without decoding obsolete payloads; current-format corruption remains an error.
