# Next-level capability audit

Date: 2026-09-04
Scope: implemented Cockpit versus the next requested slice: Herdr worktree create/open/remove, per-workspace context association and environment, and a graphical nonterminal context panel beside terminals. This is a read-only audit of the repository, the installed Herdr binary/schema, and installed plugin source. No Herdr session or server state was mutated.

## Evidence boundary

The installed executable is `/home/nnex/.local/bin/herdr`, version `0.8.2`. `herdr api schema --json` reports protocol 22 and schema version 1. Its CLI help exposes `worktree create`, `worktree open`, and `worktree remove`, and `workspace create --env KEY=VALUE`; these commands were inspected without invoking a mutating subcommand. The matching Herdr source checkout is available at `/tmp/herdr-cockpit-master` and was read directly for handler semantics. The installed `herdr-agent-inbox` plugin is an existing consumer of public plugin/socket surfaces; it reports workspace/pane metadata and agent state but does not provide worktree or context ownership.

Relevant existing project research is [herdr-0.8.2-bootstrap.md](herdr-0.8.2-bootstrap.md), especially its documented method and event inventory. Its conclusions agree with the installed schema: Herdr uses `workspace` in the API while Cockpit presents `Space` in the UI.

## What is implemented now

### Herdr mirror and ordinary layout mutations

`cockpit-herdr` validates exact Herdr `0.8.2`, protocol 22, schema 1, and a fixed list of 20 required methods, including `workspace.create`, `workspace.rename`, `workspace.close`, `worktree.list`, and all currently supported tab/pane mutations (`crates/cockpit-herdr/src/cli.rs:29-53`, `1215-1266`). It does not require or validate `worktree.create`, `worktree.open`, or `worktree.remove`.

The application seam is `HerdrAdapter`: inspect, list sessions, snapshot, focus, generic resource mutation, subscribe, and open a terminal (`crates/cockpit-core/src/lib.rs:64-91`). `CockpitService::mutate` validates and forwards the closed `ResourceMutationRequest`, then reads a fresh authoritative snapshot (`crates/cockpit-core/src/lib.rs:187-204`; the concrete adapter mapping is `crates/cockpit-herdr/src/cli.rs:1309-1330`). This is a usable seam for adding typed lifecycle operations, but the current generic mutation enum has no worktree variants. `crates/cockpit-herdr/src/lib.rs` only re-exports `HerdrCliAdapter` (`:1-5`).

The protocol's `SpaceCreate` maps directly to `workspace.create`, always sending `focus: true` and optional `cwd`/`label` (`crates/cockpit-protocol/src/v1.rs:265-328`; `crates/cockpit-herdr/src/cli.rs:1891-1902`). The frontend exposes “new” Space and ordinary tab/pane operations, with inline editing and context menus (`src/app/App.tsx:695-727`, `748-777`). A Space row shows Git data only when `worktree.list` happens to find an already-open worktree for that workspace; the adapter makes one `worktree.list` request per snapshot Space and maps `repo_key`, `repo_name`, branch, checkout path, and linked-worktree state (`crates/cockpit-herdr/src/cli.rs:1281-1307`, `536-573`).

Terminal attach is a real transport seam, but it is terminal-only: the client contract ends at `openTerminal` and session subscription (`src/client/CockpitClient.ts:62-81`). The workbench renders visible panes into the main area and has no nonterminal panel slot (`src/app/App.tsx:768-785`).

### Configuration and environment today

Cockpit can configure executable, named session, and socket via CLI/environment (`crates/cockpit-host/src/bin/cockpit.rs:29-40`, `66-70`; `crates/cockpit-herdr/src/cli.rs:75-124`). This is Herdr connection configuration, not per-workspace context configuration. The current `SpaceCreate` protocol has only `cwd` and `label`; there is no `env`, context path, association ID, repository/task artifact, or ownership metadata.

