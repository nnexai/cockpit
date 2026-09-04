# Bootstrap plan spec review

Reviewed `BOOTSTRAP_PLAN.md` against `CONTEXT.md`, `DECISIONS.md`, and `research/bootstrap-coverage-matrix.md`, with the Herdr and Tauri/toolchain research used for feasibility and command checks. The intended boundary is respected: this review does not require the Herdr session mirror, terminal attachment, WebSocket routes, or provider implementation in this bootstrap. Those are valid next-phase deferrals when the plan labels them as such.

## Blocking findings

### 1. Tauri custom-command restriction is conditional, and the capability file is not explicitly selected

- **Severity:** Blocker (security and boundary correctness)
- **Evidence:** `BOOTSTRAP_PLAN.md` §Tauri host says to use the Tauri command-manifest restriction “where supported by the pinned Tauri release” (§Tauri host, line 189), but the plan never requires `app.security.capabilities` to select `capabilities/default.json`. The Tauri research requires both controls: explicit capability selection and `tauri_build::AppManifest::commands` (`research/tauri-frontend-bootstrap.md`, lines 79–91). Coverage item C-10 likewise says a capability file alone is not a custom-command allowlist (`research/bootstrap-coverage-matrix.md`, C-10).
- **Affected plan section:** `## Tauri host`; secondarily `## Repository tree` and `## Verification runnable now`.
- **Exact correction:** Make the command allowlist mandatory, not conditional, for the pinned Tauri release; allow only `cockpit_status` in `AppManifest::commands`. Require `tauri.conf.json` to explicitly select the named capability file (for example `app.security.capabilities: ["default"]`), and add a static configuration check that fails if either control is absent or broader permissions are present.

### 2. The gateway’s required local-auth behavior is unspecified

- **Severity:** Blocker (confirmed decision is not represented)
- **Evidence:** `DECISIONS.md` §Shared core and package boundaries requires that the loopback `cockpit serve` gateway “uses local authentication” (line 37). `BOOTSTRAP_PLAN.md` §CLI and gateway specifies loopback enforcement and rejects remote access, but says nothing about authentication (§CLI and gateway, lines 146–165). The coverage matrix explicitly warns that local-auth behavior must follow the confirmed decision where implemented (C-09, line 17).
- **Affected plan section:** `## CLI and gateway`; also `## Behavior smoke`.
- **Exact correction:** State the bootstrap authentication contract explicitly: define the local-auth mechanism and require the status HTTP test to exercise it, or explicitly mark local authentication as a next-phase deferral and remove any implication that the bootstrap gateway satisfies the full C-09 gateway contract. Do not silently treat loopback binding alone as authentication.

### 3. The Herdr status subprocess command is not pinned to an authoritative 0.8.2 form

- **Severity:** Blocker (the promised real status path may not execute)
- **Evidence:** `BOOTSTRAP_PLAN.md` §Herdr adapter hard-codes `<configured-herdr> status --json` (§Herdr adapter, lines 135–144), and the verification section repeats `cockpit status --json` (§Verification runnable now, lines 218–241). The repository evidence is inconsistent: `research/herdr-0.8.2-bootstrap.md` records `herdr status server` as the observed command (lines 7–21), while `research/local-toolchain-bootstrap.md` records `herdr status --json` (lines 32–34). The official 0.8.2 CLI reference lists `herdr status`, `herdr status server`, and `herdr status client`, but does not list `status --json` in its status command syntax. This is unresolved command ambiguity, not evidence that either local observation should be ignored.
- **Affected plan section:** `## Herdr adapter`; `## Verification runnable now`; `## Behavior smoke` step 5.
- **Exact correction:** Choose and document one command form confirmed against the targeted Herdr 0.8.2 CLI (or use the documented `ping` socket operation for machine-readable identity), define its expected output/exit behavior, and fixture-test that exact invocation. Align the `cockpit status` smoke command with that same authoritative path; do not leave `status --json` as an unverified assumption.

### 4. The reproducible native command has no declared project-local Tauri CLI dependency

