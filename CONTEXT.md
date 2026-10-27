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

Project setup is plan-driven: repository discovery, artifact validation, Herdr creation with its default root terminal, exact ownership receipt, and optional shared Library save/selection. Open/Create adds no context tab or companion folder. Opening an existing directory borrows that exact path; it does not establish teardown ownership. Setup operation identity and creation receipts connect uncertain dispatch to reconciliation. The initial terminal is not retrofitted with Cockpit context environment; `cockpit-cli context --current` discovers the Space's Library selection and existing paths.

| Store | Meaning and lifetime |
| --- | --- |
| Project operation records | Setup/teardown intent, exact resource receipts and ownership proof. |
| Global Context Library | Durable provider snapshots and explicit folder captures, independent of workspace lifetime. |
| Space context selections | Relevant Library IDs and configured existing repository paths, not copied or pinned files. |
| Space tags | Cockpit-owned arbitrary organization labels; durable across Herdr restart, removed from a Space after confirmed closure, with a reusable catalog. |
| Agent attention | Cockpit-owned completion acknowledgements, shared by browser and native clients and retained across reload and restart; distinct from Herdr agent state. |
| Space Notes | Durable UUID-scoped Markdown, separate from Library and checkouts. |
| Supervisor task Markdown | Canonical task title, description, relationships, checklist and checked state, outside checkouts; current task/document revisions fence scope changes. |
| Orchestration machine document | Jobs, attachments, operation/effect journals, messages, source outcomes, integration and cleanup receipts, not live Herdr status or another task store. |
| Runtime pane state | Browser profiles/work, viewer comments/paste receipts and Review caches, cleared by their owning-runtime lifecycle. |
| Widget memory | Trusted content, selections and user-removal tombstones for one runtime run. |

Agents read selected Library paths and existing repository paths directly. Space relevance is not filesystem access control. A Library refresh becomes visible on the next read without a per-Space copy, update or follow phase. Removing a selection is distinct from removing content. Resource teardown and pane-state reset do not own Library, Notes or unrelated user files.

Notes presents Scratchpad, Todos, Kanban, Decisions and board-item comment threads as one local workarea. The board projects the same source Markdown as Todos rather than a second database. UUID-pinned content access can operate without Herdr; endpoint/session/Space bindings connect the UI to that durable identity. Editors retain scoped drafts and expose conflicts or unknown outcomes.

## Local surfaces

The sidebar defaults to Repository hierarchy and can switch to flat Tags groups; the choice is remembered locally. Tags group references to the same live Spaces, not copies or new Herdr resources. Compact 26 px Space/agent rows keep meaningful last-fetch Git counters visible on the name line in both views. Tagged Spaces add a measured line of whole text chips plus `+N`; confirmed selection wraps all tags and reveals secondary detail, without growth on hover, DOM focus or pending intent. The small picker, Space menu and Commands edit assignments. Tags are distinct from Herdr display labels and agent state; Spaces without agents use a small gray dot, while agents retain distinct state shapes, pinned tab context and inline Blocked/Done words. Space-scoped Git status/actions describe last-fetch knowledge rather than remote freshness. The workarea contains the selected Space's tab strip, selected tab canvas, and Cockpit-owned Library, Notes or Supervisor views; these views are not synthetic Herdr panes.

The Agents section is an attention queue: Blocked, Working and unseen completions remain visible, including completions Herdr has already reported as idle. Ordinary idle agents without an unseen completion are automatically excluded; new activity brings them back. Activating an agent row acknowledges its result only after Herdr confirms focus; marking seen is also explicit. Filtering does not close or stop a terminal.

Unseen results remain available as historical entries in the Agents list even if the agent disappears or is replaced. Mark a historical result seen explicitly; it does not focus a replacement agent.

During an attention-store outage, non-idle live agents remain visible with their Herdr state and a retry notice; ordinary idle agents are excluded because unseen state cannot be determined. The client bounds queued observations and prioritizes completion evidence. If that bound forces a discard, it warns that recovery may be incomplete until the warning is acknowledged; observations never received from Herdr cannot be reconstructed.

Every painted active-tab terminal is a direct control attachment rendered by xterm.js. Rendering and input permission remain separate. Herdr owns screen state, scrollback and PTY lifetime; resynchronization supplies a fresh baseline. Keyboard routing combines Cockpit's static prefix registry with identity-checked server-advertised commands. The singleton server popup floats above, rather than reshaping, the split tree.

