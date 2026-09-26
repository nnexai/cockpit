# Cockpit repository bootstrap shape

## Scope and constraints

`CONTEXT.md` and `DECISIONS.md` confirm one Cockpit product with a reusable Rust core, a versioned protocol, a Herdr adapter, CLI/gateway and Tauri hosts, and one shared frontend with native and browser client adapters. The target is Linux and Herdr 0.8.2. This report deliberately describes repository shape only; it does not add implementation code, provider behavior, or a second authority beside Herdr.

The structure should make the confirmed dependency seams visible in the filesystem and in Cargo's package graph. Rust packages are independently testable, while the frontend remains one deployable package so native and browser builds cannot silently drift.

## Layouts considered

### Layout A: seam-aligned Rust workspace plus one frontend package (recommended)

```text
cockpit/
├── Cargo.toml                         # virtual Cargo workspace
├── Cargo.lock
├── crates/
│   ├── cockpit-protocol/
│   ├── cockpit-core/
│   ├── cockpit-herdr/
│   ├── cockpit-providers/
│   ├── cockpit-host/
│   └── cockpit-tauri/
├── frontend/
│   ├── package.json
│   ├── package-lock.json              # or the repository's selected lockfile
│   ├── tsconfig.json
│   ├── vite.config.*
│   ├── index.html
│   └── src/
│       ├── client/
│       │   ├── CockpitClient.ts       # transport-neutral injected contract
│       │   ├── tauri.ts                # Tauri commands/channels adapter
│       │   └── browser.ts              # HTTP/WebSocket adapter
│       ├── protocol/                   # generated TS bindings, eventually
│       ├── app/
│       └── main.ts                    # startup adapter selection
└── apps/
    └── cockpit-tauri/
        ├── tauri.conf.json
        ├── capabilities/
        └── icons/                     # only when native packaging needs them
```

This is the smallest layout that gives each confirmed Rust seam a package without making the frontend a multi-package monorepo. It also leaves the Tauri configuration beside the native app while the reusable Tauri bridge stays in `crates/cockpit-tauri`.

### Layout B: two Rust packages with internal modules

```text
cockpit/
├── Cargo.toml
├── crates/
│   ├── cockpit-core/                  # protocol, application, Herdr/providers modules
│   └── cockpit-host/                  # CLI, gateway, Tauri host modules
├── frontend/
└── apps/cockpit-tauri/
```

This has fewer manifests and is reasonable for a very early proof of concept. It is rejected as the bootstrap recommendation because protocol DTOs become coupled to application implementation, Herdr-specific dependencies become easy to leak into core, and native/gateway host code can accidentally share transport assumptions. Rust module boundaries document intent but do not enforce the dependency direction as strongly as package boundaries.

### Layout C: separate frontend packages for contract and adapters

```text
cockpit/
├── Cargo.toml
├── crates/...                          # as in Layout A
├── packages/
│   ├── client-contract/
│   ├── client-tauri/
│   ├── client-browser/
│   └── ui/
└── apps/cockpit-tauri/
```

This is credible once multiple frontends or independently versioned adapters exist. It is rejected for the first bootstrap: it adds npm workspace/tooling and package-release boundaries before there are multiple consumers. A single `frontend` package can still keep `client`, `app`, and generated protocol directories separate and can later be split without changing the contract.

## Recommended concrete tree

The initial tree should be exactly this (files that are not required for the first foundation are intentionally omitted):

```text
.
├── Cargo.toml
├── Cargo.lock
├── crates
│   ├── cockpit-protocol
│   │   ├── Cargo.toml
│   │   └── src/lib.rs
│   ├── cockpit-core
│   │   ├── Cargo.toml
│   │   └── src/lib.rs
│   ├── cockpit-herdr
│   │   ├── Cargo.toml
│   │   └── src/lib.rs
│   ├── cockpit-providers
│   │   ├── Cargo.toml
│   │   └── src/lib.rs
│   ├── cockpit-host
│   │   ├── Cargo.toml
│   │   ├── src/lib.rs
│   │   └── src/bin/cockpit.rs
│   └── cockpit-tauri
│       ├── Cargo.toml
│       └── src/lib.rs
├── apps
│   └── cockpit-tauri
│       ├── tauri.conf.json
│       └── capabilities/default.json
└── frontend
    ├── package.json
    ├── package-lock.json
    ├── tsconfig.json
    ├── vite.config.ts
    ├── index.html
    └── src
        ├── client
        │   ├── CockpitClient.ts
        │   ├── tauri.ts
        │   └── browser.ts
        ├── protocol                 # generated bindings eventually
        ├── app
        │   └── main.ts
        └── styles.css
```

