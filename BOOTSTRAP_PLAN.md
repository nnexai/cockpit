# Cockpit bootstrap plan

Status: implementation authority for the initial repository bootstrap only.

This phase proves the repository shape and one real end-to-end status operation. It does not claim the Herdr session mirror, terminal attachment, or workspace lifecycle is implemented. Those begin in the next phase.

## Decisions

- Use one Cargo workspace, one `cockpit` executable, one shared frontend package at the repository root, and one standard Tauri v2 host.
- Keep four reusable Rust modules: protocol, core, Herdr adapter, and host. The Tauri package is an application host, not a reusable crate.
- Omit `cockpit-providers` until the first Gitea adapter contains real behavior.
- Rust owns protocol definitions. Generate and commit the TypeScript status contract from Rust. Do not hand-maintain a second DTO schema.
- Use Bun for frontend dependency installation and scripts. Commit `bun.lock`. Do not use pnpm or mutate global tool installations.
- Target Herdr 0.8.2, protocol 20, schema 1. The executable, socket, and named session remain configurable.
- The only bootstrap operation is status inspection. CLI, loopback HTTP, and the browser frontend are implemented and verified now. The Tauri IPC path is implemented but remains unverified until native dependencies are provisioned.
- `cockpit serve` binds only to an IP address for which `is_loopback()` is true. It serves root `dist/` and `/api/v1/status` from one origin.
- `cockpit serve` has no separate browser authentication in this phase. `CONTEXT.md` and `DECISIONS.md` now state this directly: native access uses Tauri capabilities, while the browser gateway remains loopback-only and must not be exposed remotely. This phase does not claim loopback binding is authentication.
- `--test-mode` is deterministic and honest. It disables the live Herdr probe and reports `mode: test` with Herdr unavailable because live inspection is disabled. It does not invent sessions, panes, terminals, or agents.
- Defer WebSocket routes until a real ordered event exists. Defer xterm.js until a real terminal frame stream exists.

## Repository tree

```text
.
├── Cargo.toml
├── Cargo.lock
├── rust-toolchain.toml
├── package.json
├── bun.lock
├── tsconfig.json
├── vite.config.ts
├── index.html
├── .editorconfig
├── .gitignore
├── src/
│   ├── main.tsx
│   ├── app/App.tsx
│   ├── app/styles.css
│   ├── client/CockpitClient.ts
│   ├── client/browser.ts
│   ├── client/native.ts
│   ├── client/select.ts
│   ├── client/client.test.ts
│   └── protocol/generated/v1.ts
├── crates/
│   ├── cockpit-protocol/
│   │   ├── Cargo.toml
│   │   ├── src/lib.rs
│   │   ├── src/v1.rs
│   │   ├── src/typescript.rs
│   │   ├── src/bin/export-typescript.rs
│   │   └── tests/typescript.rs
│   ├── cockpit-core/
│   │   ├── Cargo.toml
│   │   ├── src/lib.rs
│   │   └── tests/status.rs
│   ├── cockpit-herdr/
│   │   ├── Cargo.toml
│   │   ├── src/lib.rs
│   │   ├── src/cli.rs
│   │   ├── src/schema.rs
│   │   ├── tests/compatibility.rs
│   │   └── tests/fixtures/herdr-0.8.2-protocol-20-schema-1.json
│   └── cockpit-host/
│       ├── Cargo.toml
│       ├── src/lib.rs
│       ├── src/server.rs
│       ├── src/bin/cockpit.rs
│       └── tests/server.rs
└── src-tauri/
    ├── Cargo.toml
    ├── build.rs
    ├── tauri.conf.json
    ├── capabilities/default.json
    ├── src/lib.rs
    └── src/main.rs
```

This root frontend layout deliberately supersedes the earlier `frontend/` option. Tauri can discover sibling `src-tauri`, Vite writes one root `dist/`, and the gateway serves that same directory without cross-package working-directory rules.

The Cargo workspace includes `src-tauri`, but `default-members` contains the four reusable crates. A normal root `cargo check` stays runnable on machines without WebKitGTK. Native checks remain explicit and cannot be reported green until Fedora's Tauri development packages are installed.

## Dependency direction

