# 02: Project setup and workspace lifecycle

Status: proposed implementation plan.
Scope: LIFE-01 through LIFE-04.
Dependencies: FND-01 config/capabilities, FND-02 typed operations, FND-03 identity/storage contract, CTX-01 companion store.
Authority: Herdr owns sessions, workspaces/Spaces, tabs, panes, PTYs, processes, focus, and worktree membership. Cockpit owns repository catalog policy, task/artifact interpretation, companion context resources, and orchestration records.

This plan supplies backend and protocol behavior for the selected graphical extension-pane design. Context occupies a real Herdr extension pane; see 07-extension-panes.md. Artifact types do not create Build/Review workbench modes.

## Proven runtime constraints

The installed Herdr 0.8.2 source and schema establish the following constraints:

- `worktree.create` and `worktree.open` accept either `workspace_id` or `cwd`, never both. `workspace_id` selects the source repository/workspace; it does not retarget that workspace's cwd or environment (`/tmp/herdr-cockpit-master/src/app/api/worktrees.rs:186-220`, `246-297`).
- Create performs the Git operation asynchronously, then reuses an already-open checkout workspace or creates a new workspace with a newly generated ID (`/tmp/herdr-cockpit-master/src/app/api/worktrees/deferred.rs:94-212`, `399-462`). Open has the same reuse behavior (`/tmp/herdr-cockpit-master/src/app/api/worktrees.rs:120-181`).
- Worktree create/open have no `env` parameter. `workspace.create.env` is applied only when that workspace's initial process is launched; process-launching `tab.create`/`pane.split` operations also accept env (`/tmp/herdr-cockpit-master/src/app/api/workspaces.rs:39-81`; `/tmp/herdr-cockpit-master/src/api/schema/tabs.rs`; `/tmp/herdr-cockpit-master/src/api/schema/panes.rs`).
- Herdr-managed variables override collisions and launch env applies to newly launched processes only (`/tmp/herdr-cockpit-master/docs/versions/0.8.2/website/src/content/docs/socket-api.mdx:298-302`). Existing processes cannot be retrofitted.
- `session.snapshot` exposes workspace IDs, worktree provenance, display tokens, and pane cwd/title/agent data, but no process environment or arbitrary context path (`/tmp/herdr-cockpit-master/src/api/schema/session.rs:8-22`; `src/api/schema/workspaces.rs:61-85`; `src/api/schema/panes.rs`).
- Worktree removal requires a Herdr workspace carrying linked-worktree membership. Plain repository workspaces are rejected as `not_linked_worktree` (`/tmp/herdr-cockpit-master/src/app/api/worktrees/deferred.rs:215-257`).
- Workspace IDs are generated public handles and restored IDs reserve their numeric range, but a fresh session can begin again at low IDs (`/tmp/herdr-cockpit-master/src/workspace.rs:104-110`, `151-173`; `/tmp/herdr-cockpit-master/src/persist/restore.rs:294-303`). A bare workspace ID is not a globally durable Cockpit identity.
- Herdr 0.8.2 has no native nonterminal plugin pane. Plugin panes are terminal panes or terminal popups (`/tmp/herdr-cockpit-master/docs/versions/0.8.2/website/src/content/docs/cli-reference.mdx:460-471`). Context presentation therefore remains a Cockpit surface.

The exact no-server-change sequence for a new worktree is:

1. Resolve and validate a required local repository plus typed task/artifact input in Cockpit.
2. Call Herdr `worktree.create` with source `cwd` or `workspace_id`, branch/base/path/label, explicit focus, and trust policy.
3. Wait for the authoritative `worktree_created` response/event and capture its returned workspace ID, checkout path, repository provenance, tab, and root pane.
4. Create or open the Cockpit companion context and atomically write its association manifest.
5. Create a new Herdr tab or split pane under the returned workspace with sanitized `COCKPIT_*` context variables in `env`; the worktree root pane created by Herdr cannot receive these variables retroactively.
6. Re-read `session.snapshot` and `worktree.list`, validate the returned provenance against the request and manifest, and return the assembled operation result.

