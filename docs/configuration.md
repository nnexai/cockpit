# Configuration

This reference owns configuration values, examples and invocation precedence. [DECISIONS](../DECISIONS.md) owns security, ownership, publication and uncertainty rules; [CODE_GUIDE](../CODE_GUIDE.md) maps their implementations. Native installation and prerequisite recovery remain in [native-install](native-install.md).

## Configuration file and roots

Core loading uses an explicit configuration path, then `COCKPIT_CONFIG`, then `$XDG_CONFIG_HOME/cockpit/config.toml` (`$HOME/.config/cockpit/config.toml` when XDG is unset). A missing implicit default means defaults; an explicit unavailable file fails. Herdr-addressed agent CLI commands accept `--config`, then `COCKPIT_CONFIG_PATH`, before that core resolution. GUI and `serve` retain their own invocation discovery. The loader is [config.rs](../crates/cockpit-core/src/config.rs); root resolution is [roots.rs](../crates/cockpit-core/src/config/roots.rs).

Each root environment variable overrides the corresponding top-level TOML key; otherwise the XDG default applies. Unset XDG directories fall back beneath `$HOME` as shown below. Roots are absolute without parent traversal. Library cannot overlap cache, state or worktree roots; Notes additionally cannot overlap the Library or configured repositories. Keep durable Notes outside disposable checkouts and ephemeral pane-state roots.

| TOML key | Environment override | Default | Unset-XDG base |
| --- | --- | --- | --- |
| `cache_root` | `COCKPIT_CACHE_ROOT` | `$XDG_CACHE_HOME/cockpit` | `$HOME/.cache` |
| `worktree_root` | `COCKPIT_WORKTREE_ROOT` | `$XDG_STATE_HOME/cockpit/worktrees` | `$HOME/.local/state` |
| `state_root` | `COCKPIT_STATE_ROOT` | `$XDG_STATE_HOME/cockpit/operations` | `$HOME/.local/state` |
| `library_root` | `COCKPIT_LIBRARY_ROOT` | `$XDG_DATA_HOME/cockpit/library` | `$HOME/.local/share` |
| `notes_root` | `COCKPIT_NOTES_ROOT` | `$XDG_DATA_HOME/cockpit/notes` | `$HOME/.local/share` |

Repository discovery uses invocation roots, then the platform path-list `COCKPIT_REPOSITORY_ROOTS`, then TOML `repository_roots`, then the current directory. It enumerates supported repositories under those roots, not a hard-coded project path. `branch_template` defaults to `cockpit/{repo}/{task_id}` and `checkout_template` to `{repo}-{task_id}`. Issue metadata can supply task type/identifier; review artifacts can supply a source branch. Explicit user branch/location values override derivation.

File-picker cache entries are persisted path-only hints under `cache_root`; opens still re-enumerate the authorized root. [file_index_cache.rs](../crates/cockpit-core/src/file_index_cache.rs) owns this cache, not file content or authorization. Library limits are bounded in [limits.rs](../crates/cockpit-core/src/config/limits.rs), separately from synchronization settings:

```toml
library_root = "/data/cockpit/library"

[limits]
library_max_items = 20000
```

Discovery and Files limits are distinct from document-read and Library limits. The defaults and inclusive ranges below come from [limits.rs](../crates/cockpit-core/src/config/limits.rs); [discovery-limits](discovery-limits.md) explains breadth-first budget accounting. Changing a document limit cannot remove a discovery warning. Browser hosts accept `--repository-root`; `cockpit serve --help` describes invocation overrides.

| Limit | Default | Inclusive range |
| --- | --- | --- |
| `catalog_entries` | 16384 | 1–100000 |
| `catalog_depth` | 3 | 1–32 |
| `operation_timeout_ms` | 30000 | 1–600000 |
| `context_directory_entries` | 1000 | 1–10000 |
| `context_tree_depth` | 32 | 1–64 |
| `context_preview_bytes` | 1 MiB | 1 KiB–8 MiB |
| `context_preview_lines` | 5000 | 1–20000 |

An illustrative discovery configuration (the entry budget is nondefault):

```toml
version = 1
repository_roots = ["/path/to/projects"]

[limits]
catalog_entries = 10000
catalog_depth = 3
```

## Endpoint and CLI selection