```text
cockpit-protocol
       ▲
       │
cockpit-core ◄── cockpit-herdr
       ▲              ▲
       ├──────────────┤
       │              │
cockpit-host       src-tauri
       ▲              ▲
       │              │
browser client     native client
       └────── CockpitClient ──────┘
```

Rules:

- `cockpit-protocol` depends only on serialization and TypeScript-export support.
- `cockpit-core` depends on protocol. It knows no Herdr framing, subprocess, HTTP, WebSocket, or Tauri type.
- `cockpit-herdr` depends on core and protocol. It owns fixed-argument Herdr CLI execution, schema parsing, and compatibility checks.
- `cockpit-host` composes core and the Herdr adapter. It owns CLI parsing, Axum, Tokio runtime setup, loopback enforcement, and `tower-http` static files.
- `src-tauri` composes the same core and Herdr adapter. Its command handler only converts the shared result into Tauri IPC.
- Frontend presentation imports `CockpitClient`, never `fetch`, WebSocket, or Tauri functions. Adapter selection occurs once in `client/select.ts`.

## Minimum real behavior

### Protocol

Create a `v1` namespace containing:

- `CockpitMode`, with `normal` and `test`;
- `HerdrIdentity`, with `version`, `protocol`, and `schema_version`;
- `HerdrCompatibility`, tagged as:
  - `compatible { identity }`;
  - `incompatible { identity: optional, code, message }`;
  - `unavailable { code, message }`;
- `StatusResponse`, with `protocol_version`, `cockpit_version`, `mode`, and `herdr`;
- `ErrorResponse`, with stable `code` and `message`.

The TypeScript exporter renders those exact Rust types into `src/protocol/generated/v1.ts`. Its command contract is:

```text
export-typescript --write <path>   # replace the target atomically
export-typescript --check <path>   # compare exact bytes, write nothing, fail on drift
```

The explicit target is resolved from the caller's working directory. `tests/typescript.rs` checks deterministic rendering and check-mode drift. Do not add event or terminal DTOs before a real stream exists.

### Core

Define one small external interface, `HerdrInspector`, and one deep application entry point, `CockpitService::status()`.

`CockpitService` owns these invariants:

- normal mode performs the configured Herdr inspection;
- test mode never invokes Herdr and returns the stable unavailable code `live_inspection_disabled`;
- adapter failures become typed unavailable status rather than fabricated empty state;
- compatible status requires Herdr 0.8.2, protocol 20, schema 1, and the required bootstrap methods;
- hosts receive the same `StatusResponse` regardless of transport.

Core tests use an inspector that records calls. They assert zero calls in test mode and one call in normal mode. No production fake is exposed.

### Herdr adapter

Use `std::process::Command` or Tokio's fixed-argument equivalent. Invoke only:

- `<configured-herdr> [--session <name>] status server --json`;
- `<configured-herdr> [--session <name>] api schema --json`.

A read-only check against the installed 0.8.2 binary ran `herdr status server --help`; its usage is `herdr status server [OPTIONS]` and its options include `--json`. Use that verified server-specific form.

1. explicit Cockpit `--herdr`, `--herdr-session`, and `--herdr-socket` options;
2. `COCKPIT_HERDR_EXECUTABLE`, `HERDR_SESSION`, and `HERDR_SOCKET_PATH`;
3. Herdr's defaults.

The adapter translates an explicit socket to child `HERDR_SOCKET_PATH` and an explicit session to Herdr's global `--session` argument. Supplying both is rejected rather than relying on implicit precedence. Child processes inherit the environment when no explicit override is supplied.

Parse unknown fields leniently. Preserve execution, JSON, version, protocol, schema, and missing-method failures as structured errors. Check for `ping`, `session.snapshot`, and `events.subscribe`. Do not inspect or mutate a live workspace, pane, terminal, or agent.

Capture the installed schema as the protocol-20/schema-1 fixture. The fixture contains no live snapshot, paths, terminal bytes, or secrets.

### CLI and gateway

The `cockpit` binary exposes:

```text
cockpit status [--herdr <executable>] [--herdr-session <name> | --herdr-socket <path>] [--json]
cockpit serve [--bind 127.0.0.1:PORT] [--static-dir <path>] [--herdr <executable>] [--herdr-session <name> | --herdr-socket <path>] [--test-mode]
```

Behavior:

- `status` uses the real core and Herdr adapter.
- `serve` is a foreground process.
- non-loopback bind addresses fail before opening a socket;
- after a successful bind, stdout emits exactly `listening http://<ip>:<actual-port>` so smoke automation can discover an ephemeral port;
- `GET /api/v1/status` returns `StatusResponse`;
- API routes resolve before static fallback, and an unknown `/api/...` route remains a structured 404;
- `/`, real assets, and extensionless SPA routes are served from root `dist/`;
- missing asset paths with file extensions remain 404 and never receive `index.html`;
- a missing or unreadable static root or `index.html` fails startup with stderr and nonzero exit;
- no raw shell, raw socket forwarding, remote bind, CORS wildcard, authentication claim, or daemon mode exists.

`cockpit-host` owns Axum 0.8, Tokio 1, and `tower-http` 0.6 with its `fs` feature. Server tests bind an ephemeral loopback port and make real HTTP requests for `/`, one deep SPA route, one real asset, one missing asset, `/api/v1/status`, and one missing API route. A separate test covers missing startup assets and non-loopback rejection.

### Shared frontend

Use React 19, Vite 8, TypeScript 7, Vitest 4, `@vitejs/plugin-react` 6, `@tauri-apps/api` 2.11.1, `@tauri-apps/cli` 2.11.4, and Bun 1.3. Resolve exact compatible patches into `bun.lock`.

Root scripts include `dev`, `build`, `typecheck`, `test`, `tauri:dev`, and `tauri:build`. The Tauri scripts invoke the project-local `tauri` binary from the repository root, where `src-tauri` is discoverable.

`CockpitClient` exposes only `status(): Promise<StatusResponse>` in this phase.

- Browser adapter calls relative `/api/v1/status` and maps non-2xx or malformed responses to typed client errors.
- Native adapter invokes only `cockpit_status`.
- Selection uses Tauri's documented host detection once at startup.
- The React screen renders actual mode, Cockpit protocol version, and every field of Herdr compatibility. It labels incompatible and unavailable states explicitly.
- Styling takes the graphite palette, type scale, focus treatment, and flat one-pixel separators from `research/ui-design-direction.md`. It must not render fake Spaces, Agents, panes, terminals, or disabled controls that promise later features.

Vitest covers both adapter mappings and startup selection through injected transport functions. No component-test framework or browser E2E dependency is added.

### Tauri host

Create package `cockpit-tauri` in `src-tauri`. Pin `tauri` 2.11.5 and `tauri-build` 2.6.3. The project-local JavaScript CLI remains 2.11.4.

- `cockpit_status` calls the same `CockpitService` behavior as CLI and HTTP.
- Register only `cockpit_status`.
- `build.rs` uses `tauri_build::AppManifest::commands(&["cockpit_status"])`; absence of that pinned interface is a build failure, not an optional fallback.
- `tauri.conf.json` explicitly selects `app.security.capabilities: ["default"]`.
- `capabilities/default.json` targets only the named `main` window and grants the generated allow permission for `cockpit_status` plus only required Tauri core defaults.
- Grant no shell, process, filesystem, remote-origin, or broad network plugin permission.
- `frontendDist` is `../dist`; development and build commands run root Bun scripts.

A static configuration inspection must confirm the explicit capability selection, main-window target, exact custom-command permission, and absence of broad permissions. That inspection is not a native compile or smoke test.

## Dependency policy

- Root `Cargo.toml` is a virtual workspace using edition 2024 and `resolver = "3"`. It owns shared Rust dependency versions and workspace lints.
- `Cargo.lock` and `bun.lock` are committed.
- `rust-toolchain.toml` pins Rust 1.98.0 with rustfmt and clippy.
- `package.json` declares Bun `>=1.3.14 <1.4` and Node `>=22.12`.
- Use `ts-rs` 12.0.1 for deterministic TypeScript generation.
- Do not add pnpm metadata, npm lockfiles, global installation commands, a JavaScript workspace, router, state library, CSS framework, icon library, xterm.js, or provider dependencies in this phase.