- **Severity:** High (native bootstrap command is incomplete)
- **Evidence:** `BOOTSTRAP_PLAN.md` §Shared frontend lists `@tauri-apps/api` but does not list `@tauri-apps/cli` among frontend development dependencies (§Shared frontend, lines 167–180). The native blocker nevertheless instructs `bun run --cwd frontend tauri:dev` (§Native verification blocker, lines 246–255). The toolchain research explicitly rejects reliance on a global `cargo-tauri` and recommends a project-local `@tauri-apps/cli` invoked through Bun (`research/local-toolchain-bootstrap.md`, lines 55–63 and 94–99; `research/tauri-frontend-bootstrap.md`, lines 64–77).
- **Affected plan section:** `## Shared frontend`; `## Tauri host`; `## Native verification blocker`.
- **Exact correction:** Add a pinned `@tauri-apps/cli` dev dependency to the frontend package, define `tauri:dev` (and, if retained, `tauri:build`) to invoke that project-local binary, and require the dependency in `bun.lock`. Keep the native commands explicitly blocked by missing Fedora libraries, but make the command itself reproducible once those libraries are provisioned.

### 5. “Configurable endpoint” is asserted without a configuration path or acceptance test

- **Severity:** High (configuration claim is unimplementable as written)
- **Evidence:** `BOOTSTRAP_PLAN.md` says the Herdr endpoint and executable remain configurable (§Decisions, lines 14–15), but the CLI surface exposes only `--herdr <executable>` (§CLI and gateway, lines 148–153). The Herdr adapter is limited to fixed executable arguments (§Herdr adapter, lines 135–142); no socket endpoint, session selector, config-file field, or environment forwarding is named. Herdr research requires a configurable endpoint/session resolution and explicitly says not to hard-code the observed socket (`research/herdr-0.8.2-bootstrap.md`, lines 42–51). For the status-only phase, session mirroring may remain deferred, but the plan cannot simultaneously promise endpoint configurability and provide no input contract.
- **Affected plan section:** `## Decisions`; `## CLI and gateway`; `## Dependency policy`.
- **Exact correction:** Define the accepted endpoint/session inputs and precedence (for example explicit options/config values and the documented `HERDR_SOCKET_PATH`/`HERDR_SESSION` environment inputs), and test that the selected value reaches the status inspection. If endpoint/session selection is intentionally deferred with the mirror, remove the broad configurability claim from this phase and name it as a next-phase acceptance item instead.

### 6. Deterministic test mode is specified, but the verification does not prove that Herdr is never invoked

- **Severity:** High (explicit deterministic-mode contract is not defended)
- **Evidence:** `BOOTSTRAP_PLAN.md` promises that `--test-mode` disables the live Herdr probe (§Decisions, line 17), and `CockpitService` “never invokes Herdr” in test mode (§Core, lines 121–133). The smoke only checks returned fields and the unavailable reason (§Behavior smoke, lines 235–240); it does not assert that the inspector/executable was not called. A test could accidentally launch Herdr and still produce the same response.
- **Affected plan section:** `## Core`; `## CLI and gateway`; `## Verification runnable now`; `## Behavior smoke`.
- **Exact correction:** Add a deterministic no-call acceptance: exercise the service with an injected inspector whose invocation is observable and assert zero calls in test mode, and/or run `serve --test-mode` with an invalid/nonexistent Herdr executable and assert successful status output with `mode: test` and the stable disabled reason. Keep the test free of fabricated sessions, panes, terminals, or agents.

## Recommendations

### 7. The plan’s native IPC wording overstates what this phase can verify