For open, substitute `worktree.open` in step 2 and treat `already_open` as a normal result requiring reconciliation. For teardown, validate ownership and provenance before `worktree.remove`, then delete only the owned companion resource after Herdr confirms removal.

There is no automatic agent launch in this sequence. Cockpit may create a context-bearing tab/pane, but the developer starts an agent manually. Herdr/TUI-created terminals are outside Cockpit's env injection path and cannot be guaranteed to inherit `COCKPIT_CONTEXT_*` variables. Cockpit must never silently close the new root pane to hide this limitation.

## Shared contracts and invariants

FND-02 should expose typed operations rather than adding worktree cases to the existing generic layout mutation enum. Proposed protocol types:

```text
RepositoryRef {
  repository_key: string,
  repository_name: string,
  repository_root: absolute path,
  checkout_path: absolute path,
}

ArtifactRef {
  kind: issue | review | wiki | custom,
  canonical_id: nonempty string,
  url: optional canonical provider URL,
  title: optional string,
  source_revision: optional string,
}

WorktreeCreateRequest {
  repository: RepositoryRef,
  source_workspace_id: optional Herdr workspace ID,
  branch: optional validated branch name,
  base: optional ref,
  checkout_path: optional absolute path,
  label: optional bounded text,
  focus: boolean,
  trust_repository: boolean,
  artifact: optional ArtifactRef,
}

WorktreeOpenRequest {
  repository: RepositoryRef,
  source_workspace_id: optional Herdr workspace ID,
  checkout_path: optional absolute path,
  branch: optional branch name,
  label: optional bounded text,
  focus: boolean,
  trust_repository: boolean,
  artifact: optional ArtifactRef,
}

WorktreeResult {
  session_id: string,
  herdr_workspace_id: string,
  checkout_path: absolute path,
  repository: RepositoryRef,
  branch: optional string,
  already_open: boolean,
  companion_context: ContextAssociation,
  env_injection: new_processes_only,
}
```

The exact Rust/TypeScript names may be chosen by FND-02. The invariants are binding:

- `ArtifactRef` is optional. Repository-only setup has no remote source identity. When supplied, an artifact has a validated typed identity and a supported provider URL or canonical provider identifier.
- A local repository is required. Artifact URL alone never causes remote clone, provider mutation, or arbitrary path discovery.
- Repository roots, checkout paths, branch names, labels, env keys, and artifact fields are bounded and validated before any Herdr call.
- The result's Herdr workspace ID is captured from Herdr; Cockpit never predicts IDs or uses sidebar order.
- Cockpit association state is a companion manifest/cache keyed by `(Herdr session identity, canonical repository/worktree provenance, returned workspace ID)`. It is not a competing workspace registry and does not own Herdr lifecycle state.
- Every operation revalidates `session.snapshot` and `worktree.list` after mutation. A path collision, repository-key mismatch, branch mismatch, or workspace mismatch becomes an explicit conflict requiring resync/review.
- Herdr `workspace.close` and `worktree.remove` remain distinct operations. Close ends the Space and its processes but retains the checkout; remove additionally deletes the checkout and requires a separate exact-resource confirmation. Neither promises to restore terminated processes.
- Environment values are allowlisted, non-secret, and only advertise paths/IDs needed by Cockpit. Provider credentials never enter the manifest, snapshot, or generated env.

## LIFE-01: Repository catalog and artifact URL

### Goal

Offer setup from a known local repository and a typed artifact URL. The catalog supplies safe repository identity to later lifecycle calls; it does not clone, fetch, mutate remote providers, or infer an artifact from an arbitrary string.

### Inputs and behavior

Proposed core interface:

```text
RepositoryCatalog::list(configured_root) -> Vec<RepositoryCandidate>
RepositoryCatalog::inspect(path) -> RepositoryCandidate
SetupIntent { repository_key, artifact: optional ArtifactRef, requested_branch?, requested_label? }
```

`RepositoryCandidate` should contain canonical root, stable repository key derived from local Git remote/path policy, display name, current checkout path, current branch/detached state, and whether the path is a linked worktree. The catalog must reject paths outside the configured repository root unless an explicit one-off override is provided by FND-01 policy. It should use bounded directory traversal and Git CLI/library operations already accepted by the project; it must not execute user-provided shell text.

An artifact is optional. When a URL is supplied, require a syntactically valid absolute `http` or `https` URL and resolve a nonempty typed kind/id. The first provider adapter may recognize only Gitea issue URLs; unsupported hosts/kinds return `unsupported_artifact` while preserving the URL in diagnostic state. Do not silently reinterpret a review URL as an issue or strip query/fragment identity without recording the canonicalization.

### Implementation steps

1. Add repository and artifact value objects in the core/protocol contract package selected by FND-02/FND-03.
2. Add configured-root resolution and precedence in FND-01 (config file, environment override, one-off option), with no hard-coded home path.
3. Implement bounded local discovery and repository inspection.
4. Add a setup-intent validator requiring one local repository and validating an optional artifact.
5. Expose a task-level host operation that returns candidates and validation errors, not raw shell or filesystem access.

### Failures and recovery

- Missing/unreadable configured root: return `repository_root_unavailable` with path and remediation.
- No repository candidates: return an actionable empty result; do not create a Herdr workspace.
- Detached or linked source checkout: allow inspection but mark worktree create as requiring parent-source resolution.
- Invalid supplied URL, unsupported host/kind, or malformed supplied ID: reject before Herdr mutation.
- Repository identity collision (same key with different canonical roots): return `repository_identity_conflict`; require explicit selection and persist both roots in diagnostics.

### Tests and acceptance

- Unit tests cover root containment, symlink/canonical path handling, URL parsing/canonicalization, bounded fields, detached/link worktree classification, and repository-key collisions.
- Contract tests prove browser/native adapters expose identical candidates and error envelopes.
- Real acceptance uses a disposable local Git repository beneath a test catalog root and a representative issue URL. It verifies selected repository root/branch and exact artifact URL survive into the setup intent without network writes.

### Ownership and parallelism

FND-01 owns configuration precedence and root policy. FND-02 owns DTOs and host adapters. FND-03 owns identity normalization. LIFE-01 owns catalog implementation and validator. These can proceed in parallel after the value-object shape is frozen. Provider-specific URL interpretation is deferrable behind `ArtifactResolver`.

## LIFE-02: Reviewed create/open lifecycle and durable partial recovery

### Goal

Turn a validated setup intent into a Herdr worktree workspace, companion association, and a reviewable result while handling Herdr's asynchronous and reuse behavior. The workflow must survive a process restart or a failed step without inventing a second workspace registry.

### Create/open semantics

For create, Cockpit passes either `source_workspace_id` or source checkout `cwd` to Herdr, never both. The source workspace ID is a lookup anchor only. Herdr may return a new workspace or reuse an existing checkout; `already_open` is authoritative. For open, exactly one of checkout path or branch selects the target within the source repository. All trust and focus choices are explicit request fields.

The core orchestration should be an idempotent state machine with a durable operation record:

```text
requested -> validated -> herdr_requested -> worktree_ready
          -> workspace_verified -> companion_ready -> env_pane_ready
          -> completed
```

Error states retain the last successful checkpoint and returned Herdr identifiers. They do not roll back unrelated Herdr resources. If Herdr created a worktree but opening the workspace or writing the association fails, report a recoverable partial with checkout path/workspace ID and a resume/reconcile action.

### Implementation steps