The architecture documents promise a later workflow—create/open worktree, create companion context, attach it through Herdr metadata/environment, hydrate context—but those are design decisions rather than implemented services (`CONTEXT.md`, §4.3 and §6; `DECISIONS.md`, “Workspace and filesystem lifecycle” and “Terminal environment”). `NEXT_PHASE_PLAN.md:148-149` explicitly defers workspace/worktree creation/destruction and companion context/viewer/hydration/search.

## Installed Herdr capabilities relevant to the next slice

The installed schema's request definitions provide these exact parameters:

| Capability | Installed wire operation/parameters | Consequence for Cockpit |
|---|---|---|
| Create a plain workspace | `workspace.create`: `cwd`, `env` map, `focus`, `label`, optional `source_workspace_id` | Existing Space create drops `env` and source-workspace semantics. |
| Create a Git worktree workspace | `worktree.create`: optional `workspace_id`, `cwd`, `branch`, `base`, `path`, `label`, `focus`, `trust_repository` | Supports the requested create operation without a Herdr server change. |
| Open an existing worktree | `worktree.open`: optional `workspace_id`, `cwd`, `path`, `branch`, `label`, `focus`, `trust_repository` | Supports open by path/branch and optional target workspace. |
| Remove a worktree checkout | `worktree.remove`: required `workspace_id`, optional `force`, `trust_repository` | Supports explicit destructive teardown; Cockpit must own confirmation and companion cleanup ordering. |
| List/provenance | `worktree.list`: optional `workspace_id`, `cwd`, `trust_repository`; result includes `repo_key`, `repo_name`, `repo_root`, checkout path, branch/detached/link/prunable flags, and `open_workspace_id` | Existing `worktree.list` mapping is read-only and enough to display provenance, but not enough to create a Cockpit context association. |
| Workspace display metadata | `workspace.report_metadata`: workspace ID, source, token map, optional sequence/TTL | This is display metadata with bounded token values, not a general filesystem/context registry. |
| Pane display metadata | `pane.report_metadata`: pane ID, source, title/agent/state labels/token map, optional sequence/TTL | Installed plugin uses this class of public surface for inbox labels; it cannot carry a companion tree. |

The installed CLI help independently confirms the lifecycle commands and `workspace create --env KEY=VALUE`. The schema also exposes workspace lifecycle events (`workspace.created`, `updated`, `metadata_updated`, `closed`, etc.) and worktree events (`worktree.created`, `opened`, `removed`), so Cockpit can refresh its authoritative snapshot after lifecycle changes. The existing adapter currently subscribes to a narrower event set and treats topology changes as a reason to resnapshot (`crates/cockpit-herdr/src/cli.rs:1678-1696`, `1332-1457`); it should add the missing lifecycle event names when lifecycle APIs are added.

## Deeper Herdr source findings

The Herdr implementation clarifies an important ambiguity in `workspace_id`. Worktree create/open accept **either** `workspace_id` or `cwd`, never both (`/tmp/herdr-cockpit-master/src/app/api/worktrees.rs:186-220`, `246-297`). The supplied workspace is resolved as the source repository/workspace; it is not an instruction to mutate that existing workspace's cwd or environment. Create runs `git worktree add` asynchronously, then opens or reuses a workspace for the resulting checkout (`/tmp/herdr-cockpit-master/src/app/api/worktrees/deferred.rs:94-212`, `399-462`). If that checkout is already open, the existing workspace is reused; otherwise Herdr creates a new workspace with a newly generated ID. Open similarly reuses an already-open checkout and otherwise creates a new workspace (`/tmp/herdr-cockpit-master/src/app/api/worktrees.rs:120-181`). `label` can rename the resulting workspace, and `focus` only changes focus.

Worktree create's `trust_repository` is passed to the Git operation, while Herdr rejects linked worktree workspaces as a source and directs callers to the repository parent (`/tmp/herdr-cockpit-master/src/app/api/worktrees/deferred.rs:197-203`; `worktrees.rs:300-346`). Remove requires the target workspace to carry linked-worktree membership; it does not remove a plain repository workspace (`deferred.rs:215-257`). The operation is asynchronous and guards duplicate operations by checkout/workspace keys (`deferred.rs:140-160`, `281-300`). This supports a Cockpit flow that captures the returned workspace ID and only offers destructive removal when the authoritative membership says it owns a linked checkout.

