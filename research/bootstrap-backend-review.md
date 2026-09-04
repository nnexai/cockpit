# Bootstrap backend review

Reviewed `BOOTSTRAP_PLAN.md`, `CONTEXT.md`, `DECISIONS.md`, `Cargo.toml`, and the implemented protocol/core/Herdr/host paths. Review is limited to the bootstrap status path; it does not cover deferred Herdr mirror, terminal, provider, or workspace features.

## Findings (ordered by severity)

### 1. High — `CockpitService::status` can deadlock and blocks the Tokio runtime

- **Evidence:** `crates/cockpit-core/src/lib.rs:8-17` implements `block_on` with a `Waker::noop()` and repeatedly polls pending futures. `crates/cockpit-core/src/lib.rs:62-78` calls it from the synchronous public status operation. The Herdr implementation performs blocking `std::process::Command::output()` in `crates/cockpit-herdr/src/cli.rs:115-136`. The HTTP handler calls the synchronous operation directly at `crates/cockpit-host/src/server.rs:127-129`, and the Tauri command does the same at `src-tauri/src/lib.rs:8-12`.
- **Failure:** Any inspector future that awaits Tokio I/O, a timer, or another runtime-driven operation can remain pending forever because the ad-hoc executor never drives the runtime or honors wakeups. Even with the current inspector, a hung Herdr process blocks an Axum worker (and the synchronous Tauri command) indefinitely, allowing one status request to starve the gateway. This conflicts with the plan's asynchronous adapter boundary and makes the shared status seam unsafe for the hosts.
- **Smallest correction:** Make the core status entry point asynchronous and await it in the CLI, HTTP handler, and Tauri command. Remove the custom `block_on`; execute the fixed Herdr subprocess through Tokio's process API or explicitly isolate `std::process::Command` in `spawn_blocking` (with a bounded/cancellable operation if the adapter defines one). Add a test inspector whose future is genuinely pending before completion so this cannot regress.

### 2. High — schema compatibility checks accept unrelated nested data as authoritative

- **Evidence:** `crates/cockpit-herdr/src/schema.rs:3-15` recursively searches every object/array for a matching string key, `:18-39` does the same for numeric fields, and `:42-50` treats any matching string anywhere as a required method. `crates/cockpit-herdr/src/cli.rs:154-205` uses these helpers for status identity, schema version, and method gating.
- **Failure:** A malformed or changed Herdr response such as `{"details":{"version":"0.8.2","protocol":20},"notes":{"schema_version":1,"text":"session.snapshot"}}` can be accepted despite omitting the required top-level identity/schema fields and method declarations. A method name in a description, fixture metadata, or another schema can likewise satisfy the capability gate. The plan requires lenient handling of unknown fields, not accepting unknown locations as compatibility evidence; this can report an incompatible server as compatible and later fail when a required operation is used.
- **Smallest correction:** Parse the documented identity fields at their expected status/schema locations and inspect only the schema's request-method declarations (the fixture's `schemas.request` method constants) for required methods. Continue ignoring unknown sibling fields. Add negative fixtures with nested-only identity and method strings in descriptions.

### 3. Medium — early incompatibility responses fabricate `schema_version: 0`

- **Evidence:** `crates/cockpit-herdr/src/cli.rs:161-165` constructs an identity with `schema_version: 0` before running `api schema --json`; `:167-180` returns that identity for version/protocol mismatches without ever obtaining a schema version. The protocol makes incompatible identity optional (`crates/cockpit-protocol/src/v1.rs:27-30`).
- **Failure:** A client receives an `incompatible` identity that claims schema version `0`, although the adapter has no schema observation. `0` is indistinguishable on the wire from an observed version and can lead consumers to treat it as meaningful identity data. This also undermines the optional identity field intended for cases where identity is incomplete.
- **Smallest correction:** Return `identity: None` for version/protocol mismatches until all identity fields have been observed, or introduce an explicitly partial identity type if partial reporting is required. Add serialization assertions for each early-mismatch response.

### 4. Medium — percent-encoded asset extensions bypass the precise SPA fallback rule

