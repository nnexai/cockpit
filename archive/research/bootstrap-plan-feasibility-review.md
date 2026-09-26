# Bootstrap plan feasibility review

This review checks the bootstrap phase only: the repository shape, the status operation, the shared frontend, `cockpit serve`, and the Tauri host. It does not recommend implementing the Herdr session mirror or any other next-phase behavior. Findings are ordered by implementation impact on the observed Fedora workstation.

## Blocking findings

### 1. `[BLOCKER]` The Bun `run --cwd` verification commands do not select the frontend directory

- **Evidence:** `BOOTSTRAP_PLAN.md:227-231` prescribes `bun run --cwd frontend typecheck`, `bun run --cwd frontend test`, and `bun run --cwd frontend build`. Bun's runtime syntax is `bun [bun flags] run <script>`; `--cwd` is a Bun-level option and must precede `run`. The Bun runtime documentation also says flags after `run` are passed to the script: <https://bun.sh/docs/runtime>. The install command is different: `bun install --cwd frontend` is documented: <https://bun.sh/docs/pm/cli/install>.
- **Affected plan section:** **Verification runnable now**.
- **Exact correction:** Keep the install command as `bun install --cwd frontend --frozen-lockfile` (or run it from `frontend`), but replace the run commands with:

  ```bash
  bun --cwd frontend run typecheck
  bun --cwd frontend run test
  bun --cwd frontend run build
  ```

  Apply the same ordering rule to every frontend script invocation, including the native script. As written, these checks attempt to run a script named `--cwd` rather than a script in `frontend`, so the proposed frontend gates cannot run.

### 2. `[BLOCKER]` The nested `frontend/` package has no implementable Tauri CLI/project-root invocation

- **Evidence:** The exact tree puts `package.json` and `bun.lock` under `frontend/` (`BOOTSTRAP_PLAN.md:53-68`), while `src-tauri/` is a sibling. The native verification command is `bun run --cwd frontend tauri:dev` (`BOOTSTRAP_PLAN.md:246-255`), and the plan only says that Tauri development/build commands run Bun scripts in `frontend` (`BOOTSTRAP_PLAN.md:190`); it never declares a `tauri:dev` script or a working-directory/configuration strategy. Tauri's project-structure documentation describes the JavaScript project at the app top level and says the CLI discovers `src-tauri/` from the project root: <https://v2.tauri.app/start/project-structure/>. The official CLI docs describe `tauri dev` as using the `src-tauri/tauri.conf.json` marker and running `beforeDevCommand`: <https://v2.tauri.app/reference/cli/>.
- **Affected plan sections:** **Repository tree**, **Shared frontend**, **Tauri host**, and **Native verification blocker**.
- **Exact correction:** Choose and document one topology rather than relying on implicit discovery. The least ambiguous correction is to move the frontend package files/source to the repository root, matching the researched Tauri layout, then use root `bun run tauri:dev`/`bun run tauri:build`, `frontendDist: "../dist"`, and root `dist/` paths. If retaining `frontend/`, add the exact `@tauri-apps/cli` dependency and an explicit root-working-directory invocation (with `beforeDevCommand`/`beforeBuildCommand` explicitly running `bun --cwd frontend run dev/build`); do not use `bun run --cwd frontend tauri:dev` as the Tauri command. The corrected invocation must be exercised from the directory in which the CLI can discover the sibling `src-tauri` project.

### 3. `[HIGH]` The native check selects an undeclared Cargo package and leaves the security-critical Tauri API unpinned

- **Evidence:** The plan's tree names only the directory `src-tauri` and does not declare the package name (`BOOTSTRAP_PLAN.md:69-75`), but native verification hard-codes `cargo check -p cockpit-tauri` (`BOOTSTRAP_PLAN.md:246-253`). Cargo `-p` selects a package *name*, not a directory; the workspace/default-members discussion (`BOOTSTRAP_PLAN.md:78`, `194-200`) does not establish that name. The Tauri research explicitly records that the Cargo/Tauri Rust crate versions and concrete command list are not yet present (`research/tauri-frontend-bootstrap.md:142-143`). The plan says to use `AppManifest::commands` only “where supported” (`BOOTSTRAP_PLAN.md:181-190`), which makes the command allowlist optional despite the stated security boundary. Tauri documents the concrete build-script API as `AppManifest::new().commands(&[...])`: <https://v2.tauri.app/security/capabilities/>.
- **Affected plan sections:** **Repository tree**, **Tauri host**, **Dependency policy**, and **Native verification blocker**.
- **Exact correction:** Either declare `[package] name = "cockpit-tauri"` and pin exact compatible `tauri`/`tauri-build` versions, or make the check package-name-independent:

  ```bash
  cargo check --manifest-path src-tauri/Cargo.toml
  ```

  Pin a Tauri release whose `tauri-build` exposes `AppManifest::commands`, require `build.rs` to allowlist exactly `cockpit_status`, and fail the native build if that API is unavailable. Add the exact project-local `@tauri-apps/cli` version to the frontend package as well; the observed machine has no global `cargo tauri` subcommand (`research/local-toolchain-bootstrap.md:32-36`).