## Parallel implementation ownership

After this review is green, dispatch these disjoint slices together:

| Slice | Owner paths | Acceptance |
|---|---|---|
| Rust protocol and root Cargo workspace | `Cargo.toml`, `rust-toolchain.toml`, `crates/cockpit-protocol/`, `src/protocol/generated/v1.ts` | DTOs serialize, exporter writes and checks deterministically, no transport dependencies |
| Core and Herdr adapter | `crates/cockpit-core/`, `crates/cockpit-herdr/` | real installed CLI inspection, fixture-backed compatibility, options/env precedence, honest unavailable state, zero-call test mode |
| CLI and gateway | `crates/cockpit-host/` | real status command, loopback server, exact listening line, precise static fallback, HTTP tests |
| Shared frontend | `package.json`, `bun.lock`, `tsconfig.json`, `vite.config.ts`, `index.html`, `src/` except `src/protocol/generated/v1.ts` | generated DTO consumption, two adapters, real status screen, type and adapter tests |
| Native host and repository config | `src-tauri/`, `.editorconfig`, `.gitignore` | pinned narrow command manifest and capability, shared root frontend, no broad permissions |

Only the protocol owner writes `src/protocol/generated/v1.ts`. The frontend owner imports the exact names in this plan and must not hand-create or overwrite that file.

The core/Herdr owner publishes `CockpitService`, `HerdrInspector`, and `HerdrCliInspector`. Host owners consume those names. If an implementation detail forces a rename, that owner must notify sibling agents before callers are edited.

## Verification runnable now

The orchestrator formats once, then runs:

```bash
cargo fmt --all --check
cargo check
cargo test
cargo clippy --all-targets -- -D warnings
cargo run -p cockpit-protocol --bin export-typescript -- --check src/protocol/generated/v1.ts
bun install --frozen-lockfile
bun run typecheck
bun run test
bun run build
cargo run -p cockpit-host --bin cockpit -- status --json
```

Behavior smoke:

1. Start `cockpit serve --bind 127.0.0.1:0 --static-dir dist --test-mode --herdr /definitely/not/herdr`.
2. Parse the exact `listening http://127.0.0.1:<actual-port>` line.
3. Request `/api/v1/status` and verify protocol version, `mode: test`, code `live_inspection_disabled`, and that startup succeeded despite the invalid Herdr executable.
4. Open the served frontend in a real browser and verify the same fields are rendered without console or failed-network errors.
5. Run normal `cockpit status --json` and verify it reports installed Herdr 0.8.2, protocol 20, schema 1 as compatible.
6. Attempt a non-loopback bind and verify startup rejects it.
7. Start with a missing static root and verify startup exits nonzero with the specific asset error.

Implementation and smoke agents may run only the targeted commands delegated to them. The orchestrator repeats final gates before accepting the phase.

## Native verification blocker

The workstation lacks WebKitGTK 4.1, GTK3, libsoup3, XDo, Ayatana AppIndicator, and librsvg development packages. Therefore these remain blocked until the user approves a Fedora package transaction or equivalent disposable toolbox:

```bash
cargo check --manifest-path src-tauri/Cargo.toml
bun run tauri:dev
```

After provisioning, the native smoke must launch the actual Tauri app, exercise the status screen, verify that the displayed identity came from `cockpit_status`, and fail if the native UI requests `/api/v1/status` or shows an unavailable/blank result. Browser success or static inspection cannot substitute for this.

## Explicit non-goals

- Herdr Unix socket framing, snapshot cache, event subscription, reconnect, session switching, focus, layout, terminal observe/control, takeover, and xterm.js.
- WebSocket routes before real ordered events exist.
- Workspace creation or destruction, companion context, hydration, search, providers, credentials, or OMP setup.
- Remote access, multi-user authentication, background daemon mode, raw shell, or raw socket forwarding.
- A settings screen, router, client state framework, component kit, CSS framework, or fake dashboard data.

The next implementation plan starts the Herdr session mirror using the schema evidence and this verified status path. It must use `research/ui-design-direction.md` before writing the first dashboard or terminal UI.
