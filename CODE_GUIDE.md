# Code guide

Herdr owns live sessions, Spaces, tabs, panes, focus and layout. Cockpit projects that state and owns task setup, Context assets, local Review snapshots and comment batches. An HTTP response arriving later does not make it newer than an ordered stream event.

## Where to change behavior

| Change | Owner | Useful verification |
| --- | --- | --- |
| Public request/response shape | `crates/cockpit-protocol/src/` and its TypeScript exporter | Protocol tests and generated-file check |
| Session ordering | `src/client/streamOrder.ts`, `src/app/session/sessionStore.ts` | Ordering corpus and reducer tests |
| Focus, commands and layout projection | `src/app/session/focusCoordinator.ts`, `mutationCoordinator.ts`, `src/app/layout/` and `src/app/input/` | App integration tests and an owned runtime smoke |
| Sidebar presentation, status badges and row navigation | `src/app/sidebar/`, `src/app/sidebar.css` | Herdr ordering, state shapes, roving focus and disposable-session screenshots |
| Shared keyboard bindings and discoverability | `src/app/input/shortcuts.ts`, `keymap.ts`, `modal.ts` | Prefix routing, terminal Tab passthrough, modal Escape and Library focus return |
| Terminal lifecycle and input | `src/app/TerminalPane.tsx`, `crates/cockpit-herdr/src/terminal_wire.rs` | Lifecycle/race tests and successive runtime frames |
| Herdr methods and process evidence | `crates/cockpit-herdr/src/cli/` | Adapter fixtures against the supported schema |
| Task setup and recovery | `crates/cockpit-core/src/projects.rs`, `project_store.rs`, `project_teardown.rs` | Ownership, idempotency and uncertain-outcome fixtures |
| Context path authorization | `crates/cockpit-core/src/context.rs` | Traversal, replacement and companion tests |
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

The core depends on narrow adapter traits. Hosts compose concrete adapters; provider behavior belongs in the provider crate. Keep stable error codes with a useful message at the module owning the failure. Clients decode and match response identities before frontend state accepts them.

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

Jira and Confluence accept a token stored in the OS vault (Linux Secret Service; the macOS Keychain backend is compiled but unverified). In the Library, use `Provider token…` on a provider row, in the Add dialog's sign-in failure, or in a Jira item's attachments panel. Choose `Personal access token (Bearer)` (Data Center) or `Email and API token (Basic)` (Cloud), paste the token and save; it can be replaced or removed but never read back, and the dialog shows only `Token stored` and its kind. One item exists per provider id and `base_url`, so editing `base_url` makes an old token not apply. glab, gh and tea report `unsupported`.

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

Library presentation lives in `src/app/library/`: `LibraryItemHeader` groups page identity and actions, `LibraryDetails` discloses source/local metadata, and `StatePill`/`ProviderMark` supply shared status and source indicators. `libraryState.ts` accepts stored epoch-millisecond and ISO timestamps for display. The Library launcher sits between Browser and Commands in the tab strip; shortcut hints come from `src/app/input/shortcuts.ts`.

Setup passes `Arc<LibraryService>` and validated fetch results through `ProjectService::start` and its execution boundary; `resume` passes the Library dependency without retaining provider results. Do not store LibraryService on ProjectService: LibraryService already owns ProjectService for fresh Space/companion authorization. Both HTTP and Tauri compose the same acyclic graph. Pre-start validation fetches without persistence, and `add_fetched_and_copy` commits the validated primary and linked assets plus their pending attempts in one Library operation before any companion copy starts. A failed copy is durable and resumes through `start_space_add`, without provider refetch. `SourceService` has no disk cache or persisted import/list/refresh routes, and the old `<state_root>/sources` contents must remain untouched.

## Boundaries worth preserving

- Keep application mouse/terminal capability claims tied to the supported Herdr runtime. An unavailable capability is inconclusive.
- Keep source views canonical. Rendered Markdown and diagrams map back to physical source lines; HTML, SVG and remote assets do not execute in the host.
- Keep Reviewr's TUI state independent of Cockpit's durable Review comments.
- Paste only after the exact preview and target have been revalidated. An ambiguous dispatch requires receipt reconciliation, not an automatic retry.
- Use path-limited staging and inspect the staged diff before committing. Live installation, publication and user-session changes are separate actions.

## Changed-scope quality gate

Run `bun run quality:probe` to inspect the pinned metric tools, then `bun run quality:report --base <review-base>` for the current staged, unstaged and untracked scope. `bun run quality:gate --base <review-base> --strict` applies the strict gate. Reports are ignored local artifacts under `quality/reports/`. Missing metric providers produce an inconclusive result (exit 2); they never count as passing coverage or complexity. See `quality/README.md` for provider inputs, baseline review and exception rules.

## Inline browser implementation

`browser-runtime/browser-helper.mjs` attaches to the Space's CLI-managed Chromium and owns CDP input, metadata, inspection, and binary frames. `crates/cockpit-host/src/browser_helper.rs` supervises it. `browser_runtime.rs` forwards observer requests to the one owner. `browser_view.rs` provides the web routes and bounded binary relay; native commands use the same runtime.

`src/app/browser/` owns presentation, input mapping, annotations, and PNG composition through `CockpitClient`. Rust protocol DTOs in `crates/cockpit-protocol/src/browser_view.rs` generate the shared frontend contract. Core browser draft and feedback modules own durable data. The no-migration inline cutover is implemented; focused browser and Linux-native startup verification passed on 2026-09-13. See `planning/inline-space-browser-2026-09-13/` for the exact evidence and unclaimed A01–A25/security/performance matrix.
