# Architecture and shared contracts

Status: proposed implementation design, prepared against Cockpit `7e8fe25` on 2026-09-04. No feature implementation is included. Story IDs are stable planning references, not existing APIs.

## Product boundary

The main loop is: select a local repository, optionally resolve an issue or review URL, create or open a task worktree through Herdr, attach a companion directory, download selected context, inspect files beside terminals, collect comments, and paste the batch into an agent pane in the same tab. A user submits the pasted text separately.

The repository selector remains required even when the user starts with an artifact URL. A link supplies task identity and branch suggestions, not permission to clone a remote repository. Additional repositories must already be in the configured local catalog. They become independent snapshots inside context, preferably through reflinks. Hardlinks are not an acceptable replacement.

Herdr-server stays unchanged. It remains authoritative for sessions, Spaces, worktrees, tabs, terminal panes, agent state, focus, terminal surface geometry, and process lifetime. Cockpit owns companion files, downloaded snapshots, operation recovery records, comment drafts, and the presentation of its context viewer. None of these records is a second inventory of live Herdr resources.

## Current seams and known corrections

The current implementation already has the important transport separation. `crates/cockpit-core/src/lib.rs` defines `HerdrAdapter` and `CockpitService`; the adapter implementation is in `crates/cockpit-herdr/src/cli.rs`. The frontend uses `src/client/CockpitClient.ts`; browser and native mappings live in adjacent files. Rust protocol types in `crates/cockpit-protocol/src/v1.rs` generate `src/protocol/generated/v1.ts`.

`App.tsx` currently coordinates workbench layout, focus, mutations, and terminal streams in one file. Add a renderer choice inside the existing Herdr pane projection, then place Context and Review implementations in separate directories. Do not rewrite terminal coordination while adding a viewer. The current `status` capability struct has only terminal mouse input; new feature capabilities need explicit availability and reason fields and must distinguish CLI SGR emulation, xterm.js wheel delivery, and structured pointer routing.

Older UI research still describes protocol-20 exclusive terminal ownership and older xterm renderers. The current `CONTEXT.md` and `DECISIONS.md` protocol-22 client-shell contract overrides those statements. Comment delivery must use current source evidence, not recreate the retired takeover protocol. See [capability audit](../../research/next-level-capability-audit.md) and [Reviewr research](../../research/next-level-review-reference.md).

## FND-01: configuration and capability discovery

Required by the main path. Can be implemented independently of visual design.

Proposed files: `crates/cockpit-core/src/config.rs`, `capabilities.rs`, `crates/cockpit-host/src/config.rs`, and the existing CLI/native composition roots. Keep precedence resolution in shared code so both hosts report the same effective values.

Implementation steps:

1. Define a versioned TOML configuration with repository roots, worktree/branch templates, companion/cache/state roots, Herdr endpoint configuration, provider instances/executables, snapshot exclusions, preview/search limits, and operation limits.
2. Resolve built-in defaults, config file, environment overrides, then explicit invocation options in that order. Unknown keys and invalid paths return precise diagnostics. Return the origin of each effective non-secret setting for a future settings view.
3. Expand only documented template variables such as `{repo}`, `{task_id}`, and `{slug}`. Validate branch names through Git and destinations through configured-root policy. Never evaluate templates as shell fragments.
4. Probe independent Herdr and provider capabilities. An unavailable provider must not disable terminals or browsing existing files. Distinguish unsupported version, missing executable, unconfigured provider, authentication required, and temporary failure.
5. Revalidate a changed executable/configuration before use. Environment overrides are startup-scoped initially; avoid silently changing an in-flight operation when the file changes.

Proposed defaults are explicit implementation choices: preview 2 MiB text, 20,000 lines, image 20 megapixels and 20 MiB compressed, one diagram 32 KiB with a two-second render budget, 500 search results and five-second search budget, two concurrent provider fetches. The implementation must verify these on the actual WebKit runtime and adjust through one configuration schema. Limits report truncation or refusal; none silently alter the source file.

Acceptance: CLI and native launch with the same config and overrides report identical effective values; invalid templates cannot leave configured roots; missing Tea disables only affected source actions. Tests cover precedence and unknown settings, including roots containing spaces. The native smoke must use a disposable config and session.