Herdr's workspace creation is the viable environment injection point. `WorkspaceCreateParams.env` is normalized and passed to `create_workspace_with_launch_env`, which constructs the initial root pane with that launch environment (`/tmp/herdr-cockpit-master/src/app/api/workspaces.rs:39-81`; `/tmp/herdr-cockpit-master/src/app/creation.rs:118-145`). The socket documentation states that env applies to newly launched processes only and Herdr-managed variables override collisions (`/tmp/herdr-cockpit-master/docs/versions/0.8.2/website/src/content/docs/socket-api.mdx:298-302`). Worktree create/open themselves have no `env` field, so context variables cannot be injected atomically into the resulting root pane. A no-server-change sequence is therefore: (1) call `worktree.create` or `worktree.open`; (2) use the returned workspace ID to create a tab or split pane with the sanitized context env; (3) optionally leave the original root pane alone or close it under explicit user policy. Calling `workspace.create` would create a separate workspace and is not an association mechanism. Existing processes cannot be retrofitted.

The snapshot exposes workspace IDs, labels, display tokens, worktree provenance, and pane cwd/title/agent fields, but no process environment or arbitrary companion path (`/tmp/herdr-cockpit-master/src/api/schema/session.rs:8-22`; `src/api/schema/workspaces.rs:61-85`; `src/api/schema/panes.rs` fields). `workspace.report_metadata` can publish bounded display tokens and emits `metadata_updated`; it cannot persist or retrieve a context association as a typed resource. Thus Cockpit should retain its association in a Cockpit-owned companion manifest keyed by the returned Herdr workspace ID plus checkout provenance, while using a safe display token (for example, context status) only as optional UI decoration.

Workspace IDs are generated public handles (`w` plus readable base32 counter) and persisted workspace restoration reserves the maximum restored number before new allocation (`/tmp/herdr-cockpit-master/src/workspace.rs:104-110`, `151-173`; `/tmp/herdr-cockpit-master/src/persist/restore.rs:294-303`). They are stable across session restore for restored workspaces and are not reused within the process after closure. A fresh session that has no restored workspace can generate the same low handles again. Therefore a Cockpit manifest must not treat a bare workspace ID as globally durable identity: pair it with session identity and canonical checkout/repository provenance, and reconcile after `session.snapshot`/`worktree.list`.

The source also confirms there is no native nonterminal plugin surface in Herdr 0.8.2: plugin panes are Herdr-managed terminal panes or popups, and the documentation explicitly says native nonterminal plugin panes are outside plugin v1 (`/tmp/herdr-cockpit-master/docs/versions/0.8.2/website/src/content/docs/cli-reference.mdx:460-471`). The feasible replacement is therefore a Cockpit-rendered GUI surface occupying the rectangle of a detected Herdr terminal pane, while neighboring terminals remain ordinary xterm surfaces. Detection can use the `plugin.pane.open` receipt plus read-only `plugin.list` and `pane.process_info`; it requires no extension handshake or IPC. See [pane replacement evidence](next-level-pane-replacement.md).

## Missing-feature matrix

