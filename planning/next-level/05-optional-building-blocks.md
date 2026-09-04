# Optional and deferred building blocks

Status: proposed blueprint; implementation is deferred and requires the decisions listed in each story.
Date: 2026-09-04

This document isolates capabilities that are useful after the core lifecycle, local context, and read-only source loop. Each story is independently composable. None changes Herdr-server, creates a second Herdr authority, enables remote writes, or relaxes the current local-first boundary. The current product deliberately defers broad distribution, remote access, credential management, history, and extra provider matrices ([`CONTEXT.md`](../../CONTEXT.md), [`DECISIONS.md`](../../DECISIONS.md)).

## Shared rules

Optional work must consume the existing versioned protocol and capability gates. Herdr remains authoritative for sessions, workspaces, panes, PTYs, focus, agent state, and inbox ordering; Cockpit may own caches and presentation projections only ([UI constraints](../../research/ui-implementation-constraints.md#forbidden-shortcuts)). Provider adapters expose independent read capabilities and explicit unsupported errors. The current source plan’s immutable cache, revision/hash freshness, bounded hydration, and token-redaction rules apply to every provider extension ([context feasibility](../../research/next-level-context-feasibility.md)).

The current local entry rule remains: an artifact URL can identify a target, but the configured local primary repository is required before context placement or repository-relative expansion. Arbitrary URL downloads, remote repository discovery, and remote cloning are parked in OPT-09. Local reference snapshots continue to use reflink then verified copy; hardlinks and Git alternates remain forbidden.

## Provider expansion

### OPT-01: GitHub provider capabilities

**Dependencies:** SRC-01 provider contracts, SRC-03 hydration/freshness, CTX-01 manifest; independent of Gitea/Tea implementation. Requires an explicit GitHub adapter and credential policy decision.

**Proposed modules/interfaces:** `crates/cockpit-core/src/sources/github.rs`; provider fixture set under `crates/cockpit-core/tests/fixtures/github/`; optional host wrapper only if a documented local CLI is selected. Implement independent `IssueProvider`, `ReviewProvider`, `WikiProvider` capability traits rather than a GitHub-shaped universal interface. Do not assume GitHub wiki, review-thread, or timeline semantics match Gitea; each capability must be probed and normalized into the canonical source envelope.

**Steps:** resolve artifact owner/repository against the local primary identity; probe adapter version/API capabilities; fetch issue/PR metadata, comments/timeline, changed files/diffs, reviews/replies, and wiki only where the adapter declares support; preserve provider IDs, revision, timestamps, anchors, and canonical URL; apply SRC-03 budgets; cache and materialize generated assets.

**Verification:** fixture tests for pagination, missing scopes, deleted/renamed files, review threads, rate limits, revision changes, and redacted auth errors. Real acceptance against a disposable or explicitly authorized repository verifies read-only requests, source hashes, local-primary mismatch rejection, and bounded calls. No mutation endpoint belongs in the adapter.

**Boundaries/deferrability:** no GitHub-specific UI or webhook synchronization. Wiki and review-thread subcapabilities may ship separately. Required decisions: REST versus GraphQL or an existing local CLI, supported API versions, scope minimum, rate-limit behavior, and whether private-repository support is in scope.

### OPT-02: GitLab provider capabilities

**Dependencies:** SRC-01/SRC-03 and the same local identity/credential policy as OPT-01; independent of GitHub and Gitea.

**Proposed modules/interfaces:** `crates/cockpit-core/src/sources/gitlab.rs`; fixtures for issues, merge requests, discussions, diffs, and wiki. Keep “merge request” as provider metadata while normalizing to the review artifact contract. Implement only capabilities confirmed by the selected GitLab API/client; do not infer support from endpoint naming.

**Steps:** map issue/MR/discussion/wiki records to canonical IDs and revision metadata; preserve discussion position and file-side anchors where supplied; apply bounded pagination and byte limits; use immutable cache and generated-copy conflict rules.

**Verification:** fixture and contract tests for permission-scoped discussions, pagination, rebased positions, deleted files, wiki revisions, rate limits, and API error mapping. Real acceptance must inspect server audit state to prove no writes and test a private project only after credential scope approval.

**Boundaries/deferrability:** no provider-specific remote clone, pipeline mutation, or webhook daemon. MR discussions and wiki are independently deferrable. Required decisions: supported GitLab editions/versions, REST versus GraphQL, minimum token scopes, and position-rebase semantics.

### OPT-03: provider expansion registry and conformance suite

**Dependencies:** OPT-01/02 or another concrete adapter plus SRC-01.

**Proposed modules/interfaces:** `crates/cockpit-core/src/sources/registry.rs`; provider conformance fixtures and a test harness that runs the same capability matrix against each adapter. Registry entries declare provider name, supported artifact forms, capabilities, freshness strategy, limits, and auth mechanism without exposing secret values.

**Steps:** require adapters to pass canonical envelope, identity, pagination, bounded failure, freshness, and read-only conformance checks; report unsupported capability per provider rather than hiding it behind generic errors. Keep provider additions isolated from Context UI and local snapshot code.

**Verification:** fixture-driven matrix plus one real read-only smoke per enabled provider. Acceptance fails if a provider mutates or emits an unbounded request.

**Deferrability:** registry and conformance are useful after the second provider; until then direct capability structs are sufficient. Required decisions: minimum conformance level and whether provider adapters are built in or dynamically configured.

## Configuration and settings

### OPT-04: settings surface

**Dependencies:** stable config schema and core contracts; no dependency on provider expansion. The current decision is config file plus environment/one-off CLI overrides, with no settings UI ([`CONTEXT.md` §5.6](../../CONTEXT.md)).

**Proposed modules/interfaces:** `crates/cockpit-core/src/config.rs` owns typed, validated settings; `crates/cockpit-host/src/config.rs` owns file/env/CLI precedence; later `src/app/Settings.tsx` is presentation only. Settings include roots, Herdr endpoint/session, preview/search/watch limits, provider profile names, and feature flags. Secret material is referenced by an external credential mechanism, never serialized into snapshots or frontend state.

**Steps:** version the config schema; validate canonical roots and finite limits at load; show effective non-secret configuration and source of each override; support export/import only for redacted settings; reject remote bind/TLS settings until OPT-10 is implemented.

**Verification:** precedence tests, malformed/unknown key behavior, path containment, limit validation, redaction, restart persistence, and browser/native parity. Real acceptance edits the config file, restarts `cockpit serve`, and verifies effective values without exposing tokens.

**Boundaries/deferrability:** no settings UI is required for the current proof of concept; config-file support remains the source of truth. Required decisions: schema migration policy, writable config location, profile naming, and which limits are user-adjustable versus fixed safety ceilings.

## Agent state extensions

### OPT-05: agent history and resume

**Dependencies:** a documented Herdr history/resume capability or an explicitly separate Cockpit-owned transcript source; current snapshot/event identity rules. It must not depend on a guessed Herdr field or duplicate live state.

**Proposed modules/interfaces:** `crates/cockpit-core/src/agents/history.rs` defines `HistoryProvider`, `HistoryEntry`, `ResumeCapability`, and stale/session identity checks; `crates/cockpit-protocol/src/v1.rs` adds read-only history responses and an explicit resume request only after authority is confirmed. Herdr adapter code may implement the interface only from installed schema/documented operations.

**Steps:** discover whether Herdr exposes durable history and a resume operation; if absent, return unsupported and keep live snapshot behavior unchanged. If supported, key entries by Herdr session/workspace/pane/agent IDs, paginate and bound content, verify authority before resume, and resnapshot after any operation. Treat transcript content as untrusted text and never infer process state from it.

**Verification:** captured schema fixtures for supported/unsupported servers, stale ID/session rejection, pagination/limits, reconnect/resnapshot, and a real disposable-session resume smoke only after explicit capability evidence. Never test by resuming a user’s live agent.

**Boundaries/deferrability:** history storage, summarization, search indexing, and automatic resume are separate. No local Cockpit registry of agents or process lifetimes. Required decisions: Herdr authority/source, retention and privacy, resume confirmation UX, and behavior when the original pane/workspace is gone.

### OPT-06: inbox views and actions

**Dependencies:** Herdr authoritative inbox metadata/order and the installed/documented plugin or socket capability. Reviewr comment storage does not establish inbox semantics. Inspect the installed inbox plugin and Herdr schema independently before adding inbox actions.

**Proposed modules/interfaces:** `crates/cockpit-core/src/inbox.rs` defines read projection types and capability-gated actions; `crates/cockpit-herdr` maps only documented Herdr/plugin operations; `crates/cockpit-protocol/src/v1.rs` carries IDs, ordering, and explicit action results. Actions must target authoritative session/pane IDs and confirm current writable/control state.

**Steps:** mirror inbox ordering from Herdr snapshot/events; preserve plugin-provided labels as display metadata; implement view/filter/mark-read only when supported; keep review draft collection separate under REF-01 durable Cockpit-owned storage; resnapshot after actions.

**Verification:** fixture ordering/gap/reconnect tests, stale-session rejection, disappearing-pane behavior, unsupported-action errors, and real read-only inbox smoke. For any send/paste action, use a dedicated proven paste-capable operation and retain drafts on ambiguous outcomes as the review research requires; never auto-submit.

**Boundaries/deferrability:** no Cockpit-owned inbox registry, autonomous notifications, or provider-side comment writes. Advanced popup/history remains deferrable. Required decisions: exact Herdr action authority, read/unread semantics, draft retention, and whether plugin actions are supported in the public contract.

### OPT-07: automatic agent/bootstrap integration (explicit opt-in)

**Dependencies:** documented Herdr setup/bootstrap contract, explicit user opt-in, and credential policy. Current behavior is manual: Cockpit does not automatically launch/configure OMP or agents ([`CONTEXT.md` §6](../../CONTEXT.md)).

**Proposed modules/interfaces:** `crates/cockpit-core/src/automation.rs` with `AutomationPolicy {enabled, allowed_commands, working_roots, timeout}` and an explicit task-level operation; `crates/cockpit-herdr` invokes only documented Herdr setup APIs/CLI forms. No arbitrary shell command input is exposed to the UI.

**Steps:** require a durable opt-in flag and per-operation confirmation for first activation; validate executable identity/version and root; create environment only from sanitized non-secret values; record operation result and rollback/disable state; leave normal startup and manual agent launch unchanged when disabled.

**Verification:** default-off tests, command allowlist/timeout, cancellation, partial failure, secret redaction, and disposable Herdr smoke. Real acceptance must prove no setup occurs on ordinary workspace creation and that opt-in activation is visible in audit output.

**Boundaries/deferrability:** no autonomous background agents, scheduled execution, or credential provisioning. Required decisions: supported agent/bootstrap commands, trust model, confirmation frequency, and failure rollback.

## Credentials and trust

### OPT-08: credential store, key-store, and passkey separation

**Dependencies:** security review and chosen platform storage mechanism; no provider implementation should block on this story if it can consume externally supplied local credentials.

**Proposed modules/interfaces:** `crates/cockpit-core/src/credentials.rs` defines opaque `CredentialRef`, provider scope, redacted status, and capability errors; `crates/cockpit-host` supplies OS key-store integration. A future passkey module is an authentication ceremony/identity mechanism, not a general secret file or provider token store.

**Steps:** define credential references and process-bound injection without persisting tokens in manifests, logs, URLs, snapshots, or frontend IPC; support unavailable/locked-store states; keep provider adapters unaware of raw storage details; add explicit revoke/forget semantics only after ownership is clear.

**Verification:** secret-redaction tests across errors/logs/cache/export, locked/unavailable store, wrong provider scope, process environment inspection, and platform-specific smoke. Do not use real production credentials in tests.

**Boundaries/deferrability:** no passkey login, sync, or multi-user identity system is implied by adding a key-store. Current wrapper/tool credential handling remains authoritative. Required decisions: supported OS stores, threat model, process lifetime, rotation/revocation, and whether passkeys authenticate Cockpit or a remote service.

## Remote and distribution slices

### OPT-09: remote artifact/repository acquisition (parked)

**Dependencies:** a new product/security decision, separate from artifact URL parsing and local primary resolution. It must not be pulled into SRC-03 hydration by convenience.

**Proposed future modules/interfaces:** `crates/cockpit-core/src/acquisition.rs` with explicit `RemoteSourcePolicy`, allowlisted hosts, destination quota, provenance, cancellation, and quarantine status. Remote clones/downloads would be a separately authorized operation with no implicit relationship to a URL pasted into the current flow.

**Steps when reopened:** define host allowlist and redirect policy; authenticate through OPT-08; enforce archive/repository size and path limits; verify provenance/revision; materialize into an isolated location; require explicit promotion to an already-discovered reference root. Never silently add the result to a workspace.

**Verification:** blocked-by-default tests, redirect/host/path traversal, quota, cancellation, provenance mismatch, and real acceptance only against an authorized disposable repository.

**Deferrability/boundary:** fully parked now. No arbitrary URL download or remote clone is implemented or assumed by any other story. Required decisions: threat model, supported protocols/forges, trust/promotion workflow, storage quotas, and cleanup ownership.

### OPT-10: remote browser, TLS, and multi-user authorization

**Dependencies:** a major deployment/product decision; current `cockpit serve` is loopback-only and has no separate browser authentication ([`CONTEXT.md` §3.2](../../CONTEXT.md)).

**Proposed modules/interfaces:** `crates/cockpit-host/src/remote.rs` for bind/TLS policy; `crates/cockpit-core/src/authz.rs` for user/session/workspace authorization; protocol requests carry authenticated principal and resource scope. Keep local native/Tauri capability checks separate from remote auth.

**Steps:** define threat model and trust boundary; choose TLS certificate provisioning and identity provider; authenticate before WebSocket upgrade and every mutating/read-sensitive operation; authorize per Herdr session/workspace/context resource; audit access and disconnect/revoke; preserve loopback-safe default.

**Verification:** default-loopback regression, TLS certificate failure, origin/CSRF, token expiry/revocation, cross-user resource denial, WebSocket reconnect, and penetration/security review. Real acceptance requires two isolated identities and a disposable server.

**Boundaries/deferrability:** not a bind-address toggle. No remote exposure until auth, TLS, policy, and audit are complete. Required decisions: identity authority, public/private deployment, authorization granularity, secret/certificate lifecycle, and multi-user Herdr semantics.

### OPT-11: native packaging, update, and distribution

**Dependencies:** stable native protocol/client, Tauri build topology, platform dependency inventory, signing/update authority. Current architecture targets Linux Tauri v2, while distribution is a later concern ([`CONTEXT.md` §1/§3.2](../../CONTEXT.md)).

**Proposed modules/interfaces:** `src-tauri/` packaging configuration; repository-local release scripts and manifest metadata; optional updater adapter behind a signed-artifact contract. Keep application version, protocol compatibility, provider capability, and Herdr compatibility independently reported.

**Steps:** define supported Linux distributions/dependencies; pin reproducible build inputs; produce signed artifacts and checksums; test install/upgrade/rollback; ensure updater never changes Herdr server/config without opt-in; document manual browser-host distribution separately.

**Verification:** clean-machine install, upgrade, rollback, signature/checksum, offline start, missing native library, protocol incompatibility, and real native smoke. No “build succeeded” claim substitutes for installed-app execution.

**Boundaries/deferrability:** no auto-update or publication in the current proof of concept. Required decisions: release channels, signing key ownership, update authority, rollback retention, and support matrix.

### OPT-12: accessibility and release-quality hardening

**Dependencies:** stable UI surfaces and interaction semantics; accessibility is currently best effort, not a distribution gate ([`CONTEXT.md` §5.6](../../CONTEXT.md), [constraints](../../research/ui-implementation-constraints.md#minimal-accessibility-behaviors)).

**Proposed modules/interfaces:** frontend accessibility test fixtures and a release checklist, with no business logic in the UI. Preserve meaningful labels, keyboard reachability, visible focus, non-color-only state, and actionable inline errors.

**Steps:** audit session selector/tree/tabs/panes/context/errors; add keyboard and semantic focus paths; test terminal status without flooding assistive announcements; document known platform limitations; gate packaged release on agreed criteria only after OPT-11.

**Verification:** automated DOM/accessibility checks where meaningful, keyboard-only walkthrough, screen-reader smoke on the supported Linux stack, high-contrast/color-independent review, and real native/browser checks. Acceptance criteria must be written before making this a release gate.

**Boundaries/deferrability:** does not redesign Herdr semantics or make formal accessibility compliance claims. Required decisions: supported assistive technologies, severity threshold, release gate, and ownership of regressions.

## Search improvements without a custom engine

### OPT-13: bounded search quality improvements

**Dependencies:** CTX-01 list/read/search contract. No dependency on a new index or custom search engine.

**Proposed modules/interfaces:** `crates/cockpit-core/src/context/search.rs` extends the existing core-mediated `ripgrep` operation with structured filters, result cursors, cancellation, and explicit `truncated` reasons. Keep `ripgrep` as the execution engine and pass an argument vector against a fixed contained root; the feasibility research documents hidden/binary/ignore defaults and output limits ([bounded search](../../research/next-level-context-feasibility.md#bounded-search-watch-and-previews)).

**Steps:** add path/type/glob filters, stable result ordering, max result/output/file/time budgets, and optional cached metadata for repeated queries; never shell-evaluate query text or expose arbitrary commands. A future cache may accelerate listing but must not become an authority for file contents.

**Verification:** query/path injection, containment, binary/hidden/ignored behavior, cancellation, cursor expiry, truncation disclosure, and changed-file invalidation. Real acceptance compares results with direct `rg` on a disposable companion tree and checks no `.git` traversal.

**Boundaries/deferrability:** no custom parser/index/search database is required. Fuzzy ranking, persistent indexing, and cross-repository search are separately deferred and require measured need plus a resource budget decision.

## Sequencing and decision gate

OPT-01-03 can follow the first provider only when a second provider is justified. OPT-04 is a configuration/product convenience and should follow stable limits. OPT-05-07 require Herdr capability evidence before any code design becomes binding. OPT-08 and OPT-10-11 require security/release ownership and are major slices. OPT-09 stays parked. OPT-12 follows a defined distribution target. OPT-13 may be added incrementally without changing the search authority.

Before starting any optional story, record: authoritative state owner, capability evidence, secret/permission model, bounded resource limits, failure and rollback behavior, real acceptance fixture, and explicit non-goals. If those decisions are unavailable, retain the story as deferred with an unsupported capability response; do not approximate it through an unrelated Herdr method or provider endpoint.

## Priority for this personal application

These stories preserve a complete view of previously deferred ideas. None is a prerequisite merely because it would help distribute a product. Prioritize code tweakability, the local loop, and the separate CLEAN pass over provider breadth, multi-user deployment, release infrastructure, or a configurable extension framework. Full graphical Reviewr replacement is REV-01/02, not an inbox/history story; possible TUI backport is PANE-03.