## FND-02: shared operations and transport contract

Required. Freeze shared identity/error/operation conventions and the operation contracts needed by the next vertical slice before its parallel backend, host, and UI implementation. The table is the complete planned inventory; optional/later families do not need generated stubs at M0.

Add protocol modules as needed, re-exported through `v1`: `repository`, `workspace_setup`, `context`, `sources`, `comments`, and `operations`. Keep the existing generated-file path until a deliberate exporter change is independently verified. Proposed DTO names below must not be confused with installed Herdr methods.

| Operation group | Proposed task-level contract | Important result |
|---|---|---|
| Configuration | `configuration.inspect`, `capabilities.inspect` | Effective non-secret values, availability/reason per feature |
| Repositories | `repositories.list`, `repositories.inspect` | Stable local root identity, Git/common-dir identity, configured label, accessible state |
| Artifact resolution | `artifacts.resolve` | Typed provider resource, matching local repository candidates, branch suggestion, mismatch diagnostics |
| Setup | `workspace.plan`, `workspace.start` | Reviewed plan plus operation ID; exact requested effects |
| Recovery | `operations.get/list/retry/cancel` | Step states and owned partial artifacts; explicit non-retryable/uncertain outcomes |
| Destruction | `workspace.removal_plan`, `workspace.remove` | Exact checkout/context resources and matching confirmation token |
| Context association | `context.resolve`, `context.attach` | Companion ID, path policy, matched authoritative Herdr provenance |
| Context files | `context.tree`, `context.read`, `context.search`, `context.subscribe` | Bounded pages, file revisions, source line metadata, change generation |
| Source management | `sources.plan/add/refresh/remove` | Source identities, selected dependencies, cache/companion status |
| Reference drafts | `comments.list/upsert/remove/preview` | Versioned per-tab batch, immutable source excerpts, payload size |
| Delivery | `comments.paste` | Explicit target and result classified as accepted, rejected, or outcome unknown |
| Extension rendering | `extensions.detect/open/set_renderer` | Verified launch/process evidence or explicit per-pane override; `set_renderer` is Cockpit-local presentation, never a Herdr method; no extension IPC |
| Local review | `review.snapshot/files/file/refresh` | Cockpit-owned bounded Git diff and immutable anchors |
| External viewing | `context.open_external` | Host-mediated action limited to an approved context file/type |

The logical names are shared operations, not permission to expose a generic RPC multiplexer. Native commands and browser routes use a fixed allowlist. UI components call typed `CockpitClient` methods. Core rules never branch on Tauri versus HTTP.

Long work returns a stable operation ID promptly. A separate ordered event stream emits `{operation_id, generation, sequence, step, state, progress, error}`; a bounded `get` snapshot allows reconnect without requiring retained event history. Sequences belong to Cockpit operations only. They are not invented Herdr event cursors. Host shutdown cancels workers at safe boundaries and records interrupted operations; restart requires explicit recovery rather than repeating remote/local side effects.

Cancellation stops scheduling new steps, waits for or terminates bounded subprocesses safely, preserves completed work, and reports residual artifacts. Closing a progress dialog merely hides it. Never interpret closing a dialog or disconnecting the browser as consent to delete a worktree.

## FND-03: identity, concurrency, and storage rules

Required. This is a contract increment before lifecycle and ingestion writers land.

Use the following distinct identities:

- `RepositoryId`: a configured catalog identity verified against canonical root and Git common directory. A path match alone after a directory replacement is insufficient for destructive actions.
- `HerdrResourceRef`: endpoint identity, named session, workspace/tab/pane IDs as applicable, plus the current client session epoch. Live authority always comes from Herdr.
- `CompanionId`: random identifier in an owned companion manifest. The manifest records repository/worktree provenance and its association evidence. It does not assert whether a Space is currently live.
- `SourceId`: provider instance plus resource kind and canonical identifier, or local repository identity plus selected snapshot mode. Provider name alone cannot distinguish two Gitea installations.
- `FileRef`: a tagged authorized root reference (`companion` with CompanionId, or `checkout` with RepositoryId and verified checkout identity), normalized relative path, source ID if managed, revision/content hash, and size. Git review revisions additionally identify commit/index/worktree source and old/new side. Each variant has its own containment and provenance checks. The client never supplies an unrestricted absolute read path.
- `DraftBatchId`: a stable random batch ID, persisted owner/window identity, source provenance, and last-known Space/tab/pane location. A separate live attachment records the current endpoint/session epoch and verified identities. The epoch guards messages and paste eligibility, not durable identity. Restore disconnected batches as detached until provenance is revalidated. Drafts from different tabs/windows never merge implicitly.