**Ask an agent…** starts an agent in a dedicated tab in the selected Space without registering supervisor work. Copilot CLI and OpenCode receive the question in startup arguments; OMP, Pi, Claude Code and Codex CLI do so only when the question contains an ASCII space or line feed and does not begin with `@` or `-`. Startup questions are exposed in process arguments and Herdr layout metadata, and launch acknowledgment does not prove consumption. Otherwise, inspect the pane before explicitly sending a single-line question; multiline fallback requires manual paste. Uncertain outcomes require checking the tab rather than retrying blindly.

Files and Review are tab-local viewers opened from a real same-tab source terminal. Their core-issued binding pins the requested root and source identity; subsequent source cwd or lifetime changes do not retarget it. Files renders bounded source, Markdown/Mermaid and safe raster media with original line mapping. Review is a read-only local Git model. Run-local comments retain real paths and side-aware anchors for explicit same-tab paste without submission.

Browser is one independent managed Chromium association per Herdr tab, not a Space sidecar. Hidden views release capture resources. Fresh opening, explicit closure, retirement and owner shutdown manage its disposable profile and association work through identity-proven cleanup. Current-run failures remain visible and retryable; this is not retained navigation, credentials or saved-work recovery.

Widgets are trusted run-local HTML or declarative choices in an active opaque iframe dock, not Browser sessions or durable context. Publication does not select a terminal or grant execution authority. A narrow incarnation-bound bridge returns untrusted selection JSON to an agent pull/wait channel. Script execution and normal network access remain possible; the iframe is not CPU, memory, network or process isolation.

The existing bottom strip presents read-only OMP-owned Codex, Claude and Copilot usage. A shared private cache and cross-process schedule connect native and browser hosts without independent sign-in or GitHub quota collection. Anonymous allowlisted values retain source freshness and distinct unavailable, unknown, zero and unlimited states.

## Supervisor lifecycle and authority

The liaison's real Herdr terminal is the primary supervisor conversation. A compact read-only task rail beside it opens per-task recorded history without leaving the conversation. The rail keeps the same placement when all tasks are done and shows the completed list directly; an empty task board hides it. Optional fullscreen Overview keeps the task board or prerequisite graph visible beside the selected task's shared history; narrow views stack the detail below. Close/Escape restores the originating task's focus, then Escape returns to the conversation. Human decisions happen through the liaison, not worker-answer, task-editing, diagnostics or operational retry forms. Unmanaged agents remain legitimate; harness-internal children are not Cockpit ownership or control surfaces.

The liaison finishes a batch conversationally and remains Active/available for later tasks on the same project root. `run done` is worker-only; a low-level root Result is diagnostic history, not root retirement or a reason to create another liaison.

Tasks outlive execution attempts. Canonical Markdown holds scope, dependencies and checklist stages; machine state holds schedules, attempts, generations/credentials and receipts. Lanes are Blocked, Ready, Working, Review and Accepted (UI Done); Starting is an attempt badge. Checked prerequisites satisfy canonical dependencies, but same-project/repository accepted Code inputs must also exist in the frozen successor base before launch; otherwise scheduling stays queued with a concrete liaison hold. NonCode dependency artifact references remain accessible and preserved. Independent runnable work uses bounded slots without blocked workers or per-step model approval.

A root- or local-operator-authorized start or schedule binds the actual project target, canonical scope and placement. Source Space/context, branch/dirty state and concurrent work determine placement, not liaison cwd or labels; several Spaces may legitimately share a checkout. The first credentialed context call atomically claims an authorized pending worker generation. Private active attempt credentials, run/root/attachment epoch and task generation fence autonomous agents; optional actual conversation metadata is validated when supplied. Explicit CLI `--operator` management names its targets without requiring job ownership or accepting stale ambient credentials as authority. Fresh physical evidence separately fences launch, terminal effects and cleanup.

Orchestration placement prepares owned Git checkouts filesystem-only in the existing operation journal, or borrows an exact directory without Git/mkdir/deletion ownership. New-Space placement creates one waiting direct terminal, moves that same terminal and binds fresh exact single-tab/pane/cwd receipts before final placement environment releases the harness; ExistingSpace launches in place without a move. There is no extra setup/context shell or context tab. Saved source Library IDs/repository paths select existing paths without copies/companions. Unknown Git creation without proof is preserved, not adopted/replayed. Source Space/focus and ordinary user Open/Create setup remain unchanged.

