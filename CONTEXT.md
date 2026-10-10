# Cockpit architecture context

Cockpit is a personal, local-first developer workbench for supervising persistent coding-agent sessions and organizing context for bounded engineering tasks. It targets one developer on a trusted workstation, not a remote or multi-tenant service. Maintainability and easy behavior changes take precedence over distribution generality.

This is the mental model, not an acceptance report. [DECISIONS](DECISIONS.md) holds current rules, [CODE_GUIDE](CODE_GUIDE.md) maps implementation and development flows, [configuration](docs/configuration.md) owns settings and examples, and the [verification log](docs/verification-log.md) preserves historical observations and limitations. [Supervisor surfaces](docs/supervisor-surfaces.md) maps the observational UI.

## Product and domains

A typical workflow discovers a configured local repository, creates an owned task worktree or opens an existing directory, selects relevant Library items and existing repository paths for a Space, and works through Herdr terminals with local viewers. Inspection, testing, review and separately authorized resource retirement complete the workflow.

- **Forge:** repositories, branches, review material and worktree provenance.
- **Issue Tracker:** issues, comments, labels, relationships, references and freshness.
- **Wiki:** static page and documentation context.
- **Telemetry:** optional static log and trace context.
- **Herdr Client:** session, hierarchy, terminal and agent interaction under Herdr authority.

Provider adapters serve these domain contracts. Implemented read slices include GitHub issues/pull requests, GitLab, Gitea through Tea, Jira issues/query follows and Confluence pages/space follows; these are not a complete cross-provider feature matrix. Remote writes remain with the user's provider tools. Local-folder Library capture is a separate explicit snapshot workflow.

## Shared runtime

The Rust core owns application rules and durable state independently of transport framing or GUI hosting. Hosts compose narrow adapters rather than placing business rules in Tauri handlers.

| Layer | Responsibility |
| --- | --- |
| Protocol | Versioned request, response, error and event types; Rust-derived frontend contracts. |
| Application core | Project setup, ownership, task/run orchestration, Library, Notes, source authorization and idempotency. |
| Herdr adapter | Supported schema/socket protocol, snapshots, ordered events, attachment and Herdr identifiers. |
| Provider adapters | Configured HTTP/CLI reads and canonical normalized snapshots. |
| Vault adapter | OS credential storage behind the core credential-vault trait. |
| Hosts | CLI, explicit foreground browser gateway and native application composition. |
| Client adapters | Shared typed requests, ordered streams and boundary decoding. |

The shared frontend runs in a Tauri v2 native host or through `cockpit serve` at a loopback origin. Native requests use IPC and ordered channels; browser requests use HTTP and ordered WebSocket streams. UI components consume one `CockpitClient` contract. Tauri is a native host, not a general-purpose web server; remote access and multi-user authorization are outside the current product.

The Herdr socket carries persistent newline-delimited JSON interaction and subscriptions. Its CLI remains useful for bootstrap, documented wrappers and human debugging. A selected session supplies an authoritative snapshot followed by ordered events; switching sessions replaces subscriptions and stale selection state. Multiple windows still share Herdr's focus and writable-control authority.

## Ownership: membership versus placement

**Herdr owns live existence and membership:** named sessions, Spaces/workspaces, tabs, real terminals, PTYs, processes, focus identity and agent state. Cockpit's session mirror is a cache of that state, not another lifecycle registry. Display names and labels are presentation, not resource identity.

**Cockpit owns placement and presentation:** a run-local split tree in each tab arranges real terminals, Files/Review/Browser leaves and widget docks. This in-memory layout is keyed by session, server instance and tab ID. Local dragging, resizing, swapping and zooming do not change Herdr geometry; fitting a control-attached terminal still changes its PTY grid.

Local selected leaf, Herdr focus, attachment/input ownership and DOM keyboard focus are distinct. A viewer can be selected while Herdr remains focused on the last real terminal. Fresh changed external focus is reconciled into local selection; an unchanged snapshot does not steal selection from a viewer.

Only confirmed authoritative tab or final-real-terminal loss retires a layout and its local viewer/browser/widget resources. Loading, stale and disconnected observations do not prove loss. Hiding a tab, zoomed leaf or the Library releases renderer/capture resources without ending Herdr processes or the hidden Browser session. Lifetime and uncertainty rules are in [DECISIONS](DECISIONS.md).

## Projects and durable stores

Project setup is plan-driven: repository discovery, artifact validation, Herdr creation, exact ownership receipt, optional Library save/selection, and context-aware subsequent terminals. Opening an existing directory borrows that exact path; it does not establish teardown ownership. Setup operation identity and creation receipts connect uncertain dispatch to reconciliation.

| Store | Meaning and lifetime |
| --- | --- |
| Project operation records | Setup/teardown intent, exact resource receipts and ownership proof. |
| Global Context Library | Durable provider snapshots and explicit folder captures, independent of workspace lifetime. |
| Space context selections | Relevant Library IDs and configured existing repository paths, not copied or pinned files. |
| Space Notes | Durable UUID-scoped Markdown, separate from Library and checkouts. |
| Supervisor task Markdown | Canonical task title, description, relationships, checklist and checked state, outside checkouts. |
| Orchestration machine document | Runs, plans, grants, messages, receipts and recoverable intents, not live Herdr status or another task store. |
| Runtime pane state | Browser profiles/work, viewer comments/paste receipts and Review caches, cleared by their owning-runtime lifecycle. |
| Widget memory | Trusted content, selections and user-removal tombstones for one runtime run. |