Herdr-addressed `widget`, `context`, `browser`, `notes` and orchestration commands share [endpoint.rs](../crates/cockpit-host/src/bin/cockpit/endpoint.rs). Explicit session/socket selectors take precedence; otherwise launch `COCKPIT_SESSION_ID` pairs with the pane's `HERDR_SOCKET_PATH`, then native Herdr inheritance. Socket layout identifies `herdr.sock` as `default` and `sessions/<name>/herdr.sock` as a named session. An explicit session never borrows another session's inherited socket; a stale selected socket fails rather than falling back.

The extension uses launch-selected `COCKPIT_CLI_PATH` and configuration, not an ambient-PATH replacement. Host role may use itself. Native role selects sibling `cockpit-cli` when the installed GUI is named `cockpit`, never the GUI executable; debug `cockpit-tauri` may use the distinct sibling `cockpit` host. The installed pair needs no override. Direct inbox CLI calls remain supported with the correct binary/configuration and fresh native caller evidence.

New context terminals receive `COCKPIT_LIBRARY_ROOT`. Existing terminals are not retrofitted: `cockpit-cli context --current` reports the originating Space, Library root, selected `items[].path`, `checkout_path` and `repository_paths` for direct reads. Explicit targeting uses `--herdr-session ID --herdr-socket PATH --space ID`; `--config PATH` selects configuration. Writable task notes belong in Space Notes, not managed Library content.

## Durable Space Notes

Each Notes UUID owns `scratchpad.md`, `todos.md`, `decisions/<decisionId>.md` and `comments/<todoId>/<commentId>.md`; registry/lock metadata is under `.cockpit/`. Opening an unbound Space creates nothing; create and attach are explicit. POSIX UI recipes pin both root and UUID and escape shell-sensitive paths. In an uninstalled checkout use `target/debug/cockpit` instead of installed `cockpit-cli`.

```sh
cockpit-cli notes --current target
cockpit-cli notes --current target --create
cockpit-cli notes --current target --attach UUID

# Resolve once; use the returned UUID and the same configured root thereafter.
COCKPIT_NOTES_ROOT='/persistent/cockpit/notes' cockpit-cli notes --notes UUID scratchpad read
COCKPIT_NOTES_ROOT='/persistent/cockpit/notes' cockpit-cli notes --notes UUID todo list
COCKPIT_NOTES_ROOT='/persistent/cockpit/notes' cockpit-cli notes --notes UUID kanban list
COCKPIT_NOTES_ROOT='/persistent/cockpit/notes' cockpit-cli notes --notes UUID decision list
COCKPIT_NOTES_ROOT='/persistent/cockpit/notes' cockpit-cli notes --notes UUID comment list --todo TODO_ID
```

Use `--id ID --expected-revision REVISION` for stable-ID task mutations, or `--ref 'L<n>@sha256:…'` for an unadopted/ambiguous source task. Scratchpad, decision and comment edits use their read revision. Pinned content commands work without Herdr; `--current` requires a real current pane. Root/Space operations do not authorize arbitrary content paths. See [DECISIONS](../DECISIONS.md) for re-read after unknown outcomes and advisory-lock CAS limits.

## Providers and deployment

Each provider declares `kind`: `github`, `gitlab`, `gitea`, `jira` or `confluence`; neither ID nor executable basename chooses the adapter. Forge kinds require a CLI `executable` and retain external CLI login; Gitea can name an existing Tea `login`. Jira/Confluence use Cockpit's GET-only HTTP transport and OS-vault tokens, and reject `executable`/`login`. Cockpit performs no remote writes and never rewrites the user's configuration automatically.

```toml
[[providers]]
id = "jira"
kind = "jira"
base_url = "https://nnexai.atlassian.net"     # deployment defaults to cloud

[[providers]]
id = "confluence"
kind = "confluence"
base_url = "https://nnexai.atlassian.net/wiki"

[[providers]]
id = "wiki-dc"
kind = "confluence"
base_url = "https://confluence.example.com/confluence"  # defaults to data_center

[[providers]]
id = "gitlab"
kind = "gitlab"
base_url = "https://gitlab.com"
executable = "glab"

[[providers]]
id = "github"
kind = "github"
base_url = "https://github.com"
executable = "gh"

[[providers]]
id = "my-forge"
kind = "gitea"
base_url = "https://forge.example/gitea"
executable = "tea"
login = "my-existing-tea-login"
```