- **Evidence:** `crates/cockpit-host/src/server.rs:141-150` classifies fallback requests by calling `Path::new(path).extension()` on the raw URI path. `tower-http`'s `ServeDir` decodes percent escapes before constructing its filesystem path, so a missing request such as `/missing%2Ejs` reaches this fallback with a logical `.js` extension but with the raw string `missing%2Ejs`.
- **Failure:** The raw path has no extension, so the fallback serves `index.html` with status 200 instead of preserving the required missing-asset 404. The same raw-path classification can let encoded `/api/...` forms fall through to the SPA document rather than the structured API 404.
- **Smallest correction:** Percent-decode the URI path (rejecting invalid encodings) before API-prefix and extension classification, using the same path interpretation as the static service. Add requests for encoded extensions and encoded API separators to the host routing tests.

### 5. Medium — TypeScript exporter tests do not defend check-mode drift behavior

- **Evidence:** `crates/cockpit-protocol/tests/typescript.rs:46-50` only compares `render_v1()` to itself and checks that a substring exists. The actual byte comparison is in `crates/cockpit-protocol/src/typescript.rs:79-85`, and the `--check` command contract is implemented at `crates/cockpit-protocol/src/bin/export-typescript.rs:37-49`.
- **Failure:** The test suite would pass if `check` always returned `true`, if stale bytes were accepted, or if check mode wrote the target. This leaves the plan's generated-contract drift gate effectively untested.
- **Smallest correction:** Add a temp-file test that writes exact `render_v1()` bytes and asserts `check == true`, changes one byte and asserts `false` with no rewrite, and checks a missing file returns `false`. Exercise the binary's malformed-argument and check-mode exit behavior if the CLI contract is part of the acceptance.

### 6. Medium — Herdr fixed-command and endpoint propagation are not tested

- **Evidence:** `crates/cockpit-herdr/src/cli.rs:103-113` constructs the command and translates session/socket settings, but `crates/cockpit-herdr/tests/compatibility.rs:99-123` only tests resolved config values and conflict detection. The fixture script at `:25-36` branches on a loose `"$*"` pattern and does not record argv or `HERDR_SOCKET_PATH`.
- **Failure:** A regression in argument ordering, command spelling, socket environment propagation, or inherited environment behavior could still pass every current compatibility test. Those details are explicit bootstrap requirements (`BOOTSTRAP_PLAN.md:149-160`) and are security/boundary-sensitive because the adapter must not invoke arbitrary commands or rely on implicit endpoint precedence.
- **Smallest correction:** Use a recording fixture executable that logs each argv vector and the socket environment for both status and schema calls. Assert the exact `status server --json` and `api schema --json` invocations, global `--session` placement, explicit socket translation, and inheritance when no explicit override is supplied.

### 7. Low — host tests do not prove `serve` startup/output contracts

- **Evidence:** `crates/cockpit-host/tests/server.rs:60-90` constructs a listener and calls Axum `serve` directly, while `:92-105` tests only the validation helpers. No test invokes `cockpit_host::server::serve`, captures the exact `listening http://<ip>:<actual-port>` line, or proves non-loopback rejection occurs before listener creation and missing static assets fail startup with the required error.
- **Failure:** The router tests can pass while the foreground binary emits a changed/malformed discovery line, binds before rejecting a remote address, or changes startup validation order. These are explicit gateway requirements in `BOOTSTRAP_PLAN.md:175-188` and are needed by the ephemeral-port smoke harness.
- **Smallest correction:** Add a subprocess-level CLI test (or an equivalent controllable `serve` harness) that exercises an ephemeral loopback bind and asserts the exact single startup line, and separately exercises non-loopback and missing-root/index startup failures with nonzero results and stderr diagnostics. Keep the existing real HTTP route assertions.

## Non-findings

The crate dependency direction is consistent with the plan: protocol has no transport dependencies, core depends only on protocol plus its async trait support, Herdr depends on core/protocol, and host owns Axum/Tokio/static serving. No implementation finding was identified in the deferred Herdr mirror, terminal, provider, or workspace features because those paths are out of scope for this bootstrap review.