The `src/bin/cockpit.rs` entry point is part of `cockpit-host`; the unusual tree notation above means `src/lib.rs` and `src/bin/cockpit.rs` are siblings. The package name remains `cockpit-host`, while its executable is named `cockpit`.

### Cargo workspace and package membership

The root is a virtual workspace, with `resolver = "2"`, and members:

```toml
[workspace]
members = [
  "crates/cockpit-protocol",
  "crates/cockpit-core",
  "crates/cockpit-herdr",
  "crates/cockpit-providers",
  "crates/cockpit-host",
  "crates/cockpit-tauri",
]
resolver = "2"
```

Use the package names `cockpit-protocol`, `cockpit-core`, `cockpit-herdr`, `cockpit-providers`, `cockpit-host`, and `cockpit-tauri`. The `cockpit` binary is produced by `cockpit-host`; do not create a second CLI package merely to hold a one-binary host. The Tauri app directory is packaging/configuration, not an additional Cargo member; its Rust entry point depends on `cockpit-tauri`.

The frontend is not a Cargo member and is not initially an npm workspace. Its one `package.json` owns the browser build and the static assets consumed by Tauri. Pin Tauri's JavaScript API and Vite/toolchain versions in its package lock; use the repository's one chosen package-manager lockfile rather than multiple competing lockfiles.

## Dependency direction

```text
cockpit-protocol
       ▲
       │
 cockpit-core ◄──── cockpit-herdr ────► Herdr 0.8.2 socket/CLI/schema
       ▲                  ▲
       │                  │
 cockpit-providers        │
       ▲                  │
       └──────── cockpit-host (cockpit CLI + serve gateway)
                          │
                          └──── cockpit-tauri ────► Tauri v2 IPC/channels

frontend/src/client/CockpitClient.ts
       ├── browser.ts ── HTTP/WebSocket ── cockpit-host
       └── tauri.ts ─── Tauri commands/channels ─── cockpit-tauri
```

More precisely:

- `cockpit-protocol` depends only on serialization/schema primitives and contains versioned request, response, error, and event types. It must not depend on Herdr, Tauri, HTTP, WebSocket, or socket framing.
- `cockpit-core` depends on `cockpit-protocol`. It owns application services, lifecycle rules, authorization, idempotency, freshness, and provider/context orchestration. It must not depend on host transports.
- `cockpit-herdr` depends on `cockpit-protocol` and `cockpit-core` interfaces/types as needed. It alone owns Herdr 0.8.2 newline-delimited JSON framing, schema gating, reconnect, session selection, terminal attachment, and Herdr identifiers.
- `cockpit-providers` depends on `cockpit-protocol` and `cockpit-core`; it owns capability-detected external CLI adapters and normalization. The first concrete provider can be added here without changing host packages.
- `cockpit-host` depends on protocol, core, Herdr, and providers. It composes services, implements CLI commands, and exposes `cockpit serve` as a loopback HTTP/WebSocket gateway. HTTP/WebSocket code stays here, not in core.
- `cockpit-tauri` depends on protocol, core, and Herdr (and only the Tauri crates needed for commands/channels). It is a thin native transport host; business rules stay in core.
- The frontend protocol-facing TypeScript types are generated from the versioned Rust protocol contract, not hand-maintained as a second schema. `CockpitClient.ts` exposes protocol types and transport-neutral operations; `tauri.ts` and `browser.ts` only map transport mechanics.
- There is no dependency from `cockpit-core` to either host, and no dependency from frontend UI components to either adapter. Adapter selection happens once in `main.ts`.