| Feature | Evidence of current state | Status | Proposed next seam/resolution |
|---|---|---|---|
| Repository discovery under configured root | Architecture requires it, but no repository/provider/worktree service exists in `crates/cockpit-core`; current frontend “new” calls plain `space_create` (`src/app/App.tsx:769`) | Missing | Add a core `RepositoryCatalog` capability with configured-root enumeration and typed repository identity. Keep filesystem policy in core, Herdr calls in adapter. |
| Worktree create | No protocol request, no trait method, and required-method gate omits `worktree.create` | Missing | Add typed `WorktreeCreateRequest/Response` to protocol and `HerdrAdapter`; map directly to schema params; return authoritative workspace/worktree provenance and snapshot. |
| Worktree open | Same absence; existing `worktree.list` is only enrichment during snapshot | Missing | Add typed open operation. Resolve path/branch in core and let Herdr validate repository/trust. |
| Worktree remove | Same absence; current UI only calls `workspace.close`, which is not checkout removal (`src/app/App.tsx:701-706`) | Missing/high risk | Add explicit remove operation with confirmation, ownership check, `force`/trust policy, and post-remove resnapshot. Do not equate closing a Space with deleting a checkout. |
| Worktree capability gate | `REQUIRED_METHODS` does not include the three lifecycle methods (`crates/cockpit-herdr/src/cli.rs:32-53`) | Missing | Gate only the worktree feature on those methods, preserving mirror usability with an explicit unsupported-capability error. |
| Per-Space worktree provenance | Read-only Git summary exists, sourced from `worktree.list` (`crates/cockpit-protocol/src/v1.rs:62-83`; `crates/cockpit-herdr/src/cli.rs:536-573`) | Partial | Extend response with stable provenance/ownership fields returned by lifecycle operation; never create a second local registry. |
| Companion context creation | No context filesystem service, DTO, or endpoint; only architecture prose (`CONTEXT.md`, §4.3/§5.5) | Missing | Add core-owned `ContextStore` rooted by configured policy. Create atomically after Herdr worktree success; return path/resource identity without exposing arbitrary filesystem access. |
| Context association with Space | Herdr metadata tokens are bounded display metadata, and current protocol has no context field | Missing | Store association in Cockpit-owned companion metadata keyed by authoritative workspace/worktree provenance, or use a narrowly defined Herdr display token only for a human label. Do not misuse `report_metadata` as a registry. |
| Context environment for new terminals | `workspace.create` supports an `env` map in Herdr schema, but Cockpit `SpaceCreate` and `mutation_call` omit it; `TerminalOpenRequest` has no environment (`crates/cockpit-protocol/src/v1.rs:265-283`, `371-403`) | Missing | Carry a sanitized, non-secret context/workspace environment through the workspace creation orchestration. Define inheritance semantics and reject secret values. Prefer Herdr workspace/session environment authority. |
| Existing terminals after association | Herdr `workspace.create.env` affects launched process creation; no current operation changes env of existing PTYs | Unsupported/needs explicit UX | Document that association/env applies to newly created terminals; offer resync/new pane guidance. Do not imply retroactive process mutation. |
| Context tree enumeration | No core/client method or route; `CockpitClient` has only status/sessions/snapshot/focus/mutate/events/terminal (`src/client/CockpitClient.ts:62-81`) | Missing | Add typed, bounded `context.list/read` core APIs with path containment checks and file identity/mtime. |
| Markdown/frontmatter/plain text/image viewers | No context component. `.agent-context` CSS is agent metadata styling, not a file viewer (`src/app/styles.css:421`, `1328`) | Missing | Add a read-only `ContextPanel` component and safe type-based renderer with finite limits and external-open fallback. |
| Graphical context panel beside terminals | Main workbench currently allocates tab strip plus `.pane-canvas` only (`src/app/App.tsx:768-777`) | Missing | Add a Cockpit renderer selected by explicit pane provenance and process inspection; omit xterm for the proven pane rectangle while neighboring terminal panes continue normally. Herdr remains authoritative for layout/focus/close. |
| Context refresh/search | No file watcher, refresh, or search client method; plan says ripgrep is core-mediated and deferred (`CONTEXT.md`, §5.5; `NEXT_PHASE_PLAN.md:149`) | Missing/deferred | Add bounded watcher invalidation and allowlisted core-mediated ripgrep after list/read contract. |
| Partial provisioning and cleanup | No orchestration transaction or owned-resource record | Missing | Model steps and durable ownership in core operation result. On failure, preserve diagnosable partials; on destroy, remove only Cockpit-owned context plus requested Herdr checkout. |
| Provider hydration (Gitea issue first slice) | No provider crate in current file list; explicitly deferred | Deferred | Keep provider ingestion separate from context browser/lifecycle. Add after generic context resource contract. |
| Review/issue/wiki/telemetry domains | Architecture defines provider-neutral domains, but no implementations | Deferred | Preserve capability interfaces and explicit unsupported errors; do not couple worktree/context panel to remote writes. |
| Agent history/resumable conversations and inbox popup | Installed plugin supports these, but Cockpit only mirrors live agent summaries; decisions defer history/popup | Deferred | Consume plugin-derived display tokens only if useful; do not reproduce plugin state or mutate its control socket. |
| Settings, credentials, remote access | Explicitly deferred in `CONTEXT.md`/`DECISIONS.md` | Deferred | Keep roots, executable, socket, preview limits, and provider settings in config/env/CLI; no secrets in snapshots or generated environment. |

