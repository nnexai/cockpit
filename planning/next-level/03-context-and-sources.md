# Context and sources implementation plan

Status: proposed implementation plan; no implementation is authorized by this document.
Date: 2026-09-04

This plan covers Cockpit-owned context storage, local repository snapshots, and read-only source ingestion. It assumes the existing Rust core/protocol/host boundaries and the Herdr authority described in [`CONTEXT.md`](../../CONTEXT.md) and [`DECISIONS.md`](../../DECISIONS.md). It does not add a UI selection model, change Herdr-server, clone a remote repository, or download an arbitrary URL.

## Invariants

* An artifact URL may identify the requested source, but provisioning fails until the configured local primary repository is resolved. The URL is never permission to discover or clone another repository.
* The local primary repository is the only implicit source root. Additional local reference repositories must already be discovered/configured and are copied into the companion context explicitly.
* Context is a Cockpit-owned companion resource associated with authoritative Herdr worktree/workspace identity. It is not a second Herdr workspace/session registry.
* Source ingestion is read-only. Existing issue comments, pull-request reviews/replies, wiki pages, and optional telemetry are collected; Cockpit does not post, edit, merge, resolve, or mutate the Herdr server.
* Reflink is attempted for local snapshots, then a verified normal copy. Hardlinks and Git alternates are forbidden for mutable snapshots. The feasibility evidence explains why: reflink is copy-on-write while hardlinks alias one inode, and Git warns that `--shared` alternates can become corrupt after source maintenance ([feasibility research](../../research/next-level-context-feasibility.md#local-repository-cow-semantics), [git-clone](https://git-scm.com/docs/git-clone.html), [FICLONE](https://man7.org/linux/man-pages/man2/ioctl_ficlone.2.html)).
* Every asset has stable identity, source metadata, content hash, bounded status, and a source-to-render mapping where applicable. Limits produce an explicit truncation/degraded state.

## Proposed resource and configuration model

### CTX-01: companion store ownership and manifest

**Dependencies:** FND-01/02/03 configuration, identity and operation contracts. Storage can be built against a provenance fixture independently of LIFE-02; real association is performed by LIFE-03. Depends on no provider implementation. Enables CTX-02 and SRC-01-06.

**Proposed files/contracts:**

* `crates/cockpit-core/src/context/mod.rs`: `ContextStore` trait, `ContextResourceId`, `ContextOwner`, `ContextManifest`, `ContextEntry`, `ContextStatus`, and bounded operation errors.
* `crates/cockpit-core/src/context/store.rs`: path policy, manifest persistence, atomic replacement, and ownership checks.
* `crates/cockpit-protocol/src/v1.rs`: versioned request/response types for `context.list`, `context.read`, `context.refresh`, and status events. Keep transport adapters thin.
* `crates/cockpit-host/src/config.rs` (or the established config module): proposed `context_root`, `primary_repository_root`, `additional_reference_roots`, preview/search/watch limits, and provider configuration. CLI/env overrides must be documented alongside config-file keys.
* `companion/context-manifest.json`: Cockpit-owned manifest format, schema version, owner workspace/worktree IDs, primary repository identity, entries, revisions, hashes, and failure records. The manifest is metadata, not a generated index shown as user content.

The resource owner is the Cockpit operation that created the companion directory; its association key must include authoritative Herdr workspace ID plus worktree checkout identity/path and a Cockpit resource ID. Store no credentials. A manifest entry should include `{logical_id, relative_path, kind, source, generated, revision, content_hash, bytes, copy_mode, status, updated_at}`. User files have `source = user`, `generated = false`, and are never overwritten by synchronization.

**Implementation steps:** validate configured roots as absolute canonical directories; create the companion directory and manifest atomically after successful primary worktree provisioning; register the association only after both are durable; enumerate entries through the manifest plus bounded directory scan; replace individual generated assets by temp-file plus rename; retain per-entry failure and retry state.

**Failures:** missing primary repository, root not a directory, path escape, manifest schema mismatch, permission/space failure, stale Herdr provenance, duplicate owner, or interrupted atomic write. Never silently adopt a pre-existing companion directory; report owner mismatch and require explicit recovery policy.

**Tests and real acceptance:** unit-test schema round trips, ownership mismatch, path containment, atomic replacement, and restart recovery. Integration-test create/refresh/destroy against a disposable companion tree and authoritative worktree fixture. Real acceptance requires a real local companion directory containing user-created files, generated source assets, an interrupted write simulation, and a restart that preserves ownership and user files.

**Parallel ownership:** one core/storage owner; protocol types may proceed in parallel after the DTO is reviewed. Host configuration changes depend on the core limits contract. UI work is downstream and is not part of this plan.

**Deferrable:** durable manifest version history and a repair command are later. Ownership, atomicity, and user/generated separation are required for the first slice.

### CTX-02: local repository snapshot

**Dependencies:** CTX-01 path policy and configured roots. No provider or UI dependency.

**Proposed files/contracts:** `crates/cockpit-core/src/context/snapshot.rs`, with `SnapshotRequest {source_root, destination, policy}` and `SnapshotResult {copy_mode, files, bytes, hash, fallback_reason}`. A filesystem adapter may live in `crates/cockpit-host` only if the core trait remains testable and transport-free.

**Exact include/exclude policy (proposed):**

* Include regular files beneath the already-resolved primary repository root and explicitly selected, already-discovered reference repository roots. Preserve relative paths and file bytes. Include tracked, untracked, modified, staged, and ignored files only when the caller explicitly selects that source mode; the default mode is `working_tree`: current regular-file bytes for tracked files plus untracked non-ignored files. Ignored untracked files are excluded. Tracked files remain included even if an ignore pattern now matches, subject to explicit safety/size exclusions. Record mode, exclusion policy version, HEAD, and dirty status in the manifest.
* Never copy `.git/` internals, Git object stores, alternates, locks, sockets, FIFOs, devices, native executable binaries, or symlinks. Executable-bit scripts and source files may be copied and viewed as inert text; do not execute them. A symlink is recorded as skipped with reason `symlink`; do not follow it even when it points inside the root. Submodules are recorded as a gitlink entry with commit/path metadata; their working trees are not traversed unless separately resolved as an already-discovered local repository and explicitly selected.
* Exclude configured VCS/build/cache directories (`.git`, `target`, `node_modules`, common build output) by default. An explicit include glob may add a regular file only within the root and under byte/file/count limits; it may not re-include `.git` or escape containment.
* Dirty state is captured as current filesystem bytes plus a source status record from local Git inspection. Ignored files are omitted by default and can be requested as a bounded explicit mode. No snapshot operation stages, cleans, resets, or modifies Git state.

Containment must use canonicalized parent checks plus a race-resistant open/read strategy where available; recheck file type and containment immediately before copy. Reject destination nested inside source and source/destination on unsupported cross-device paths for reflink, then use copy only when safe. Hash each destination and compare expected size/hash before manifest commit.

Generated provider assets and local repository snapshot files are separate namespaces. Synchronization may replace only entries with `generated = true` and matching source identity/revision. If a generated destination was edited by a user, detect hash divergence and record `sync_conflict`; preserve the user file and write a new versioned generated candidate or leave the asset stale according to policy. Never overwrite `source = user` entries.

**Failures:** unsupported reflink, cross-device destination, read race, file mutation during copy, permission error, limit exhaustion, symlink/type change, hash mismatch, destination collision, or partial cancellation. Reflink failure is a diagnostic fallback, not a correctness failure; all other failures are per-entry unless the primary snapshot cannot be established.

**Tests and real acceptance:** fixture tests cover tracked/dirty/ignored files, symlink inside/outside root, special files, submodules, nested destination, hardlink rejection, reflink success where available, copy fallback, mutation/hash mismatch, and limits. Real acceptance uses a disposable local repository with dirty and ignored files, a submodule fixture, a second already-discovered local reference repository, and `stat`/hash checks proving destination edits do not alter sources.

**Parallel ownership:** filesystem/snapshot owner can work after CTX-01 path policy is fixed; Git status policy should be reviewed separately. Provider owners consume the resulting snapshot contract but do not implement it.

**Deferrable:** ignored-file opt-in and submodule expansion may wait. Root containment, no symlink traversal, no hardlinks/alternates, and dirty-state reporting are first-slice requirements.

## Source ingestion contracts

### SRC-01: provider contracts, cache, and canonical frontmatter

**Dependencies:** FND-02/03 provider/cache contracts and byte/hash limits. Cache/normalization can be built independently; companion materialization integrates with CTX-01. Enables all provider-specific stories.

**Proposed files/contracts:** `crates/cockpit-core/src/sources/mod.rs` for `SourceProvider`, `SourceRef`, `SourceCapabilities`, `SourceAsset`, `FetchRequest`, `FetchResult`, `Freshness`, and `ProviderError`; `crates/cockpit-core/src/sources/cache.rs` for immutable central cache records and companion materialization; `crates/cockpit-protocol/src/v1.rs` for read-only source operations.

Canonical Markdown envelope frontmatter should contain `schema_version`, `provider`, `resource_type`, `canonical_id`, `provider_instance`, `source_url` when available, `fetched_at`, `source_revision`/ETag when available, `content_hash`, and `generated = true`. The central cache owns immutable, revision-addressed source payloads. The companion manifest owns association and materialized-copy ownership. A changed source creates a new cache record; syncing the companion replaces a generated copy atomically only after revision/hash validation. Central cache garbage collection must never run during workspace destruction.

Revision comparison order: provider revision/ETag/hash if supplied; otherwise canonical normalized-content hash. Treat equal revision with differing content as provider inconsistency and retain both evidence records. Treat unavailable revision as `unknown`, not fresh. Keep source URL for display/opening only; it is never fetched outside the selected provider adapter.

**Implementation steps:** define capabilities independently; normalize provider records to Markdown/frontmatter; validate schema and size before cache commit; write immutable cache records; materialize generated companion copies; expose freshness and per-asset failures.

**Failures/tests/acceptance:** test malformed envelope, missing identity, duplicate revision, hash mismatch, cache write interruption, stale/unknown freshness, and generated/user conflict. Real acceptance pulls a fixture snapshot twice, confirms idempotent hash/revision behavior, changes one asset, and confirms only that generated asset is replaced while user files survive.

**Parallel ownership/deferrability:** contract/cache owner first; provider stories can develop against fixtures afterward. Cache garbage collection is deferrable; immutable content-hash addressing and shared cache objects are required.

### SRC-02: Gitea issue first slice and Tea capability verification

**Dependencies:** SRC-01; LIFE-01 local primary repository identity; configured provider credentials supplied by existing local tooling. No remote repository discovery.

**Proposed files/contracts:** `crates/cockpit-core/src/sources/gitea.rs`; `crates/cockpit-host/src/providers/tea.rs` if the wrapper is used; fixture files under `crates/cockpit-core/tests/fixtures/gitea/`. The provider exposes `get_issue`, `list_issue_comments_timeline`, and capability probe. Gitea’s official API index lists issue retrieval, issue comments/events, and pull-request review operations; Tea is the official CLI and advertises issue listing/commenting and PR workflows ([Gitea API](https://docs.gitea.com/api/), [Tea](https://about.gitea.com/products/tea/)).

Probe installed Tea version/help and machine-readable support before parsing. If capability or output contract is absent, return `unsupported` and keep existing context/terminals available; do not parse a human table as proof. The first slice uses the configured Tea executable or a configured compatible local wrapper and its existing credential handling. Capture version/help and machine-readable output fixtures before enabling each capability. A direct HTTP adapter is a separate explicitly configured later implementation, not fallback. If later selected, it uses header authentication rather than query tokens ([authentication](https://docs.gitea.com/api/#authentication)).

**Implementation steps:** require artifact owner/repository to match the local primary repository identity; fetch issue metadata then comments/timeline with pagination and limits; normalize labels, milestone, assignees, timestamps, author IDs, canonical URL, and comment IDs; parse only bounded recognized references whose repository is unambiguous; cache and materialize.

**Failures/tests/acceptance:** fixture-test pagination, missing permissions, 404, rate/size limits, deleted comment, malformed body, Tea unavailable, unsupported version, and token-redaction logs. Real acceptance uses a configured Gitea instance and one issue with comments, verifies read-only behavior and canonical snapshots, and confirms an artifact URL for a different repository is rejected until a matching local primary repository is supplied.

**Parallel ownership:** Gitea provider owner; cache owner reviews normalized envelope. The provider must not own context directory policy. PR/wiki stories may proceed against the same provider interface after SRC-01.

**Deferrable:** automatic reference expansion beyond direct comments and a separately configured direct HTTP adapter are deferrable; primary issue plus bounded comments are the first slice.

### SRC-03: bounded hydration, sync, and freshness

**Dependencies:** SRC-01 and one provider adapter; CTX-01 manifest.

**Proposed files/contracts:** `crates/cockpit-core/src/sources/hydration.rs` with `HydrationBudget {max_depth, max_assets, max_comments, max_bytes_per_asset, max_total_bytes, timeout}` and `HydrationReport {completed, skipped, failed, truncated}`; `sync.rs` for revision-aware materialization.

**Implementation steps:** fetch primary first; queue only recognized references; deduplicate by canonical ID; detect cycles; stop at every budget boundary; preserve successful assets when secondary assets fail; compare provider revision/ETag/hash; atomically materialize changed generated assets; retain stale/conflict/failure status.

**Tests and real acceptance:** deterministic graph fixtures for depth/cycles/duplicates, budget truncation, partial failure, unchanged revision, changed revision, unknown revision, and user edit conflict. Real acceptance uses a multi-comment issue/PR graph and verifies bounded network calls, persisted report, and no deletion of prior successful assets on a later partial failure.

**Parallel ownership/deferrability:** hydration owner depends on SRC-01; provider-specific graph parsing remains with each adapter. Background scheduling and automatic periodic refresh are deferrable; explicit refresh is required first.

### SRC-04: review/MR/PR source

Selectable after the main issue loop. Provider-neutral review identity is designed now; concrete ingestion may be deferred.

**Dependencies:** SRC-01 and SRC-03; provider capability probe. “MR/PR” is a normalized review artifact; provider terminology remains metadata.

**Proposed files/contracts:** `crates/cockpit-core/src/sources/review.rs`; provider adapter methods `get_review`, `get_diff`, `list_changed_files`, `list_reviews`, `list_review_comments`; fixture diff with old/new sides and file/line anchors.

Gitea’s API index explicitly lists PR retrieval, diff/patch, changed files, reviews, review comment operations and review listing ([Gitea API](https://docs.gitea.com/api/)). Normalize base/head refs, revision IDs, changed-file paths, review state, author/timestamps, body, and comment side/ranges. Preserve provider anchors and verbatim snippets where supplied; never claim line rebasing after source changes. All operations are reads. The API operation named “Reply to a pull request review comment” is a write and must never be used for collection. Replies are included only if a documented read response exposes them and the configured adapter fixture proves their structure; otherwise report that subcapability unavailable.

**Failures/tests/acceptance:** test binary/renamed files, deleted lines, missing review permission, pagination, oversized diff, stale head revision, and comment without a resolvable file. Real acceptance opens a real PR and verifies metadata, changed files, reviews, replies, source revision, and read-only server audit state.

**Parallel ownership/deferrability:** review provider owner can proceed after SRC-01. Diff rendering and any comment-to-agent transfer are separate later work; review ingestion itself remains useful without them.

### SRC-05: wiki

Selectable after the main issue loop; independently deferrable.

**Dependencies:** SRC-01 and SRC-03; local primary repository identity.

**Proposed files/contracts:** `crates/cockpit-core/src/sources/wiki.rs`; fixtures for page listing, page retrieval, revision, base64 content, and missing page.

Gitea documents wiki listing, retrieval, and revisions in the API index. Wiki retrieval returns base64 content, title/sub-URL, HTML URL, and last-commit SHA/date/message; listing uses page/limit pagination ([wiki page](https://docs.gitea.com/api/next/operations/repo-get-wiki-page/), [Gitea API](https://docs.gitea.com/api/)). Decode and validate bounded content, preserve page title and revision metadata, normalize into the canonical envelope, and reject write operations.

**Failures/tests/acceptance:** test disabled/missing wiki, invalid base64, pagination, revision mismatch, oversized page, and permission errors. Real acceptance fetches a real page and confirms its revision/content hash and that Cockpit makes no wiki mutation.

**Parallel ownership/deferrability:** wiki owner after SRC-01; revision history beyond the current page revision is deferrable.

### SRC-06: optional telemetry

**Dependencies:** SRC-01 and a separately approved provider adapter. No dependency from core context browsing to telemetry.

**Proposed files/contracts:** `crates/cockpit-core/src/sources/telemetry.rs` with `TelemetryProvider` capability, bounded log/trace asset type, timestamp/source metadata, and explicit `unsupported` default. No provider is selected by this plan.

Telemetry must obey the same local-root, byte/time, redaction, and immutable-cache rules. Treat logs as untrusted text; do not execute or interpret embedded commands. Preserve source timestamps and mark partial windows/truncation.

**Failures/tests/acceptance:** fixture-test malformed records, binary payload, secret-redaction failure, time-window limit, and unavailable adapter. Real acceptance is deferred until a concrete provider and local credential policy exist.

**Parallel ownership/deferrability:** isolated telemetry owner; all of SRC-06 is deferrable and must not delay issue/PR/wiki or local context browsing.

## Cross-cutting verification gate

Before implementation is called complete, exercise the real path with a configured local primary repository, a disposable companion directory, an already-discovered local reference repository, dirty/ignored/symlink/submodule cases, a Gitea issue with comments. Add review/wiki acceptance only when SRC-04/05 are selected; their absence must not block the main loop. Verify bounded limits, source revisions/hashes, cache versus companion ownership, generated/user conflict preservation, reflink or copy mode, no hardlinks, path containment, token redaction, and zero remote writes. A green unit suite alone is insufficient for these filesystem/provider/runtime claims; acceptance must inspect the resulting tree, hashes, and provider state.


## Exact storage and refresh decisions

Use one companion per task worktree association. Suggested layout is `context-manifest.json`, `sources/<provider-instance>/<kind>/<safe-id>.md`, `repos/<catalog-name>/...`, and `notes/...`. A user may add other files anywhere except the reserved metadata/staging paths; bounded discovery finds them without a generated INDEX or scratchpad. The example is relative to configurable roots, never a hard-coded home directory.

Managed membership is authorized by the manifest plus last-written hash, never by a frontmatter `generated` flag alone. User-created frontmatter cannot grant Cockpit overwrite/delete permission. On refresh, compare the current companion file with the previous written hash before replacement. A source-side deletion marks the asset removed-at-source and retains the local snapshot until an explicit removal action. Removing a source from this companion never deletes its shared cache object or its original local repository.

Canonical content hashing excludes volatile `fetched_at`, last-checked timestamps, and the hash field itself. Hash normalized semantic data plus stable provenance; record retrieval status separately. Equal content with a later fetch time must remain unchanged, and `content_hash` cannot hash an envelope containing itself. Source-level ETags/revisions may need comment-list revisions or a canonical composite hash: an unchanged issue metadata revision alone cannot prove comments are unchanged.

Local snapshots are copies of filesystem bytes, not Git clones. Default `working_tree` includes tracked current bytes and untracked non-ignored files under configured exclusions; it does not duplicate primary source automatically. Additional repositories are explicitly selected. Copy eligible files to a staging directory, track source identity/size/mtime/hash before and after, then atomically publish a complete snapshot generation. A filesystem-wide point-in-time snapshot is not guaranteed by per-file reflink; concurrent source changes trigger bounded retry or a visible partial/stale result. Never label a mixed-generation tree an exact commit checkout.

Fallback from reflink to copy is allowed for unsupported/cross-device cases, with a visible copy-mode/performance note. Permission, space, corruption, and policy failures are not silently treated as unsupported CoW. Hardlink aliases and Git alternates are never used. Reflink/copy code uses a safe Rust library or a fixed-argv vetted system utility, consistent with the workspace's unsafe-code prohibition.

Provider code belongs in a reusable `crates/cockpit-providers` adapter crate once SRC-02 has real behavior. Core defines the normalized interface and orchestration; browser/native hosts compose the same adapter. Do not place Tea business logic only in the browser host or require native callers to depend on host HTTP code. No crate is added merely to hold empty interfaces.