1. Add typed `worktree.create/open` calls to `HerdrAdapter`, capability detection, request validation, and response parsing.
2. Add core orchestration around a single selected Herdr session. Check compatibility before mutation and reject missing worktree methods with `unsupported_capability`.
3. Resolve source workspace/cwd and repository provenance; reject a linked worktree as source unless Herdr's parent resolution returns the canonical parent.
4. Call Herdr and await the response/event completion; do not assume immediate synchronous completion.
5. Verify returned workspace, checkout path, repo key/root, branch, and `open_workspace_id` using a fresh snapshot/list.
6. Invoke CTX-01 companion creation and write the association manifest atomically.
7. Create a context-bearing tab or pane with a sanitized env map. Keep the Herdr-created root pane visible and untouched unless the user explicitly chooses a separate cleanup operation.
8. Persist completion/partial state and emit an operation result that names the exact env scope: Cockpit-created processes only.

### Failure matrix

| Failure | Required behavior |
|---|---|
| Herdr incompatible/missing lifecycle method | No filesystem or Herdr mutation; explicit unsupported capability. |
| Git branch/path collision | Return `worktree_create_failed` with Herdr/Git message; preserve no speculative association. |
| Herdr timeout after request | Mark `herdr_operation_unknown`; reconcile by `worktree.list` and snapshot before retry. Never blindly retry create. |
| Worktree exists and is already open | Reuse only if canonical path/repo provenance match; return `already_open=true`. |
| Worktree exists but belongs to another session/workspace | Return conflict and require explicit open/review; do not hijack or delete. |
| Companion creation fails after worktree success | Persist partial with Herdr workspace/checkout identity; offer retry companion or explicit cleanup. |
| Env tab/pane creation fails | Preserve root pane and companion; report `env_pane_failed`; allow retry. |
| Snapshot/list verification fails | Mark stale, retain operation record, resync before any destructive follow-up. |

### Durable recovery

FND-03 should define an append-safe operation/association record under the configured Cockpit state root. It stores operation ID, session identity, Herdr workspace ID, canonical repository/worktree provenance, branch, artifact identity/URL, companion path/ID, lifecycle checkpoint, and last error. It must not store credentials or full remote payloads.

On startup, core scans incomplete records, validates path containment and artifact shape, then reconciles each against the selected Herdr session's snapshot/worktree list. It may transition a record to `completed`, `orphaned_companion`, `orphaned_worktree`, `foreign_resource`, or `needs_review`; it must not auto-delete an ambiguous resource.

### Tests and real acceptance

- Adapter fixtures cover exact Herdr request/response/event shapes, already-open reuse, new workspace return, missing method capability, malformed provenance, and async timeout.
- Core tests cover state transitions, idempotency keys, retry after each checkpoint, collision handling, and restart reconciliation.
- Host contract tests cover equivalent HTTP/Tauri operation and error envelopes.
- Real acceptance uses a disposable named Herdr session and temporary Git repository. It creates a branch worktree, verifies the returned checkout/workspace in `session.snapshot` and `worktree.list`, repeats open to prove reuse, injects a failure after worktree success, restarts Cockpit, reconciles, and cleans only resources created by the test.

### Ownership and deferrable work

FND-02 owns Herdr wire calls; FND-03 owns durable operation records; CTX-01 owns companion creation. LIFE-02 owns orchestration and reconciliation. Provider hydration, automatic branch naming from remote artifacts, and UI wizard polish are deferrable after the create/open API and recovery contract are proven.

## LIFE-03: Companion association and environment after worktree

### Goal

Associate one Cockpit companion context with the verified Herdr workspace and make it discoverable to processes Cockpit launches, while stating the exact inheritance boundary.

### Association contract

CTX-01 provides a companion resource identity/path and atomic manifest write. LIFE-03 writes a manifest containing:

```text
schema_version
cockpit_operation_id
herdr_session_identity
herdr_workspace_id
repository_key/root/checkout_path
artifact kind/id/url
created_at / updated_at
ownership = cockpit
```