`deployment` applies only to Jira/Confluence: explicit `cloud` or `data_center` overrides host-based defaults. Case-insensitive `.atlassian.net` hosts default to Cloud; other hosts to Data Center. Authentication kind does not select deployment. Confluence Cloud requires base path exactly `/wiki`; Data Center retains a configured context path such as `/confluence` or `/jira`. No Jira initialization file or Confluence profile is used.

Jira Cloud uses REST v3 enhanced-search token paging; Data Center uses REST v2 offset paging. Confluence Cloud uses v2 under `/wiki`; Data Center uses v1 under its context path. [DECISIONS](../DECISIONS.md) covers selected-instance/canonical-URL authority, credential failures, safe redirects/paging, attachment identity and private publication. Explicit refresh may change locally converted Confluence Markdown; unchanged follow versions do not mass-rewrite pages.

## Provider token storage and entry points

Tokens are stored in the running user's OS vault: Linux Secret Service through [cockpit-secrets](../crates/cockpit-secrets/src/lib.rs); the compiled macOS Keychain backend remains unverified. [credentials.rs](../crates/cockpit-core/src/credentials.rs) owns provider-instance identity, status, validation, caching and timeouts. Tokens are write/replace/remove only, never read back through the UI. One vault item belongs to each provider ID and `base_url`; changing that URL makes the old token inapplicable. Forge CLI credentials remain external; their Cockpit token status is `unsupported`.

Current Library entry points use singular **Provider token…**: Library actions menu, provider row, Jira/Confluence item menu, Add-dialog sign-in failure, Jira attachments panel and an issue's missing-token state line. Commands uses plural **Provider tokens…** in its Library group. The dialog title remains **Provider tokens**.

Label owners are [LibraryViewerDialogs.tsx](../src/app/library/LibraryViewerDialogs.tsx), [LibraryTree.tsx](../src/app/library/LibraryTree.tsx), [LibraryItemHeader.tsx](../src/app/library/LibraryItemHeader.tsx), [AddContextSourceStep.tsx](../src/app/library/AddContextSourceStep.tsx), [AddContextOptionsStep.tsx](../src/app/library/AddContextOptionsStep.tsx) and [commands.ts](../src/app/shell/commands.ts).

Choose **Personal access token (Bearer)** or **Email and API token (Basic)**. New forms default to Basic for resolved Cloud and Bearer for Data Center, including explicit overrides; both remain selectable and replacements retain stored kind. A colon in the Basic email produces an inline error without clearing the token. Status says **Token stored** plus kind, **No token stored**, or **Keyring unavailable**. Closing a dialog restores the opener or nearest surviving ancestor; attachment headers wait for token status before claiming download state.

Vault calls time out at 20 seconds and values are cached per process; edits in Seahorse/KWallet appear after restart. Removal blocks reads until another token is stored, but existing Library items remain. Linux verification guidance uses an isolated `dbus-run-session` keyring; historical exercised coverage and platform limitations are in [verification-log](verification-log.md), not a promise of current native/macOS coverage.

## Library synchronization and traversal values

Jira/Confluence synchronization is enabled while native Cockpit or `cockpit serve` runs. `[library_sync]` is separate from `[limits]` and never raises existing page/item limits. Defaults:

```toml
[library_sync]
enabled = true
delta_minutes = 60
lag_allowance_minutes = 5
overlap_minutes = 30
inventory_hours = 24
audit_days = 7
related_hours = 24
background_min_interval_seconds = 10
background_in_flight = 1
hourly_request_cap = 300
```

Inclusive bounds are: `delta_minutes` 1–10080, `lag_allowance_minutes` 0–1440, `overlap_minutes` 0–10080, `inventory_hours` and `related_hours` 1–8760, `audit_days` 1–365, `background_min_interval_seconds` 1–3600, `background_in_flight` 1–32 and `hourly_request_cap` 1–100000. Validation is in [config.rs](../crates/cockpit-core/src/config.rs).