## High-impact implementation findings

### 4. `[HIGH]` The Axum static-serving contract is specified, but its required dependencies and fallback/error routing are not

- **Evidence:** `BOOTSTRAP_PLAN.md:102-104` assigns HTTP/static hosting to `cockpit-host`, while `BOOTSTRAP_PLAN.md:146-165` requires both static assets and SPA fallback, and requires missing assets to produce a specific error rather than a blank success page. No `cockpit-host/Cargo.toml` dependency responsibilities or Axum/tower-http configuration are listed. `tower_http::services::ServeDir` serves missing files as 404 by default and offers `fallback`/`not_found_service`, but an unconditional fallback would also turn missing asset requests into the SPA document: <https://docs.rs/tower-http/latest/tower_http/services/struct.ServeDir.html>.
- **Affected plan section:** **CLI and gateway**.
- **Exact correction:** State that `cockpit-host` owns the `axum` HTTP runtime and `tower-http`'s `fs` feature (plus the selected Tokio/runtime dependencies), while core remains transport-free. Define routing precisely: API paths are handled before static fallback; extensionless browser routes may serve `index.html`; missing assets such as `/assets/missing.js` remain an error; a missing/unreadable static root or `index.html` fails startup. Add server tests for `/`, one deep SPA route, one real asset, a missing asset, `/api/v1/status`, and a missing API route so the claimed behavior is actually proven.

### 5. `[HIGH]` The generated TypeScript check has no declared command contract or test location

- **Evidence:** The tree contains `src/bin/export-typescript.rs` and the generated file (`BOOTSTRAP_PLAN.md:29-35`, `53-68`), and the protocol section promises a drift test (`BOOTSTRAP_PLAN.md:108-119`), but the tree lists no protocol test file and never defines what `export-typescript --check <path>` accepts or how it computes the output. Nevertheless, the final gate invokes exactly that undocumented interface (`BOOTSTRAP_PLAN.md:227`).
- **Affected plan sections:** **Protocol**, **Repository tree**, and **Verification runnable now**.
- **Exact correction:** Specify the exporter CLI contract: `--check <path>` reads the generated target, compares exact bytes with deterministic exporter output, writes nothing, and exits nonzero on mismatch or malformed arguments. Add the test location to the tree (for example `crates/cockpit-protocol/tests/typescript.rs`) or explicitly place the comparison in a named unit test. Define that the target path is resolved from the repository/workspace root, or require an explicit path in every invocation. This makes the generated-file gate implementable rather than an assumed flag.

## Recommendations and verification gaps

### 6. `[MEDIUM]` The frontend layout conflicts with the researched Tauri layout and creates path ambiguity

- **Evidence:** `research/tauri-frontend-bootstrap.md:5-27` calls for a browser frontend and package at repository root, and `research/tauri-frontend-bootstrap.md:47-60` describes root `dist/` and `frontendDist: "../dist"`. The plan instead puts the package under `frontend/` and uses `frontend/dist` (`BOOTSTRAP_PLAN.md:53-68`, `167-190`, `237`). Both can work, but the research and implementation authority prescribe different relative paths and CLI working directories.
- **Affected plan sections:** **Repository tree**, **Shared frontend**, **Tauri host**, and **Verification runnable now**.
- **Exact correction:** Mark the plan's `frontend/` topology as an explicit supersession of the research, then specify every path relative to its owning working directory (`frontend/dist` for the gateway and `../frontend/dist` from `src-tauri`), or adopt the researched root-package layout consistently. Do not leave the two layouts as simultaneous guidance.

### 7. `[MEDIUM]` A virtual Cargo workspace needs an explicit resolver

- **Evidence:** The root tree contains a workspace manifest but no root package (`BOOTSTRAP_PLAN.md:20-52`). Cargo documents that virtual workspaces must set `resolver` explicitly because there is no root package edition from which to infer it: <https://doc.rust-lang.org/cargo/reference/workspaces.html>. The dependency policy names shared versions/lints but does not specify a resolver (`BOOTSTRAP_PLAN.md:194-200`).
- **Affected plan section:** **Dependency policy**.
- **Exact correction:** Require the root manifest to include `resolver = "3"` (with the selected member edition, e.g. 2024) alongside `members` and `default-members`. This keeps root `cargo check`/`cargo test` selection deterministic and avoids silently using an unintended resolver.

### 8. `[MEDIUM]` `StatusResponse` does not specify the payload needed for the required identity and reason claims