The manifest is an association document, not a registry of Herdr workspaces. Herdr remains authoritative for whether the workspace exists and what checkout it owns. Every read validates the manifest against fresh Herdr provenance. A mismatch creates `association_conflict` and prevents env injection until reviewed.

### Env sequence and limits

Because worktree create/open cannot carry env, LIFE-03 should use this sequence:

1. Verify the returned Herdr workspace and companion manifest.
2. Build an allowlisted env map such as `COCKPIT_CONTEXT_PATH`, `COCKPIT_WORKSPACE_ID`, `COCKPIT_REPOSITORY_KEY`, and `COCKPIT_ARTIFACT_URL` (subject to FND-01 naming policy).
3. Call Herdr `tab.create` or `pane.split` under the returned workspace with that env map and explicit focus behavior.
4. Record `env_injection = cockpit_created_processes_only` in the operation result.

The initial Herdr root pane remains without these context values. Cockpit must not claim that the root pane, a pane created in Herdr's TUI, or an existing terminal inherited them. There is no automatic agent launch. A future explicit “create context terminal” action may create another Cockpit-bearing pane, but that is separate from the graphical context renderer.

### Failures and security

- Context path outside configured companion root: reject and retain Herdr workspace.
- Manifest write interrupted: use temp-file-plus-rename and mark association pending until reread succeeds.
- Env key collision with Herdr-managed variables: reject or let Herdr authority win; report the final effective scope. Never try to override `HERDR_*` identity values.
- Artifact URL or path contains secrets: redact diagnostics and refuse secret-bearing env fields according to FND-01 policy.
- Existing workspace has a foreign association: stop with `association_conflict`; do not overwrite.
- Context creation fails: leave worktree and root pane visible, preserve partial record, offer retry or explicit cleanup.

### Tests and acceptance

- Unit tests cover manifest schema, atomic replacement, ownership, provenance mismatch, path traversal/symlink escape, redaction, env allowlist, and Herdr-variable collision.
- Herdr adapter tests assert `tab.create`/`pane.split` carry only approved env and never mutate the worktree operation request.
- Real acceptance launches a harmless shell in the Cockpit-created context pane and verifies exact `COCKPIT_*` values. It also creates a separate Herdr/TUI terminal and verifies the acceptance report explicitly marks inheritance as unavailable rather than making a false assertion.

### Ownership and deferrable work

CTX-01 owns file tree creation, reads, and watcher hooks. LIFE-03 owns association validation and env orchestration. UI context rendering, search, hydration, and provider payloads are deferrable. Herdr server/config changes are out of scope.

## LIFE-04: Teardown, borrowed checkout, and orphan recovery

### Goal

Make cleanup precise: closing a Space is not deleting a checkout; only Cockpit-owned resources may be removed automatically; borrowed or ambiguous worktrees require explicit user action.

### Teardown modes

1. **Close Space:** call Herdr `workspace.close`. Keep checkout and companion association. This is the default reversible UI action.
2. **Remove owned worktree:** verify linked-worktree membership, canonical path, repository key/root, session identity, and manifest ownership; confirm with the user; call Herdr `worktree.remove` with explicit `force` and trust policy; after authoritative success, remove the Cockpit-owned companion resource and mark the operation closed.
3. **Borrowed checkout:** a worktree opened from an existing checkout without Cockpit ownership may be closed from the session but must not be deleted by automatic cleanup. Provide “forget association/close only”; borrowed checkout deletion is outside the current removal flow. A future explicit ownership-adoption story would need its own reviewed policy before deletion becomes available.
4. **Orphan recovery:** if the manifest remains but Herdr no longer reports the workspace, verify canonical checkout and repository provenance. Offer to retain/read the companion, reattach by opening the checkout, or delete it after explicit confirmation. If the checkout remains but workspace is absent, offer open/reconcile; never silently create a duplicate.

### Implementation steps