Agents read selected Library paths and existing repository paths directly. Space relevance is not filesystem access control. A Library refresh becomes visible on the next read without a per-Space copy, update or follow phase. Removing a selection is distinct from removing content. Resource teardown and pane-state reset do not own Library, Notes or unrelated user files.

Notes presents Scratchpad, Todos, Kanban, Decisions and board-item comment threads as one local workarea. The board projects the same source Markdown as Todos rather than a second database. UUID-pinned content access can operate without Herdr; endpoint/session/Space bindings connect the UI to that durable identity. Editors retain scoped drafts and expose conflicts or unknown outcomes.

## Local surfaces

The sidebar follows Herdr's Space hierarchy and agent attention ordering, with Space-scoped Git status/actions. Local status counters describe last-fetch knowledge rather than remote freshness. The workarea contains the selected Space's tab strip, selected tab canvas, and Cockpit-owned Library, Notes or Supervisor views; these views are not synthetic Herdr panes.

Every painted active-tab terminal is a direct control attachment rendered by xterm.js. Rendering and input permission remain separate. Herdr owns screen state, scrollback and PTY lifetime; resynchronization supplies a fresh baseline. Keyboard routing combines Cockpit's static prefix registry with identity-checked server-advertised commands. The singleton server popup floats above, rather than reshaping, the split tree.

Files and Review are tab-local viewers opened from a real same-tab source terminal. Their core-issued binding pins the requested root and source identity; subsequent source cwd or lifetime changes do not retarget it. Files renders bounded source, Markdown/Mermaid and safe raster media with original line mapping. Review is a read-only local Git model. Run-local comments retain real paths and side-aware anchors for explicit same-tab paste without submission.

Browser is one independent managed Chromium association per Herdr tab, not a Space sidecar. Hidden views release capture resources. Fresh opening, explicit closure, retirement and owner shutdown manage its disposable profile and association work through identity-proven cleanup. Current-run failures remain visible and retryable; this is not retained navigation, credentials or saved-work recovery.

Widgets are trusted run-local HTML or declarative choices in an active opaque iframe dock, not Browser sessions or durable context. Publication does not select a terminal or grant execution authority. A narrow incarnation-bound bridge returns untrusted selection JSON to an agent pull/wait channel. Script execution and normal network access remain possible; the iframe is not CPU, memory, network or process isolation.

The existing bottom strip presents read-only OMP-owned Codex, Claude and Copilot usage. A shared private cache and cross-process schedule connect native and browser hosts without independent sign-in or GitHub quota collection. Anonymous allowlisted values retain source freshness and distinct unavailable, unknown, zero and unlimited states.

## Supervisor lifecycle and authority

Supervisor is a Cockpit-owned observational workarea below the tab strip. Tasks is the default; Graph shows explicit run/subagent parentage, while Dependencies shows the separate canonical prerequisite DAG. Details distinguish durable reports and history from fresh Herdr observations. Internal native subagents have lifecycle telemetry but no separate Herdr pane; unmanaged agents remain legitimate.

A proposal names an explicit project target; it does not infer routing from cwd or the supervisor's Space. Read-only preparation inspects the source Space, selected context, branch/dirty state and concurrent plans. Disjoint work can share its checkout; conflicting or uncertain work uses an owned linked SpaceWorktree from that source's immutable HEAD unless an explicit base is provided.

Prepare and Execute are separate authority boundaries. A freshly bound active top-level Supervisor or Adopted root's actual main OMP session can authorize only strict-descendant Workers. Prepare binds the reviewed setup revision and permits setup/launch plus bounded read-only initialization. The worker's Ready receipt carries an exact work plan; Execute binds that reviewed revision. Changed plans require fresh review. Advanced operator intervention remains distinct.

Launch ACK is pending evidence, not readiness. Fresh actual OMP/process proof and the main SDK-session binding connect a run to its real resource incarnation. The per-process extension supplies preparation gates, native caller evidence and durable inbox pull/ACK, without global OMP configuration or authentication management. These same-UID accident-prevention controls are not an OS sandbox.

Progress, Ready, NeedsInput and Result are explicit durable reports. Result enters Review without checking canonical Markdown. Reviewed successful Result plus the current exact task revision permits acceptance; runtime idle/done or completed checklist does not. Acceptance preserves task, history, files and checkout. Worker retirement is separately fenced cooperative shutdown and managed-resource closure, not workspace teardown or evidence that every descendant stopped.

The private runtime owner dispatches and reconciles durable intent under per-run leases. Owner restart preserves orchestration records and Herdr processes. Unknown effects require exact-receipt reconciliation; recovery, cancellation and retirement have different authority and retention conditions. The detailed launch, native-child, external-edit and unknown-outcome fences remain in [DECISIONS](DECISIONS.md).

## Scope and evidence

Errors remain inline at their affected resource with last-known state and supported recovery, leaving unrelated resources usable. Keyboard operation, visible focus, meaningful labels and non-color-only states are best-effort goals, not formal accessibility acceptance for broader distribution.

Settings use supported file/environment/invocation sources rather than a settings UI. Global OMP configuration/authentication, provider sign-in, remote access, resumable agent history, a separate inbox popup, provider writes, arbitrary user-entered shell execution, raw socket forwarding and unbounded previews remain outside the current contract. Server-advertised configured commands and per-process supervisor/worker launch are supported.

Development and disposable smoke recipes are in [CODE_GUIDE](CODE_GUIDE.md); installation is in [native installation](docs/native-install.md). Historical live observations belong to the [verification log](docs/verification-log.md), never an inference of current all-platform or all-provider acceptance.