The coordinator starts after one minute and wakes every minute, coalescing overdue schedules. Jira uses epoch-millisecond `[lower, upper)` JQL windows. Confluence fixed `lastmodified` CQL envelopes round outward to minutes and widen by UTC ±14 hours, a discovery superset rather than an exact timestamp filter. Equal revisions/hierarchy skip bodies; default daily inventories backstop membership/hierarchy. Durable checkpoints live beneath the Library at `.cockpit/sync/state.json`; [DECISIONS](../DECISIONS.md) defines boundary, retries, accumulation, audits and incomplete-discovery safety.

Background pacing defaults to one request per 10 seconds, one in flight and a rolling 300-request hourly budget per origin, shared across provider instances. Manual requests share the origin pacer with two in flight and one-second spacing. Interactive priority, server cooldown and `Retry-After` govern both lanes. Mounted visible listings check the first-page generation roughly every 60 seconds; manual refresh remains distinct. Safety and publication rules belong in [DECISIONS](../DECISIONS.md).

`reference_depth` is 0–5 on add requests and stored items/follows, with `included_by` reasons. Traversal budgets are Single: 32 items, 8 MiB, 60 seconds; Query: 100 items, 16 MiB, 180 seconds. Confluence page adds and space follows stay at depth 0. Jira follows use the configured `library_space_pages` bound; key moves retain identity/refusal semantics, not numeric-ID migration. Live-follow removal grace is 14 days and edited Library items are retained. See [DECISIONS](../DECISIONS.md) for complete-enumeration, lock/ref and seed-preservation requirements.

Attachments are metadata-only by default; downloads are explicit. Jira's bounded attachment list reports `source_attachments_partial` for malformed or beyond-256 entries. PNG/JPEG previews are supported; PDF has no active renderer and SVG/HTML never execute. Attachment identity and removal behavior remain in [DECISIONS](../DECISIONS.md).

## OMP quota and extension paths

`COCKPIT_OMP_EXECUTABLE` overrides `[quota] omp_executable`. With neither, Cockpit finds executable `omp` through `PATH`, then `$HOME/.local/bin`, `$HOME/.bun/bin`, `/opt/homebrew/bin`, `/usr/local/bin`, `/home/linuxbrew/.linuxbrew/bin`, then falls back to bare `omp`. Discovery never overrides an explicit selection, modifies `PATH` or runs shell startup files. Existing `PI_CODING_AGENT_DIR` authentication overrides are inherited by the child. Restart Cockpit after configuration changes; usage-source-not-found means launch failure, not a JSON parse failure.

The embedded integration defaults to a private `<state_root>/orchestration/omp/` materialization. `COCKPIT_OMP_EXTENSION` overrides `[orchestration] omp_extension`, which must be absolute. Installer receipts separately own the distributed extension artifact; launches select it per process with `-e`. No global OMP configuration/authentication mutation is implied. [native-install](native-install.md) owns installation instructions; [DECISIONS](../DECISIONS.md) owns caller, startup and grant authority.

## Native window and Browser settings

```toml
[window]
scale_factor = 1.0
decorations = true
```

`scale_factor` is finite, 0.2–10.0 inclusive, and controls WebView page scale. `decorations` controls title bar/borders. These defaults apply on every native platform; the browser client ignores them.

Install Node and Playwright CLI before opening Browser. The host embeds the capture helper and attaches the same CLI-managed Chromium session, not a second browser for the inline view. Environment overrides `[browser]` settings:

| TOML key | Environment override |
| --- | --- |
| `playwright_cli` | `COCKPIT_PLAYWRIGHT_CLI` |
| `chromium_executable` | `COCKPIT_CHROMIUM_EXECUTABLE` |
| `node_executable` | `COCKPIT_NODE_EXECUTABLE` |
| `browser_helper` | `COCKPIT_BROWSER_HELPER` |
| `playwright_core` | `COCKPIT_PLAYWRIGHT_CORE` |
| `default_url` | `COCKPIT_BROWSER_DEFAULT_URL` |

`playwright_core` names the package paired with the CLI. `default_url` is validated at load and defaults to `about:blank`; fresh leaf creation uses it, while CLI Open/existing-leaf Reconnect retain attach/new-page semantics. Missing/invalid prerequisites are checked lazily at attach and name the corresponding variable/key; [native-install](native-install.md) explains diagnostic distinctions and retry. Browser lifetime, current-run feedback and identity-proven cleanup rules are in [DECISIONS](../DECISIONS.md).