This follows Cargo's package/workspace model ([Cargo workspaces](https://doc.rust-lang.org/cargo/reference/workspaces.html)) and Tauri's documented native host/frontend split ([Tauri architecture](https://v2.tauri.app/concept/architecture/), [commands and channels](https://v2.tauri.app/develop/calling-rust/)). Herdr's documented socket API is the external authority for the adapter seam ([Herdr socket API](https://herdr.dev/docs/socket-api/)).

## Generated protocol bindings

The source of truth eventually lives in `crates/cockpit-protocol`, with an explicit protocol version namespace (for example `protocol::v1`). Generated browser bindings should be emitted into `frontend/src/protocol/generated/` (or the equivalent generated subdirectory under `frontend/src/protocol`), never into `cockpit-core` or a host crate. Generation should be a deliberate command/build step from the Rust protocol declarations; generated output must be versioned with the frontend build so native and browser clients consume the same DTO names and error/event discriminants. Do not introduce a separate hand-authored TypeScript schema or a runtime schema registry in the bootstrap.

If a later generator requires an intermediate schema artifact, place it under `crates/cockpit-protocol/schema/` and keep it versioned beside the Rust protocol source. The first bootstrap need only reserve `frontend/src/protocol/`; it does not need to select or install a generator before the protocol types have stabilized.

## Bootstrap file inventory

Required first-bootstrap files:

- root `Cargo.toml` virtual workspace manifest and `Cargo.lock`;
- one `Cargo.toml` plus `src/lib.rs` for each of the six Rust packages;
- `crates/cockpit-host/src/bin/cockpit.rs` for the single `cockpit` executable;
- `apps/cockpit-tauri/tauri.conf.json` and its default capability declaration;
- `frontend/package.json`, package-manager lockfile, `tsconfig.json`, `vite.config.ts`, and `index.html`;
- `frontend/src/client/CockpitClient.ts`, `tauri.ts`, and `browser.ts`;
- `frontend/src/app/main.ts` and minimal shared UI entry files;
- protocol-versioned Rust source and the reserved frontend protocol directory.

The manifests may initially expose only the foundation commands and mirror/attach path, but package boundaries must exist from the first commit. A package with no implementation should not be added solely as a placeholder; provider behavior can begin as a real capability boundary in `cockpit-providers`, and generated bindings can wait until the protocol has concrete operations.

## Deliberate omissions from the first bootstrap

Do not add a second daemon/session registry, a standalone protocol server, raw shell or raw socket forwarding, remote authentication, settings UI, provider-specific mutation commands, automatic OMP setup/launch, a provider matrix, an npm monorepo, a Tauri browser target, or a Tauri sidecar Herdr server. Do not duplicate protocol types by hand in TypeScript, put business rules in Tauri handlers, or make Tauri own Herdr's lifecycle. Do not create generated `INDEX.md`/scratchpad context files or an independent workspace registry. These omissions preserve the confirmed authority and avoid scaffolding deferred behavior.

## Bootstrap decisions

- **Recommend Layout A:** six narrowly named Rust packages plus one shared `frontend` package. It preserves every confirmed seam while avoiding premature npm package/version boundaries.
- **Package/version choices:** use a virtual Cargo workspace with `resolver = "2"`; package names are `cockpit-protocol`, `cockpit-core`, `cockpit-herdr`, `cockpit-providers`, `cockpit-host`, and `cockpit-tauri`; the executable is `cockpit`. Target the installed/documented Herdr release **0.8.2** and gate adapter behavior on its schema/capabilities.
- **Dependency rule:** protocol → core → adapters/hosts; hosts may compose adapters, but core never imports Tauri, HTTP, WebSocket, or Herdr framing. Frontend UI imports only `CockpitClient`; native and browser adapters are selected at startup.
- **Generated bindings:** eventual generated TypeScript protocol bindings live under `frontend/src/protocol/generated/`, sourced from versioned Rust protocol declarations under `crates/cockpit-protocol` (with any intermediate schema beside that crate). No generator is required for the initial empty reservation.
- **Rejected alternatives:** two-package internal modules hide enforceable seams; separate frontend packages add tooling and release complexity before there are multiple consumers. A Tauri sidecar/second daemon would duplicate lifecycle authority and is explicitly out of scope.
- **Commands/versions observed or targeted:** Cargo virtual workspace (`resolver = "2"`), Tauri v2 native host, shared static Vite-style frontend, `cockpit serve` loopback gateway, and Herdr 0.8.2. The first native platform is Linux. Relevant primary references are [Cargo workspaces](https://doc.rust-lang.org/cargo/reference/workspaces.html), [Tauri architecture](https://v2.tauri.app/concept/architecture/), [Tauri commands/channels](https://v2.tauri.app/develop/calling-rust/), and [Herdr socket API](https://herdr.dev/docs/socket-api/).
- **Unresolved blockers:** the exact Herdr 0.8.2 schema fixture and the protocol-to-TypeScript generator are not selected by the architecture documents. Choose both only after inspecting the installed schema and stabilizing the first protocol operations; neither should widen the initial repository shape.