- **Evidence:** `BOOTSTRAP_PLAN.md:110-117` only says that `HerdrCompatibility` has tagged `compatible`, `incompatible`, and `unavailable` variants and that `StatusResponse` contains the compatibility value. Yet the behavior requires the test mode to expose an honest unavailable reason (`BOOTSTRAP_PLAN.md:15-17`, `235-241`), the frontend to label incompatible/unavailable states (`BOOTSTRAP_PLAN.md:171-179`), and normal status to report Herdr 0.8.2, protocol 20, and schema 1 (`BOOTSTRAP_PLAN.md:240-242`). Without variant fields, those values cannot be represented or asserted through the generated TypeScript contract.
- **Affected plan sections:** **Protocol**, **Core**, **Shared frontend**, and **Behavior smoke**.
- **Exact correction:** Define the serialized payload explicitly. For example, make `compatible` carry `version`, `protocol`, and `schema_version`; make `incompatible` carry the observed identity plus a stable reason/code; and make `unavailable` carry a stable reason/code and message (including the test-mode reason). Generate those exact fields into TypeScript and require the CLI/HTTP/Tauri/frontend checks to assert them.

### 9. `[MEDIUM]` “Configurable endpoint” is not represented by the bootstrap CLI or a documented environment bridge

- **Evidence:** The plan promises a configurable Herdr endpoint (`BOOTSTRAP_PLAN.md:14`) and the adapter invokes only fixed CLI forms (`BOOTSTRAP_PLAN.md:135-144`), while the CLI surface exposes only `--herdr <executable>` and no socket/session option (`BOOTSTRAP_PLAN.md:148-153`). Herdr's documented resolution order includes `HERDR_SOCKET_PATH` and `HERDR_SESSION`/`--session`; the research explicitly says the endpoint and session selection must remain configurable (`research/herdr-0.8.2-bootstrap.md:42-51`).
- **Affected plan sections:** **Decisions**, **Herdr adapter**, and **CLI and gateway**.
- **Exact correction:** Add explicit Cockpit configuration for the endpoint and selected session, or document the exact inherited environment mapping. A minimal status-phase contract is `--herdr-socket <path>` and `--herdr-session <name>`, translated by the adapter to `HERDR_SOCKET_PATH` and the documented session selection for each child CLI invocation. Do not claim endpoint configurability while only the executable path can be changed.

### 10. `[MEDIUM]` The ephemeral-port smoke step has no stable address-output contract

- **Evidence:** `BOOTSTRAP_PLAN.md:152` permits `--bind 127.0.0.1:PORT`; the behavior smoke then requires `127.0.0.1:0` and says to “read the emitted bound address” (`BOOTSTRAP_PLAN.md:235-243`). Neither the CLI section nor server behavior defines an output format, stream, or machine-readable mode for that address.
- **Affected plan sections:** **CLI and gateway** and **Behavior smoke**.
- **Exact correction:** Define a stable startup line or JSON event emitted only after binding, for example `listening http://127.0.0.1:<actual-port>`, and require the smoke harness to parse that line before making the request. Keep startup failure on static-root errors and non-loopback addresses on stderr/nonzero exit so the harness can distinguish them.

### 11. `[MEDIUM]` The native smoke claim is stronger than the listed command

- **Evidence:** The plan correctly says static inspection is not a native smoke test (`BOOTSTRAP_PLAN.md:181-192`) and later requires launching the actual app, invoking `cockpit_status`, and rendering the real Herdr result (`BOOTSTRAP_PLAN.md:246-255`). However, the only listed command is the malformed `bun run --cwd frontend tauri:dev`, with no explicit assertion that the UI used Tauri IPC rather than the browser adapter and no stated observation of the command result.
- **Affected plan section:** **Native verification blocker**.
- **Exact correction:** After correcting the launch command/topology, specify the native smoke actions: launch the Tauri app, exercise the initial status screen, verify the rendered identity/mode came from `cockpit_status`, and record failure if the app instead requests `/api/v1/status` or shows an unavailable/blank state. Keep this as a manual/interactive native smoke because browser/HTTP success cannot prove Tauri IPC wiring.

## Non-findings / observed workstation constraints

- The missing Fedora WebKitGTK/GTK3/libsoup3/XDo/Ayatana AppIndicator/librsvg development packages are a real native-build blocker, but the plan already records that blocker and correctly does not claim native checks are green (`research/local-toolchain-bootstrap.md:37-53`, `BOOTSTRAP_PLAN.md:246-255`).
- Herdr 0.8.2, protocol 20, schema 1, and the two read-only CLI calls are observed and implementable (`research/herdr-0.8.2-bootstrap.md:5-30`, `research/local-toolchain-bootstrap.md:31-35`). No full mirror behavior is required to correct the findings above.
- Cargo `default-members` is a sound way to keep ordinary root Rust checks off the WebKitGTK-dependent Tauri member; the explicit native check must nevertheless select the Tauri manifest/package by a defined name or manifest path (`BOOTSTRAP_PLAN.md:78`, `218-231`).
