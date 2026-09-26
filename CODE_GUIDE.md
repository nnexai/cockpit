# Code guide

Herdr owns live sessions, Spaces, tabs, panes, focus and layout. Cockpit projects that state and owns task setup, Context assets, local Review snapshots and comment batches. An HTTP response arriving later does not make it newer than an ordered stream event.

## Where to change behavior

| Change | Owner | Useful verification |
| --- | --- | --- |
| Public request/response shape | `crates/cockpit-protocol/src/` and its TypeScript exporter | Protocol tests and generated-file check |
| Session ordering | `src/client/streamOrder.ts`, `src/app/session/sessionStore.ts` | Ordering corpus and reducer tests |
| Focus, commands and layout projection | `src/app/session/focusCoordinator.ts`, `mutationCoordinator.ts`, `src/app/layout/` and `src/app/input/` | App integration tests and an owned runtime smoke |
| Terminal lifecycle and input | `src/app/TerminalPane.tsx`, `crates/cockpit-herdr/src/terminal_wire.rs` | Lifecycle/race tests and successive runtime frames |
| Herdr methods and process evidence | `crates/cockpit-herdr/src/cli/` | Adapter fixtures against the supported schema |
| Task setup and recovery | `crates/cockpit-core/src/projects.rs`, `project_store.rs`, `project_teardown.rs` | Ownership, idempotency and uncertain-outcome fixtures |
| Context path authorization | `crates/cockpit-core/src/context.rs` | Traversal, replacement and companion tests |
| Local snapshot/generated-file writes | `crates/cockpit-core/src/context_assets.rs` | Dirty/untracked snapshots, generated refresh and user-edit conflict tests |
| Provider authority and source cache | `crates/cockpit-core/src/sources.rs` and Context's source facade | Origin/instance, cache pointer and conflict tests |
| Tea JSON and CLI behavior | `crates/cockpit-providers/src/tea.rs` | Real Tea against a localhost fixture with a fake login |
| Local diff and frozen sources | `crates/cockpit-core/src/review.rs` | Real Git fixtures; compare index and working files before/after |
| Comment text and delivery | `crates/cockpit-core/src/comments/` | Exact payload bytes, CAS recovery and acknowledged paste tests |
| Markdown and media display | `src/app/context/`, `context_media.rs` | Source mapping, hostile input, byte/pixel caps and browser/native rendering |
| Host request decoding and composition | `crates/cockpit-host/src/server/`, `src-tauri/src/` | Equivalent browser/native DTOs and real native startup |

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

A provider selects an executable and an existing Tea login by name. Cockpit does not create credentials or perform remote writes. For example, in a task-specific Cockpit TOML configuration:

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

Source import verifies the companion's primary repository origin against the configured forge instance before starting its CLI. Jira work items have no repository, so they are checked against the configured site instead. A missing login, unavailable adapter or unsupported artifact returns an explicit failure. Generated source files carry provenance and a last-written hash; a refresh preserves user edits. Raw frontmatter never grants overwrite permission.

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