Atomic replacement fences/supersedes the expected attempt, revokes credentials/launch authorization and preserves task/history plus old dirty work, terminals and claims. Typed source evidence selects either an isolated Git worktree at committed HEAD or a private no-copy Directory attempt under `state_root/orchestration/attempts/<run_id>` retaining the original project/context. `COCKPIT_SOURCE_PATH` points to the original Directory root, read-only by instruction rather than sandbox enforcement; outputs belong in the attempt directory. Same-checkout reuse requires exact writer-exit proof, registered consumer proof and fresh Herdr inventory—not main-process exit or universal absence. Nonempty attempt outputs are never discarded by acceptance/NotApplicable; only an exact accepted drained retired empty private directory can retire.

Literal harness command/argv carries a short initial prompt at launch. Core task/schedule/replacement/credential/messaging/cleanup contracts are harness-neutral; no OMP authentication export or session file grants product authority. Herdr-supported agent interaction, credential CLI and optional harness hooks are adapter choices. OMP's per-launch thin public-API hook is one optional adapter, not a restriction on Claude Code, Copilot, Codex or other configured harnesses. Extension-free CLI wait/pull remains complete. `inbox next` captures returned messages internally, without a separate ACK model turn; capture is not an Answer or model processing proof. The liaison reviews exact Result/current-scope SourceOutput, handles worker questions and records target integration separately from source acceptance.

Scheduling, inbox waits and safe cleanup wake on durable filesystem changes, in-process notifications, relevant Herdr events and explicit pending-operation deadlines. Watches register before reads, coalesce wakes and recover from notification gaps/backend errors; idle passes do not repeatedly rebuild full snapshots. Launch acknowledgement, observed physical readiness, credential activation and the first model response are distinct observations. Under-ten-second launch and under-five-second communication are requirements, not verified latency claims.

Orchestration-only pane invalidations exclude changes confined to terminal titles, revision and scroll state; ordinary UI subscriptions still carry live title updates. Pane/terminal identity, working directories, agent/status fields, tokens, labels and unknown metadata remain relevant, as do lifecycle events and reconnects.

Stable principal/operation identity, exact payload/source fences and effect journals support restart and no-blind-replay handling. Scarce build resources retain FIFO-ready queues and exact input/toolchain fingerprints; release needs known process exit, never a PID lease timeout. Automatic cleanup retains dirty, unmerged, borrowed, unknown or unproven resources. Accepted idle owned panes retire under exact fresh proof, with targeted retries for transient foreground-process holds. Clean owned accepted Git output already contained in the root's actual checkout can be removed non-force without a prior integration receipt; this does not mark integration tested or complete. Accepted retired worker scope stop does not wait for target integration. Explicit manual removal of a selected linked worktree uses confirmation, including discarding reviewed dirty paths, rather than historical setup ownership. A vanished root PID is not consumer closure proof.

New DirectProcess scopes are harness-neutral on supported Linux/cgroup-v2 hosts; exact InvocationID/cgroup/member receipts never retrofit old launches. Accepted-worker stop requires retired exact pane, current canonical completion/code-or-noncode proof and no Held/Queued holder. Separately, committed WorkerReplace authority may retire an exact old idle Superseded pane and stop only its freshly verified old scope with no open descendants/resources. Foreign checkout-cwd panes do not block stopping that isolated scope, but do block claim release/deletion proof; old files/worktree/branch/attempt outputs remain Kept unconditionally even after release. Roots/cancelled/arbitrary scopes never stop. Escaped D-Bus/Docker/self-migrated consumers need separate proof—not universal containment.

## Scope and evidence

Errors remain inline at their affected resource with last-known state and supported recovery, leaving unrelated resources usable. Keyboard operation, visible focus, meaningful labels and non-color-only states are best-effort goals, not formal accessibility acceptance for broader distribution.

Settings use supported file/environment/invocation sources rather than a settings UI. Global harness configuration/authentication, provider sign-in, remote access, an agent-history browser, a separate inbox popup, provider writes, arbitrary user-entered shell execution, raw socket forwarding and unbounded previews remain outside the current contract. Server-advertised configured commands, per-process supervisor/worker launch, OMP's built-in same-conversation resume and explicitly configured resume for other harnesses are supported. The session-first cutover has no legacy orchestration migration or compatibility aliases; unsupported store schemas fail without discarding their bytes.

Development and disposable smoke recipes are in [CODE_GUIDE](CODE_GUIDE.md); installation is in [native installation](docs/native-install.md). Historical live observations belong to the [verification log](docs/verification-log.md), never an inference of current all-platform or all-provider acceptance.