Store context and operation ownership adjacent to the owned resource or beneath the configured Cockpit state root. Operation journals are receipts for effects performed by Cockpit, not a session/workspace registry. Reopening an association requires a fresh Herdr provenance check. Session names and reusable `w1` IDs alone never authorize reading or removing a companion.

Cross-process locking matters because native and browser hosts can run together. Use per-resource filesystem locks with bounded acquisition, and atomic writes on the same filesystem. Cache objects are immutable and keyed by content hash; pointer/manifest updates use compare-and-swap on a generation. Do not rely on an in-process mutex for filesystem correctness.

Crash recovery handles a file written before its manifest, a manifest staged before companion replacement, and completed Herdr work with no local acknowledgement. Preserve an unresolved operation and reconcile with Herdr and disk. Do not recreate a branch/worktree until absence is proven.

## Core file access policy

A root configured by the user is authority to enumerate eligible context, not authority to expose the whole filesystem. File operations resolve relative paths beneath an already-resolved companion. Reject absolute paths, parent traversal, NULs, encoded traversal, and symlink escapes. Use descriptor-relative no-follow opens or an equivalent safe Rust library to close the check/open race. The workspace forbids unsafe Rust, so do not add local unsafe syscall wrappers.

Only bounded regular-file contents become previews or repository snapshots. Show symlinks as links with metadata; do not traverse them in the main implementation. Ignore sockets/devices/FIFOs. Executable-bit source scripts may be shown as inert text after type/content checks; native binaries and active HTML/SVG are never executed in the viewer. This clarifies the older blanket “executable” refusal without allowing execution.

Context reads enforce revision checks so a selection and its line numbers refer to the same bytes. File-change notifications invalidate caches; they never grant write authority or silently rebase a saved comment.

## Browser and native boundaries

Mutating filesystem endpoints expand what the existing loopback gateway can do. In FND-02 add exact same-origin checks for browser requests and WebSocket upgrades, reject wildcard CORS, require JSON for mutation requests, validate Host against the bound loopback origin, and use an unguessable startup browser-session token where a cross-origin check cannot establish intent. Loopback binding remains mandatory. This is local gateway request integrity, not a remote login system. Native commands receive the same operation bounds through explicit Tauri capabilities.

Preview HTML has no Tauri access, arbitrary navigation, or direct filesystem URLs. Images and previews use bounded host operations/resource handles; revoke temporary URLs on disposal. External-open follows an explicit user click and typed file policy. Browser fallback can offer a bounded download or copyable local path when native opening is unavailable; report that difference as capability data, not component-level transport branching.

## Verification and integration ownership

The integration owner edits protocol exports, generated TypeScript, `CockpitClient.ts`, host route/command tables, manifests, and lockfiles. Domain implementers propose DTO changes before writing consumers. Backend and UI work can use checked-in fixtures after the contract freeze.

FND acceptance requires round-trip DTO and malformed-input tests, native/browser contract equivalence, operation reconnect/cancel cases, concurrent-host writes, root-containment tests, and one disposable real run of the capability and configuration paths. No new provider or lifecycle behavior can be reported implemented solely because DTOs compile.

## Personal code-tweak design

Apply CLEAN-01-04 before widening central files. This is a personal application, so prefer a small static renderer registry, explicit task-level operations, and ordinary Rust/TypeScript modules. No dynamically loaded frontend plugin system, generalized workflow engine, dependency-injection framework, or template language is required. A thin host adapter is intentional because there are two real transports; a provider abstraction should arrive with the first real provider behavior, not an empty crate hierarchy.