1. Add typed `worktree.remove` with capability gate, target validation, force/trust fields, and structured response parsing.
2. Add an ownership classifier: `owned_created`, `borrowed_opened`, `foreign`, `unknown`, based on operation record plus fresh provenance.
3. Separate close and remove commands/routes/UI intents in protocol and audit logs.
4. Before removal, resnapshot and list worktree, then inspect local Git status including untracked files through a bounded read-only operation. `worktree.list` does not expose dirty status. Reject stale/mismatched IDs, foreign associations, non-linked workspaces, dirty/unknown state, or in-progress operations.
5. Call Herdr remove, await `worktree_removed`, then remove only the companion path recorded as Cockpit-owned. If companion deletion fails, record `orphaned_companion` and offer retry; do not retry destructive Herdr removal.
6. Reconcile leftovers on startup and after session changes. Preserve evidence for foreign/ambiguous paths.

### Failure matrix

| Failure | Required behavior |
|---|---|
| User cancels confirmation | No operation; retain resources. |
| Target is plain workspace | Refuse with `not_linked_worktree`; close remains available. |
| Borrowed/foreign checkout | Close/forget association only; no ownership override in this flow. |
| Herdr reports dirty/active/conflict | Surface exact state; do not force automatically. |
| Remove response times out | Mark unknown, reconcile with `worktree.list`; never issue a second remove blindly. |
| Herdr removed checkout but context cleanup fails | Mark orphaned companion and provide bounded cleanup; no rollback claim. |
| Manifest missing but checkout/workspace remains | Mark `orphaned_worktree_association`; require re-association or explicit cleanup. |
| Workspace ID reused in a fresh session | Session/provenance pair prevents accidental match; require fresh snapshot validation. |

### Tests and acceptance

- Unit tests distinguish close from remove, ownership classes, borrowed checkout, path/provenance collision, unknown timeout, idempotent reconciliation, and orphan states.
- Adapter fixtures cover remove success, `not_linked_worktree`, dirty/force errors, and worktree removed events.
- Real acceptance creates one owned linked worktree and one borrowed/opened checkout in a disposable named session. It closes both and confirms both checkouts remain, explicitly reopens the owned checkout through Herdr to obtain valid removal provenance, then removes only that owned worktree after explicit confirmation, verifies the borrowed checkout remains, then exercises orphan association recovery after deleting/renaming only test metadata.

### Ownership and deferrable work

FND-03 owns operation/association durability; LIFE-04 owns classifier, teardown orchestration, and recovery. Background cleanup scheduling and provider-specific archive policy are deferrable. Dirty/untracked-state inspection and explicit process/data-loss confirmation are required. No broad filesystem deletion, Herdr server modification, or automatic force removal is permitted.

## Cross-story delivery and review gates

The integration owner should land the stories in this order: FND-01/FND-02/FND-03 contracts, CTX-01 companion store, LIFE-01 catalog, LIFE-02 create/open, LIFE-03 env-bearing pane sequence, then LIFE-04 teardown/recovery. LIFE-01 can be developed alongside CTX-01; LIFE-02 can begin once FND-02 and FND-03 types are frozen; LIFE-03 and LIFE-04 depend on LIFE-02's returned identity and ownership record.

Each increment should have a narrow reviewable commit and preserve unrelated worktree state. Before declaring the setup slice complete, the integration owner must verify:

- protocol Rust source and generated TypeScript agree;
- Herdr capability discovery distinguishes unsupported lifecycle methods;
- no operation predicts/reuses a workspace ID without fresh provenance;
- optional artifact URL, when supplied, and required local repository are preserved end to end;
- partial records recover after process restart;
- root-pane env limitation is visible in the result and documentation;
- Cockpit-created env-bearing panes work while Herdr/TUI-created terminals are explicitly outside guarantee;
- no agent is auto-launched and no root pane is silently closed;
- close, remove, borrowed, foreign, and orphan paths are distinct;
- real disposable Herdr/Git acceptance proves lifecycle and cleanup behavior.