- **Severity:** Recommendation (verification honesty)
- **Evidence:** The opening decision says the one bootstrap operation reports through “the CLI, loopback HTTP, Tauri IPC, and shared frontend” (§Decisions, lines 14–17). The same plan correctly states that Tauri compilation and native launch are blocked until Fedora WebKitGTK/GTK dependencies are provisioned (§Native verification blocker, lines 246–255), and the runnable-now commands contain no native compile or launch. Coverage C-21 forbids claiming native parity from browser-only checks (`research/bootstrap-coverage-matrix.md`, C-21 and the unsupported-verification trap at lines 37–39).
- **Affected plan section:** `## Decisions`; `## Tauri host`; `## Verification runnable now`; `## Native verification blocker`.
- **Exact correction:** Reword the phase claim to distinguish “implemented transport path” from “verified transport path”: CLI, loopback HTTP, and shared browser frontend are runnable-now evidence; Tauri IPC is declared and remains unverified/blocked until native provisioning and the required actual-app smoke. Retain the explicit next-phase/native provisioning boundary rather than claiming browser results establish native parity.

### 8. Frontend package/output layout contradicts the adopted frontend research and creates avoidable path drift

- **Severity:** Recommendation (maintainability and configuration consistency)
- **Evidence:** The plan puts the frontend package under `frontend/`, builds `frontend/dist`, and sets Tauri `frontendDist` to `../frontend/dist` (§Repository tree, lines 53–68; §Tauri host, line 190; verification commands use `--cwd frontend`). The adopted Tauri frontend research proposes one frontend package at repository root with `dist/` and `frontendDist: "../dist"` (`research/tauri-frontend-bootstrap.md`, lines 3–7, 45–62, and 131–135). Either layout is technically possible, but the plan does not record an intentional deviation, leaving the gateway static root and Tauri path vulnerable to divergence.
- **Affected plan section:** `## Repository tree`; `## Tauri host`; `## Verification runnable now`.
- **Exact correction:** Select one canonical layout. Prefer aligning the plan with the adopted research (`package.json`, Vite config, and source at repository root; `dist/`; `frontendDist: "../dist"`), or explicitly document the intentional `frontend/` subpackage and update the research-derived path invariant and a static check that both consumers point to that one output.

### 9. Several promised negative/static-file contracts lack named verification evidence

- **Severity:** Recommendation (acceptance completeness)
- **Evidence:** The plan promises a specific startup/server error for missing static assets and SPA fallback behavior (§CLI and gateway, lines 160–165), and promises generated TypeScript drift detection (§Protocol, lines 106–119). The tree names `crates/cockpit-host/tests/server.rs` but gives no test case for missing assets, and names no protocol test file or explicit acceptance entry for the generated-file comparison. The command list only invokes the comparison binary and generic `cargo test` (§Verification runnable now, lines 218–233).
- **Affected plan section:** `## Protocol`; `## CLI and gateway`; `## Parallel implementation ownership`; `## Verification runnable now`.
- **Exact correction:** Add named acceptance cases/owners: a server test must exercise missing static root/assets and SPA fallback, and a protocol test (or explicit exporter check owned by the protocol slice) must fail on generated TypeScript drift. Keep these as status/static-host contracts; do not add mirror, terminal, or provider scope.

## Accepted deferrals and conforming areas

- The status-only phase and explicit next-phase Herdr mirror boundary are coherent (§Decisions, line 15; opening scope, lines 1–5, and §Explicit non-goals, lines 257–265). The matrix’s snapshot/event/terminal requirements apply once those operations are claimed; they are not blockers for a status-only implementation.
- The plan correctly omits `cockpit-providers` until a real provider exists (§Decisions, line 11; §Explicit non-goals, lines 261–263), matching coverage C-17.
- Rust is the protocol source and TypeScript is generated/checked rather than hand-maintained (§Decisions, line 12; §Protocol, lines 110–119), matching coverage C-02.
- The four reusable crates plus the Tauri host have concrete status-path roles, and the dependency direction keeps transport out of `cockpit-core` (§Repository tree, lines 20–104). No duplicate gateway or Cockpit-owned Herdr session/PTY authority is introduced.
- Loopback-only binding, foreground `serve`, no raw shell/socket forwarding, honest unavailable state, and no fake UI entities are stated clearly (§Decisions, lines 15–18; §CLI and gateway, lines 146–165; §Shared frontend, lines 167–179).
