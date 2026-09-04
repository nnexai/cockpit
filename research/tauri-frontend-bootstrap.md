# Tauri frontend bootstrap

## Scope and recommendation

Cockpit should have one browser frontend at repository root, built once as static files and consumed in two ways: bundled by the Linux Tauri host, and served by `cockpit serve`. This follows the architecture in `CONTEXT.md`: Tauri is a native host, while the browser path is the same frontend behind a loopback HTTP/WebSocket gateway. Do not create a second frontend, a Tauri-specific UI, or a framework layer that is not required by this split.

The conservative target is **Tauri 2.x**, **React 19.x**, **Vite 8.x**, **TypeScript 7.x**, **Bun 1.x**, and **xterm.js 6.x**. Pin exact versions in the lockfile; use the major ranges below only as bootstrap policy. As observed in the official npm registry on 2026-09-03, `@tauri-apps/api` is 2.11.1, `@tauri-apps/cli` is 2.11.4, Vite is 8.2.2, TypeScript is 7.0.2, React is 19.2.8, `@xterm/xterm` is 6.0.0, and `@xterm/addon-fit` is 0.11.0.

## Package roles and proposed layout

```text
package.json                 # single frontend/tooling package and scripts
bun.lock                     # committed dependency lockfile
index.html                   # Vite entry document
src/
  main.tsx                   # React bootstrap
  app/                        # presentation and UI state
  client/                     # versioned CockpitClient interface + adapters
    native.ts                 # thin @tauri-apps/api invoke/channel adapter
    browser.ts                # HTTP/WebSocket adapter for cockpit serve
  terminal/                   # xterm renderer lifecycle; no PTY ownership
  styles/
src-tauri/
  Cargo.toml, src/, build.rs
  tauri.conf.json
  capabilities/default.json
public/                      # only assets that must be copied verbatim
```

Runtime dependencies:

* `react` and `react-dom` (React UI and DOM renderer), both 19.x.
* `@tauri-apps/api` 2.x, used only by `client/native.ts` for commands/channels and native capability-gated APIs. The browser adapter must not import it on its execution path.
* `@xterm/xterm` 6.x and `@xterm/addon-fit` 0.x (currently 0.11.x). Import the renamed scoped packages (`@xterm/xterm`, `@xterm/addon-fit`), not the deprecated `xterm` and `xterm-addon-fit` names. Include `@xterm/xterm/css/xterm.css` from the UI entry or terminal stylesheet.

Development dependencies:

* `vite` 8.x and `@vitejs/plugin-react` 6.x (6.1.1 observed; its peer dependency accepts Vite `^8.0.0`) for JSX/React fast refresh.
* `typescript` 7.x, with a strict `tsconfig` and `noEmit`; Vite performs bundling while `tsc --noEmit` is the independent type contract check.
* `vitest` 4.x for deterministic unit/contract tests colocated with client adapters and protocol mapping. Vitest is Vite-powered and reads Vite configuration by default; use `bun run test` (not `bun test`, which invokes Bun’s different test runner).
* `@types/react` and `@types/react-dom` matching React 19; `@types/node` only if Vite config or test setup uses Node APIs.

Avoid adding a router, state-management library, CSS framework, component kit, or frontend WebSocket/HTTP framework at bootstrap. `fetch`, the platform WebSocket API, React state/hooks, and the existing versioned protocol are sufficient. Add a package only when an observable requirement cannot be met by these primitives.

## Build output and Tauri wiring

Vite’s production output should be the repository-root `dist/` directory. Tauri’s `src-tauri/tauri.conf.json` should set:

```json
{
  "build": {
    "beforeDevCommand": "bun run dev",
    "beforeBuildCommand": "bun run build",
    "devUrl": "http://localhost:5173",
    "frontendDist": "../dist"
  }
}
```

`frontendDist` is relative to `src-tauri`, so `../dist` points at the shared Vite output. `devUrl` is used only by Tauri development; it does not turn Tauri into the browser server. `cockpit serve` should serve the same `dist/` contents and its HTTP/WebSocket API from one loopback origin. Keep Vite’s `base` at `/` unless the gateway deliberately serves under a prefix. If a future packaging layout changes, update both the gateway static-root setting and `frontendDist` together rather than copying or maintaining a second asset tree.

Vite config responsibilities are intentionally small: register the React plugin, set the root (repository root), keep `build.outDir = "dist"`, and configure a development proxy only if it is needed to reach a separately running `cockpit serve` API. The browser adapter should use relative URLs in production so one loopback origin works without CORS configuration. Native startup selects the Tauri adapter; browser startup selects the browser adapter; UI components never branch on transport details.