## Contradictions and proposed resolutions

1. **Architecture says “use Herdr’s worktree API,” implementation gates only `worktree.list`.** Resolve by adding the three lifecycle methods as optional capability gates and typed adapter operations. Keep the base Herdr mirror usable when a release lacks them, with a visible unsupported capability.

2. **`workspace.close` is presented as Space destruction, while the desired lifecycle requires checkout removal.** Herdr distinguishes `workspace.close` and `worktree.remove`; resolve by making “Close Space” a non-destructive Herdr operation and adding a separately labeled “Remove worktree” flow with confirmation and ownership proof.

3. **The architecture promises context attached through Herdr metadata/environment, but Herdr metadata is display-only and bounded.** The schema’s `report_metadata` token maps are not a general context registry. Resolve by keeping context ownership/association in Cockpit core metadata keyed to Herdr’s authoritative workspace/worktree identity, using Herdr env for new terminal discovery and optional display tokens only for labels.

4. **Herdr has no native nonterminal pane or file surface protocol.** Resolve the first implementation as a Cockpit-rendered replacement for a detected Herdr terminal rectangle. Keep the helper as TUI fallback, omit xterm only after explicit launch receipt/process reconciliation, and do not require extension communication.

5. **The current Space creation path sends `focus: true` unconditionally, while the installed schema defaults focus false and worktree operations expose trust flags.** Resolve with explicit user intent in typed requests and a central trust policy. Avoid silently trusting repositories or focusing another client’s resource.

6. **`workspace_id` on worktree create/open might be mistaken for a destination workspace to mutate.** Source inspection shows it selects the repository source and the resulting checkout is opened in an existing matching workspace or a newly generated workspace. Resolve Cockpit's request model around `source_workspace_id` and returned `workspace_id`; never promise in-place cwd/env mutation.

7. **Workspace ID durability is narrower than a global Cockpit resource ID.** Restored IDs are reserved, but fresh sessions can start their public sequence again. Resolve by scoping associations to Herdr session identity and canonical worktree/repository provenance, with reconciliation on every authoritative snapshot.

## Recommended implementation order

1. Extend protocol/core/adapter capability discovery and typed worktree create/open/remove; add fixture tests against the captured schema and no live mutation tests beyond a disposable Herdr session.
2. Add repository catalog and lifecycle orchestration with explicit ownership/partial-failure records, then wire the UI flow and worktree provenance display.
3. Add context store and association model, plus sanitized environment propagation for newly created Herdr terminals.
4. Add context list/read/refresh/search APIs and the graphical Context tab/dock beside terminals. Verify with a real companion tree, Markdown/frontmatter, bounded text, image, unsafe/oversized files, and user-created files.
5. Add provider hydration and deferred domains behind capability interfaces only after the generic context surface is proven.

This ordering preserves Herdr as authority for sessions, workspaces, panes, processes, focus, and worktree lifecycle while giving Cockpit ownership only of its companion context resources and presentation.