Minimal scripts (names are recommendations, not code to add in this report):

```json
{
  "dev": "bunx --bun vite",
  "build": "tsc --noEmit && bunx --bun vite build",
  "preview": "bunx --bun vite preview",
  "typecheck": "tsc --noEmit",
  "test": "vitest run",
  "test:watch": "vitest"
}
```

A separate `tauri:dev` script may invoke `bunx tauri dev`, and `tauri:build` may invoke `bunx tauri build`; these are host commands, not alternate frontend builds. Bun’s official Vite guide documents `bun create vite`, `bun install`, `bunx --bun vite`, and the `bun run dev` workflow. Vite’s guide documents the `dev`, `build`, and `preview` commands and the `dist` static production model.

## Capabilities and custom commands

Create an explicit `src-tauri/capabilities/default.json` and reference it in `app.security.capabilities` in `tauri.conf.json`. Do not rely on broad implicit defaults for a load-bearing client. The default capability should target the named main window and contain only the core permissions actually needed (for example `core:event:default`, `core:window:default`, and `core:app:default`; add others only when code uses them).

Tauri v2’s capability documentation states that capability files in `src-tauri/capabilities` are automatically enabled unless `app.security.capabilities` explicitly selects a set. It also states an important default: commands registered through `tauri::Builder::invoke_handler` are allowed to all app windows/webviews unless restricted using `tauri_build::AppManifest::commands`. Therefore:

1. Keep handlers thin and expose only application-level Cockpit operations, never shell/socket forwarding.
2. Explicitly restrict the registered command set with `AppManifest::commands` in `build.rs` once the command list exists.
3. Treat command allowlisting and window targeting as separate controls; do not assume a capability file alone narrows custom invoke handlers.
4. Do not grant filesystem, process, shell, or network plugin permissions unless a concrete frontend operation requires that plugin and its scope can be bounded.
5. Do not add remote URL capability entries: the native frontend is bundled code, and the browser frontend is not a Tauri target.

This keeps the native boundary consistent with `CONTEXT.md` and avoids accidentally giving a compromised WebView a general local-system API.

## Development, test, and typecheck choices

Use Bun as the package manager and script runner, but install Node 22 LTS as a compatibility prerequisite for Vite/Tauri tooling. The current Vite package declares Node `^20.19.0 || >=22.12.0`; current Vitest documentation requires Node `>=22.12.0` and Vite `>=6.4.0`. Bun can run Vite (`bunx --bun vite`) and install packages, but pinning a supported Node LTS as well removes ambiguity for tools whose engines or native packages assume Node.

`typecheck` is `tsc --noEmit`, independent of Vite transpilation. `build` should run it before `vite build` so the static artifact is not produced from type-invalid source. Vitest is the minimal test choice because it shares Vite’s module resolution/configuration and supports the required protocol/client contract tests. Start with tests for transport selection, command/event mapping, ordered event handling, and reconnect/error transitions; do not add a browser E2E stack or React component test library until an actual browser-only contract requires it. Rust protocol and native smoke tests remain in the Rust workspace, as specified by the architecture.

## Linux prerequisites and local risks

Official Tauri v2 Linux prerequisites list WebKitGTK 4.1 development headers, a C/C++ build toolchain, `curl`, `wget`, `file`, `libxdo` development headers, OpenSSL development headers, Ayatana AppIndicator development headers, and librsvg development headers. Tauri also requires Rust (the official guide recommends rustup). For this Fedora workstation, use the distribution equivalents rather than copying the guide’s Debian `apt` command: verify the installed package names for `webkit2gtk4.1-devel`, `gcc-c++`/`make` (or the Fedora build tool group), `libXdo-devel`, `openssl-devel`, `libayatana-appindicator-gtk3-devel`, and `librsvg2-devel`; package naming may vary by Fedora release. A working graphical session and WebKitGTK runtime are also required to launch the native window.

The local machine is x64 Linux, so the Tauri CLI’s x86_64 Linux target is the relevant native package. Keep Rust and frontend toolchains independently pinned: a frontend lockfile does not pin Cargo dependencies. Bun’s native optional dependencies and Tauri’s platform binary must be resolved on the target architecture, so avoid copying `node_modules` between platforms; commit `bun.lock` and reinstall on Linux.

Risks specific to the shared build:

* Native and browser adapters can drift if they are allowed to leak into UI components. Keep a single `CockpitClient` interface and select the adapter at startup.
* Tauri’s `frontendDist` and the gateway static root can silently diverge. Treat `dist/` as the one artifact and make both consumers point there.
* Tauri capability files are security policy, not merely boilerplate. New commands/plugins require a deliberate permission review and command allowlist update.
* xterm rendering is browser UI only; Herdr owns PTYs, scrollback authority, attachment, input, and resize semantics. Do not introduce `node-pty` or another frontend PTY package.
* Tauri’s Linux WebKitGTK behavior and Fedora package names are local integration risks; record the exact installed package/version during workstation bootstrap.
* Current registry “latest” values can move independently. Resolve and commit exact versions after selecting compatible React/Vite/plugin/TypeScript versions; do not blindly use floating `latest` in reproducible bootstrap instructions.

## Primary sources

* Tauri prerequisites: <https://v2.tauri.app/start/prerequisites/>
* Tauri project structure and frontend bundling: <https://v2.tauri.app/start/project-structure/>
* Tauri configuration (`devUrl`, `frontendDist`, build commands): <https://v2.tauri.app/reference/config/>
* Tauri capabilities and custom-command defaults: <https://v2.tauri.app/security/capabilities/>
* Vite getting started, React TypeScript template, scripts, and Node requirement: <https://vite.dev/guide/>
* Bun’s official Vite workflow: <https://bun.sh/guides/ecosystem/vite>
* Vitest installation, Vite integration, Node requirement, and Bun invocation warning: <https://vitest.dev/guide/>
* `@tauri-apps/api` registry metadata (2.11.1 observed): <https://registry.npmjs.org/@tauri-apps%2Fapi/latest>
* `@tauri-apps/cli` registry metadata (2.11.4 observed): <https://registry.npmjs.org/@tauri-apps%2Fcli/latest>
* `@xterm/xterm` registry metadata (6.0.0 observed): <https://registry.npmjs.org/@xterm%2Fxterm>
* `@xterm/addon-fit` registry metadata (0.11.0 observed): <https://registry.npmjs.org/@xterm%2Faddon-fit>
* Vite registry metadata (8.2.2 observed): <https://registry.npmjs.org/vite/latest>
* `@vitejs/plugin-react` registry metadata (6.1.1 observed): <https://registry.npmjs.org/@vitejs%2Fplugin-react/latest>
* React registry metadata (19.2.8 observed): <https://registry.npmjs.org/react/latest>

## Bootstrap decisions

* **Adopt:** one root Vite/React/TypeScript frontend; output `dist/`; Tauri `frontendDist: "../dist"`; `cockpit serve` serves that same output.
* **Adopt:** Tauri 2.x, React 19.x, Vite 8.x, `@vitejs/plugin-react` 6.x, TypeScript 7.x, Bun 1.x, `@xterm/xterm` 6.x, and `@xterm/addon-fit` 0.x; resolve exact compatible patch versions into `bun.lock`.
* **Adopt:** `@tauri-apps/api` 2.x only in a thin native client adapter; browser code uses platform HTTP/WebSocket APIs and relative URLs.
* **Adopt:** `bunx --bun vite` for dev/build scripts, `tsc --noEmit` for typecheck, and Vitest 4.x for unit/protocol contract tests. Run Vitest with `bun run test`, not `bun test`.
* **Adopt:** explicit capability file plus explicit `AppManifest::commands` allowlist; begin with no filesystem/process/shell permissions and add narrowly scoped permissions only when required.
* **Adopt:** Fedora-specific equivalents of Tauri’s WebKitGTK 4.1, compiler, XDo, OpenSSL, Ayatana AppIndicator, and librsvg development prerequisites, plus Rust and Node 22 LTS alongside Bun.
* **Reject:** deprecated unscoped `xterm`/`xterm-addon-fit` package names; use the scoped packages.
* **Reject:** a second browser/native frontend, a Tauri browser target, a frontend PTY implementation, or an unneeded router/state/CSS/component framework.
* **Reject:** copying Debian `apt` commands as Fedora instructions; package names must be resolved on the local workstation.
* **Observed commands/versions:** official Bun guide uses `bun create vite`, `bun install`, `bunx --bun vite`; Tauri CLI/API 2.11.4/2.11.1; Vite 8.2.2; `@vitejs/plugin-react` 6.1.1; TypeScript 7.0.2; React 19.2.8; `@xterm/xterm` 6.0.0; fit addon 0.11.0.
* **Unresolved blockers:** exact Fedora package names and installed versions need workstation inspection; exact patch versions still need to be resolved and committed as one compatible lockfile; Cargo/Tauri Rust crate versions and the concrete Cockpit command list are not present in the current repository, so capability command allowlisting cannot yet be filled with final names.
