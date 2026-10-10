# Cockpit decisions in force

Current rules are grouped by behavioral contract: conditions, limits and caveats of one rule share a bullet. [Architecture](CONTEXT.md), [code navigation](CODE_GUIDE.md), [configuration](docs/configuration.md) and [historical verification](docs/verification-log.md) have separate roles. Links locate applicable owners; procedural delivery/evidence policies and explicitly unverified requirements are not claims of runtime enforcement.

## Herdr authority & compatibility

- Herdr owns sessions, Spaces/workspaces, tabs, real-terminal existence/membership, PTYs, processes, focus identity and agent state; Cockpit caches this authority without a competing live registry, and Space context records relevance, not ownership. [Owner](crates/cockpit-herdr/src/cli.rs). Rationale: One live authority prevents divergent lifecycle state.

- Use supported socket methods for persistent state/streams; require protocol 22, schema 1 and every required method, failing closed on mismatch rather than using display-version patch allowlists. [Owner](crates/cockpit-herdr/src/schema.rs). Rationale: Capabilities come from the inspected schema.

- Command/popup discovery requires identity-checked generation-1 full surfaces and mandatory codecs; negotiate endpoint.surface-delta.v1 only when advertised with compatible bounded decoding, otherwise retain full surfaces. [Owner](crates/cockpit-herdr/src/shell_wire.rs). Rationale: An optional codec cannot weaken admission.

- Before accepting shell deltas, match boot/peer identity, baseline projection/surface revisions and dimensions; validate cells in reusable bounded scratch without retaining them. Popup patches require matching terminal/grid and corresponding cell updates; replacements establish new grids. [Owner](crates/cockpit-herdr/src/shell_wire.rs). Rationale: Metadata alone cannot authorize a popup.

- Keep unsupported, disconnected, timeout, malformed and identity errors explicit: shell_unsupported never permits guessed bindings, retired commands reject command_not_available, invalid targets command_target_invalid, and stale popup attachment popup_not_open. [Owner](crates/cockpit-herdr/src/cli/shell.rs). Rationale: Unavailable capability is not dispatch permission.

- Bootstrap from an authoritative snapshot and ordered events; resnapshot/rebind on gaps, identity changes, reconnect or stale state, and clear old subscriptions/renderers/selections on session switch. ACKs and later HTTP arrival do not supersede ordered focus/control state. [Owner](src/app/session/sessionStore.ts). Rationale: Arrival order is not state authority.

- Keep tab placement only in memory, keyed by session/server instance/tab: same-instance resync preserves it, changed instance starts fresh. Ignore Herdr rectangles/zoom hints; local drag/divider/swap/zoom sends no pane_resize/pane_swap/pane_zoom, though fitted control attachments resize PTYs. [Owner](src/app/layout/tabLayoutStore.ts). Rationale: Presentation is separate from terminal membership.

- First load balances stable pane-ID order; external terminals enter at the full-height right edge with share 1/(existing leaves+1), including zoom-hidden leaves. Place Cockpit-created terminals beside the acted-on leaf only from a validated creation receipt. [Owner](src/app/layout/splitTree.ts). Rationale: Focus or snapshot differences cannot attribute creation.

- Follow changed external Space/tab/pane focus unless it echoes current local intent, restoring zoom if needed; unchanged focus never steals viewer selection. Viewer selection sends no Herdr focus request; leaf selection, server focus, control ownership and DOM focus remain distinct. [Owner](src/app/layout/reconcile.ts). Rationale: Repeated observations must not override local intent.

- Only confirmed tab/final-real-terminal loss retires layouts and local viewers/Browser/widgets; loading, stale or disconnected state is not loss. Browser association work is discarded, while viewer batches remain run-local until owner reset. [Owner](src/app/layout/browserLifecycle.ts). Rationale: Uncertain observation grants no cleanup authority.

- Order agents blocked, done, working, idle, unknown, newest state change first; this is client presentation matching Herdr TUI priority, not API workspace order, agent_panel_sort or plugin agent views. [Owner](src/app/sidebar/Agents.tsx). Rationale: Presentation ordering must not impersonate server state.

## Terminal attachment & input

- Herdr owns PTYs, models and scrollback; xterm renders/captures input. Painted active-tab terminals attach control-only at fitted grid/cell pixels, never observe/downgrade; input additionally needs DOM focus, confirmed Herdr focus and owned control. [Owner](crates/cockpit-herdr/src/terminal_wire.rs). Rationale: Rendering alone cannot authorize input.

- Require a full first frame and consecutive subsequent sequences, including full repaints; never reset xterm for a full ANSI baseline. Reconnect needs a new baseline; one uninterrupted reader provides bounded buffering and deterministic shutdown. [Owner](crates/cockpit-herdr/src/terminal_wire.rs). Rationale: Continuous framing preserves terminal state.

- Tab switches prepare painted target attachments before Herdr focus, gated by the focused pane’s first frame or 300 ms, with no gate if none are painted; retain outgoing attachments until swap paint. Fresh membership, not geometry, determines eligibility; Herdr resizes on control attach and restores TUI size on disconnect. [Owner](src/app/session/focusCoordinator.ts). Rationale: The incoming PTY must fit before focus changes.

- Move DOM focus to the incoming selected terminal only after paint and confirmed control, never to its hidden attachment. Hidden tabs, zoom leaves and Library detach without stopping processes/scrollback; Files/Review state persists and hidden Browser releases captures without closing its session. [Owner](src/app/TerminalPane.tsx). Rationale: Visibility and process lifetime are independent.

- Use raw Input for ordinary text/binary bytes and bare LF for Shift+Enter; use AttachScroll for wheel/page and structured cell-coordinate AttachMouse only under MouseCapture demand. Never substitute raw SGR; mode-off/Shift-drag retain selection, without idle hover/pixel forwarding. [Owner](crates/cockpit-herdr/src/terminal_wire.rs). Rationale: Herdr chooses the application mouse encoding.

- Keep graphics parked: consume known auxiliary messages without exposing graphics payloads or disconnecting usable text terminals, and do not load the image addon. Local Kitty keyboard support is not end-to-end enhanced-reporting acceptance. [Owner](src/app/TerminalPane.tsx). Rationale: Unsupported imagery must not break text.

- Attach/resync/retry failures leave resources intact and visible with stale/disconnected status and recovery; inspect authoritative state before retrying uncertain mutations. Name evidence paths precisely: physical attach, xterm wheel, structured AttachMouse or CLI-injected SGR. [Owner](src/app/TerminalPane.tsx). Rationale: One input-path observation cannot prove another.

- Render the singleton popup centered over the unchanged split tree using server title/cell/percentage hints and validated direct popup transport. Keep painted underlay attachments, make it inert and gate input even during pending invocation; Esc/Enter/Tab/prefix keys belong to the program. [Owner](src/app/ServerPopup.tsx). Rationale: A popup must not reflow or control underlying panes.

- Only fresh popup metadata closes/replaces it; stale/disconnected state retains disabled input/status/retry. On closure prefer the eligible selected terminal, then an available opener, then tab chrome; delayed restoration must not steal focus or mutate Herdr focus. [Owner](src/app/ServerPopup.tsx). Rationale: Staleness is not closure.

- The active 120×40 shell subscription required for command/popup metadata can resize unattached panes and concurrent Herdr TUI; it is not passive or geometry-neutral. Direct attachments protect fitted Cockpit PTYs without changing local splits. [Owner](crates/cockpit-herdr/src/shell_wire.rs). Rationale: Document the chosen API’s real side effect.

## Keyboard

- Keep platform-independent Ctrl+B prefixes and Herdr-default action meanings or leave them unbound; Cockpit-only actions use free keys. Omit Herdr-only s/q/e/[/o/Shift+G/Shift+R; cycle Ctrl+B Tab/Shift+Tab in local visual order. Static registry drives Commands/tooltips/generated docs, not runtime plugin entries. [Owner](src/app/input/shortcuts.ts). Rationale: One registry keeps local discoverability accurate.

- Discover generic shell/pane/popup/plugin commands, opaque IDs, aliases and configured prefixes from identity-checked live projection; reload replaces retired bindings. Invoke only currently advertised actions against confirmed Herdr focus and revalidated membership, never uncertain-result replay. [Owner](src/app/input/herdrBindings.ts). Rationale: Advertisements are revocable dispatch authority.

- Outside editors/dialogs, direct custom bindings precede viewer keys; configured Herdr prefixes coexist with Ctrl+B, and shared Ctrl+B custom aliases win collisions. Hide shadowed local hints; reserve literal Ctrl+B Ctrl+B and Herdr-prefix double-prefix passthrough before custom lookup. [Owner](src/app/input/keymap.ts). Rationale: Collision policy must preserve literal program input.

- Terminal/inline Browser claims stay narrow: prefixes/custom bindings, clipboard Ctrl+Shift+C/V (Cmd+C/V on macOS) and Shift+Enter; unbound Tab/Shift+Tab/Alt/function/Ctrl keys reach content. Recognize Browser prefixes on its surface, not URL/note editors; popup routing yields entirely. [Owner](src/app/input/keymap.ts). Rationale: Workbench shortcuts must not trap content keys.

- Local modals own focus; delayed pane autofocus yields, and portalled inputs cannot select their Files pane through bubbling. Viewer handlers require DOM containment and yield to editable/modal focus. [Owner](src/app/input/modal.ts). Rationale: React ancestry is not keyboard ownership.

- Pane-scoped prefixes close Library first and act on the selected leaf of any kind: splits create terminals, focus/cycle/swap/zoom use local placement. Viewers cannot cross tabs/Spaces; Esc cancels drag or restores zoom only on chrome, otherwise belongs to content. [Owner](src/app/shell/useWorkbenchActions.ts). Rationale: Commands must respect the selected surface.

- Library open focuses selected/first tree row; close by any route restores a connected sidebar invoker, else an eligible reattached terminal or still-selected graphical origin, else safe chrome. Closing changes DOM focus only, never Herdr focus. [Owner](src/app/shell/useWorkbenchInput.ts). Rationale: View navigation must preserve explicit focus ownership.

## Workspace & filesystem ownership

- Setup is input-bound and plan-driven: Open accepts exact validated plain/nested directories, never initializes Git, switches branches, clones or requires repository selection; optional Git discovery is metadata only. Stale plans need a fresh start; provider links cannot choose a local repository. [Owner](crates/cockpit-core/src/projects/plan.rs). Rationale: Artifact identity is not checkout authority.

- Cockpit owns operation records, Library, Notes and context selections, not borrowed opened directories. Setup creates no companion copies; configured actions need no per-operation checkbox but never set Herdr trust_repository. Preserve operation identity across closed dialogs/uncertain dispatch and reconcile, not duplicate setup. [Owner](crates/cockpit-core/src/projects/execute.rs). Rationale: Ownership comes from exact receipts.

- Initial worktree/open panes and TUI-created terminals cannot be retrofitted with Cockpit context environment; pass supported variables explicitly to later Cockpit-created tabs/panes, report the limitation and never silently close the initial pane. Existing Herdr metadata stays inherited. [Owner](crates/cockpit-herdr/src/cli/projects.rs). Rationale: Unsupported environment injection cannot justify process replacement.

- Teardown needs explicit ownership, exact creation receipt and fresh linked-worktree/endpoint/clean checks; ambiguity refuses deletion. Preserve unrelated folders/files/user notes without importing, adopting or deleting them; Library and Notes survive workspace destruction. [Owner](crates/cockpit-core/src/projects/teardown.rs). Rationale: Removed migration code grants no new cleanup authority.

- Keep reads bounded/rooted/no-follow/identity-checked; revalidate roots and source identity before mutations/comments. Selected extra repositories must still belong to the configured catalog and pass fresh filesystem/Git checks; revocation preserves diagnosed selections, while fresh own-checkout authority is separate. [Owner](crates/cockpit-core/src/context.rs). Rationale: Selection is relevance, not access control.

- Pull fetches without mappings changing local branches, then fast-forwards only, never rebase/merge/autostash. Push uses explicit branch-to-existing-upstream refspec, never force/mirror/upstream setup; preserve normal credentials/URL aliases and never expose remote URLs in status/diagnostics. [Owner](crates/cockpit-core/src/space_git_action.rs). Rationale: Status actions must not rewrite Git intent.

- Row actions target their explicit Space without selecting it; Commands targets selected Space/worktree. After reservation, recheck fresh Herdr checkout, filesystem/Git identity, branch/upstream/effective destination; pane-folder status gives no write authority and counters mean last fetch. [Owner](crates/cockpit-core/src/space_git_action.rs). Rationale: Queued work must not silently retarget.

- Confirmed Git outcomes refresh status and show brief row-local success; refusal/not-run/unknown remains inline, echoed elsewhere only if offscreen. Unknown keeps Pull/Push disabled until dismissed; inspect ambiguous spawned/transport failures, never retry or claim no change. [Owner](src/app/sidebar/SpaceGitAction.tsx). Rationale: Uncertainty must remain actionable and honest.

## Supervisor orchestration

- Supervisor is a local view below tabs, not a Herdr resource. Start defaults to a fresh current-Space OMP tab without terminal-focus change; Existing Space/Directory/Dedicated folder are secondary options. Give work in its terminal, without bottom composer/resize or unmanaged overview/filter. [Owner](src/app/supervisor/SupervisorView.tsx). Rationale: Presentation must not invent lifecycle resources.

- Canonical task title/description/relationships/checklist/check live only in orchestration/tasks/<root_id>.md with stable UUID markers; Task.body is read-only full continuation. Edits splice description/core-owned spans while preserving protected metadata, interleaved prose and neighbors; unmarked/duplicate/ambiguous source needs diagnosis, not guessed mutation. [Owner](crates/cockpit-core/src/orchestration/tasks_md.rs). Rationale: One Markdown authority prevents copied task stores.

- TaskAssign journals one stable UUID, canonical item and root-inbox pointer across documents; submitted title/description persist only until both commit. Authoring alone is not assignment; exact-byte task/document revisions fence writes and recovery preserves external edits without rollback. [Owner](crates/cockpit-core/src/orchestration/assignments.rs). Rationale: Cross-document intent must be recoverable.

- Task lanes derive checked state/current run; UI Preparing/Done project setup/accepted and blocked is attention, not stored state. Runtime Done, successful Result or 100% steps never checks/accepts a task. Multiple Supervisor/Adopted roots and unmanaged agents are valid; parentage is independent of Space. [Owner](crates/cockpit-core/src/orchestration/projection.rs). Rationale: Runtime and canonical completion are different facts.

- First-continuation metadata permits at most 32 raw depends_on edges and one immutable follow_up_of. Only canonically checked prerequisites satisfy work; fresh Prepare/Execute/dispatch/new effects/Accept recheck the affected graph, blocking malformed/missing/duplicate/cyclic components while healthy independent work stays runnable. [Owner](crates/cockpit-core/src/orchestration/dependencies.rs). Rationale: Prerequisites cannot be inferred from runtime success.

- Missing follow-up source is a nonblocking provenance warning, never SDK revocation. Relationship writes fence task and document revisions; an open attempt permits only strict removal of positively parsed edges. Same-UUID/same-payload create replay is inert, changed payload conflicts. [Owner](crates/cockpit-core/src/orchestration/dependencies.rs). Rationale: Provenance and runnable dependencies have different authority.

- Stable step UUIDs identify managed tails or already tracked externally edited forests; leaf checks derive Open/Partial/Done and progress. Explicit subtree checks/moves are atomic and preserve subtree IDs/order without accepting tasks; removing last child keeps Done/Open and turns Partial Open. [Owner](crates/cockpit-core/src/orchestration/steps.rs). Rationale: Checklist progress is a projection, not acceptance.

- Checklist limits are 64 rows, depths 0–4, 200 Unicode title scalars, 16 KiB continuation and 8 MiB document. Numeric overflow stays readable with safe checks/shrinking; malformed identities/structure and ambiguous unmarked protected source stay read-only with unavailable progress. [Owner](crates/cockpit-core/src/orchestration/steps.rs). Rationale: Bounded authoring must preserve external Markdown.

- Only the exact current executed worker main and genuinely registered live native task child may author its Working task; actual child session/type/registry liveness/main binding/process proof matter, not labels/telemetry/descriptors. Root/operator management is for non-Working unchecked work; accepted tasks are read-only. [Owner](crates/cockpit-core/src/orchestration/caller.rs). Rationale: Live task ownership cannot be forged by parent labels.

- Retain task/root-scoped drafts and unknown-operation scope/UUID/payload/revisions until explicit saved-state resolution. Use trusted native/local keyboard checks and explicit subtree/move/remove previews; saved moves reveal ancestry and restore UUID only after matching revision render, never stealing newer navigation. [Owner](src/app/supervisor/useStepRecovery.ts). Rationale: Delayed saves must not discard drafts or navigation.

- An actual main session of a freshly bound active top-level Supervisor/Adopted root may authorize only strict-descendant Workers; no worker/subagent self-grants or sibling/ancestor/other-root control. Advanced operator origin remains separate, and widget selections grant nothing. [Owner](crates/cockpit-core/src/orchestration/mutate/grants.rs). Rationale: Authority follows verified actor and subtree.

- Proposals return without waiting; single-use Prepare binds the reviewed setup revision for setup/real-tab launch and bounded read-only initialization. Ready retains the initialization receipt and exact work plan; Execute requires its reviewed exact revision, with fresh review after changes and no routine operator confirmation. [Owner](crates/cockpit-core/src/orchestration/mutate/grants.rs). Rationale: Preparation permission is not coding permission.

- Route explicit repository/Open/Space proposals from configured or actual forge-origin evidence, never cwd/current Space. Read source Space/context/branch/dirty/concurrent plans first; share disjoint checkout work, use owned SpaceWorktree for overlap/uncertainty, preserving project_workspace_id and direct context paths. [Owner](crates/cockpit-core/src/orchestration/routing.rs). Rationale: Placement must bind the real source project.

- SpaceWorktree defaults to the source checkout’s immutable HEAD, not primary-checkout branch, unless base is explicit; fresh Git/configured-repository/Herdr proof binds it. Refuse duplicate generic project Open/Create and supersede later conflicts rather than retarget launched workers. [Owner](crates/cockpit-core/src/orchestration/routing.rs). Rationale: A launched process must not change checkout identity.

- agent.start ACK is pending only; Active/Launched needs fresh actual nonpending OMP and main SDK binding. Working OMP is valid without interactive-ready or a display alias; exact endpoint/location/terminal and native-session or live foreground PID/start/boot proof fence conflicting evidence. [Owner](crates/cockpit-core/src/orchestration/dispatch.rs). Rationale: Names are mutable display facts, not launch identity.

- Launch through typed Herdr tab/agent methods with per-process -e, no shell typing/prompt injection/global settings/auth management. Before Execute the extension blocks mutation/shell/eval/delegation using fresh per-tool authority; same-UID filesystem/socket access remains broad, not an OS/hostile-agent sandbox. [Owner](integrations/omp/extension.ts). Rationale: Policy gates prevent accidents without promising isolation.

- Use the launch-selected CLI/configuration and fresh native caller evidence; Host may use itself, installed Native cockpit must select sibling cockpit-cli, and debug cockpit-tauri may use distinct cockpit host. No installed-pair override is needed; direct CLI use requires matching executable/config/caller. [Owner](integrations/omp/cliCall.ts). Rationale: An ambient old binary cannot supply current authority.

- Caller checks use fresh original-endpoint/membership/native identity at pre/core/post fences, not TTL/global observation caches. Timed-out empty reads fence authority outside the read budget and never ACK mail. [Owner](crates/cockpit-core/src/orchestration/caller.rs). Rationale: Timeouts cannot weaken identity checks.

- Persist brief/instruction/answer/report bodies; wake idle OMP with non-steering aside and coalesce busy followUp containing counts/pull instruction only. Main pulls untrusted bodies and explicitly ACKs after processing via bound inbox tools; Stored/Woken/Read/Acked are distinct. [Owner](integrations/omp/wake.ts). Rationale: A wake is neither processing nor acceptance.

- Never deliver orchestration by terminal typing, draft submission or Herdr prompt/callback steering. Native external Cockpit IRC Send targets the actual registered child without parent impersonation; Cancel awaits exact-child lifecycle disposal and owned background work, not turn abort. Applied/failed receipts follow native outcomes; unknown effects are not replayed. [Owner](integrations/omp/tools.ts). Rationale: Control receipts must represent real native delivery.

- Answer must link in_reply_to to the destination run’s current main NeedsInput message_id; other kinds cannot link. Locked exact same-sender retry preserves ID/payload/link before currency rejection; changed payload/link conflicts. New questions, unrelated messages and subagent reports cannot replace the main request. [Owner](crates/cockpit-core/src/orchestration/messages.rs). Rationale: An answer is question-scoped, not inferred from mail.

- Question receipts distinguish unresolved, delivered (Stored/Woken/Read) and acknowledged (Acked); ACK still awaits a fresh main report, not resumed work. Recovery/offline authority takes precedence. Main-only Ready/Result receipts remain separate from provenance-bearing subagent progress/reports/annotations and fresh Herdr observations. [Owner](crates/cockpit-core/src/orchestration/projection.rs). Rationale: Delivery status cannot substitute for work status.

- Explicit Progress/Ready/NeedsInput/Result may report upward to ancestors without control authority; only bound main files Ready/Result. Idle/done/exited without Result is missing-result attention; Result enters Review unchecked, and Accept needs reviewed explicit success plus exact current task revision. [Owner](crates/cockpit-core/src/orchestration/mutate/reviewed.rs). Rationale: Reporting alone cannot complete canonical work.

- Send back resumes execution; advanced operator acceptance remains supported. Acceptance preserves canonical task, Result/history/files/checkout; coding completion additionally requires all in-scope touched files committed, not merely staged or reported. Retirement is a separate durable outcome. [Owner](crates/cockpit-core/src/orchestration/mutate/reviewed.rs). Rationale: Completion must preserve reviewable durable evidence.

- One named lock and atomic replacement protect orchestration/state.json machine records, not a second task store/live status; assignment/Accept use recoverable exact-revision intents. External editors ignore the lock, leaving a final recheck-to-rename race; receipts do not promise exactly-once external effects. [Owner](crates/cockpit-core/src/orchestration/store.rs). Rationale: Participating-writer fencing is not global filesystem CAS.

- Only private owner dispatches/reconciles under per-run leases; restart preserves records and Herdr processes. Record effect intent first and retain exact receipts; ambiguous setup/launch/stop/close reconciles or uses guarded recovery, never automatic repetition. Publish only changed durable revisions, including recovery before later error, without self-waking unchanged passes. [Owner](crates/cockpit-core/src/orchestration/dispatch.rs). Rationale: Durable uncertainty must not become duplicate effects.

- Check status/Reconcile reviews an already-launched exact receipt read-only; fresh matching proof restores Launched while preserving lifecycle/Ready/Result/grants/location/inbox, including concurrent legitimate changes. Missing/stale/conflicting/unavailable evidence remains Missing/Unknown/NeedsReview and never proves absence. [Owner](crates/cockpit-core/src/orchestration/dispatch.rs). Rationale: Recovery observation must not replay setup.

- Only RunBindSession may roll over main SDK binding on trusted same-live PID/start/kernel-boot and exact endpoint/Space/tab/pane/terminal proof. Reports/subagents/reused terminals/contradictory evidence cannot. Preserve lifecycle/grants/attempt and instruct each new main once to inspect durable work without replaying effects. [Owner](crates/cockpit-core/src/orchestration/mutate/runs.rs). Rationale: A session rollover is not a relaunch.

- Automatic startup/transient failed-ephemeral recovery is bounded independently of manual attempts: durable reviewed-CAS intent revokes old incarnation, exclusive-tab preflight closes only freshly matched owned pane, proves absence, then reuses retry. Refuse foreign/racing splits or reused identity; never broadly tab.close a racing split. [Owner](crates/cockpit-core/src/orchestration/dispatch.rs). Rationale: Recovery may reclaim only its exact incarnation.

- Preserve a sole-tab project Space/context by recording a genuine ordinary terminal at exact Space/cwd with empty managed environment before closing failed pane; no placeholder/agent command. Uncertain preservation creation retains the old pane and escalates, never repeats. [Owner](crates/cockpit-core/src/orchestration/dispatch.rs). Rationale: Recovery must preserve project and selection identity.

- Kernel-fence retry commit on exited recorded shell and any bound-native PID/start/boot incarnation, not close ACK/layout loss. Live/suspended originals cannot duplicate; running waits on dispatcher tick without repeated close, and missing/changed/unavailable fingerprints stay unproven under locked recheck. [Owner](crates/cockpit-core/src/orchestration/dispatch.rs). Rationale: Queued accepted commands must not survive a duplicate attempt.

- Plain Launched snapshots are no-op consumers; mature automatic review queues a reviewed-CAS LaunchIntent and requires known original native exit. Stored binding with unavailable/changed boot evidence is not exited; never-bound explicit retry requires same-endpoint/boot old-pane and terminal absence, not expired Pending alone. [Owner](crates/cockpit-core/src/orchestration/dispatch.rs). Rationale: Missing attestation is not kernel-proven absence.

- Reviewed explicit RetryLaunch rechecks original absence/launch-process-shell fence, retains run/root/setup and durable work, advances attempt and requires new main binding; do not recreate checkout. Missing-process/root recovery stays operator-owned. Durable cancellation excludes late binding and conflicting Cancel/Retry/Reconcile until effects settle. [Owner](crates/cockpit-core/src/orchestration/mutate/runs.rs). Rationale: Retry changes incarnation, not task or checkout.

- Healthy reconciliation never renames/restarts agents or duplicates resources. Preserve unrelated panes, project/context/tasks/mail/plans/grants/history; only exhausted/unsafe/unproven automatic recovery emits deduplicated attempt-scoped diagnostics after prior failure. Root failures/operator-only or ambiguous destructive choices need the user; diagnostic JSON is untrusted evidence. [Owner](crates/cockpit-core/src/orchestration/escalation.rs). Rationale: Escalation follows failed recovery, not transient startup.

- Exact-bound startup without attestation is retryable caller_not_ready, not revocation; absent-to-matching OMP/native proof needs a new caller snapshot with all fences. Back off silently, cancel superseded binds and dispose watcher on caller_mismatch/session_mismatch/attempt_stale without stopping OMP or closing pane. [Owner](integrations/omp/controlLoop.ts). Rationale: Observer revocation is distinct from native shutdown.

- Reviewed accepted-worker retirement separately requests cooperative shutdown and closes only identity-matched managed pane after OMP exit, then empty owned Space. Recheck acceptance revision/authority/launch/native/shell/membership before effects; drafts, busy children, uncertain observation or changed identity retain/defer, and Unknown never permits stop/close replay. [Owner](crates/cockpit-core/src/orchestration/retire.rs). Rationale: Acceptance does not itself authorize resource disposal.

- Preserve unrelated resources/task/Result/history/files/checkout; original shell fingerprint proves ownership, not idleness, so an unchanged-shell busy builtin may end on eventual owned-pane closure. Detectable replacement executable/argv/foreground ownership retains it. CloseTracking/cancellation preserves records but does not guarantee native/descendant termination. [Owner](crates/cockpit-core/src/orchestration/retire.rs). Rationale: Shell fences cannot promise protection of every busy command.

- CLI idle runtime reobservation is capped at three seconds with early durable wake; core wait checks revision/task tokens on notification or one-second cross-process poll without live snapshots each time. Mounted Supervisor waits 5000 ms then refreshes runtime even unchanged; confirmed mutations refresh immediately. [Owner](crates/cockpit-host/src/cli_orchestration/wait.rs). Rationale: Wait bounds are not latency or CPU-improvement guarantees.

- Keep live RunAdopt for explicit unmanaged-agent adoption/binding, not retired checklist-format adoption. Supervisor task Markdown stays outside checkouts, implementation notes inside worker checkout, and selected Library/repositories are read directly without companion creation/copy/teardown. [Owner](crates/cockpit-core/src/orchestration/mutate/runs.rs). Rationale: Current run adoption does not revive retired formats.

- Persistent observer repair requires authorization for the exact session/operation and fresh identity/draft inspection; stopped OMP observers are not revived by Cockpit restart, and plugin reload is not proven rebinding. Guarded reconcile/retry retains canonical task/run rather than replacing task or closing tracking. [Owner](docs/native-install.md). Rationale: Guidance cannot authorize destructive persistent-session repair.

## Context & Review

- At most one Files/Review/Browser leaf per tab; existing Files/Review opening focuses and switches source while retaining per-source state, including unsaved editor text across unmounts. No addon launch/private inspection/terminal-renderer toggle; addon panes stay ordinary terminals. Resolve sources selected real, last real, Herdr-focused, then first real terminal. [Owner](crates/cockpit-core/src/viewer.rs). Rationale: Viewer source is distinct from placement.

- Open Files/Review from fresh same-tab real terminal evidence, pinning selected root/cwd/endpoint/tab/Space. Every viewer_id/binding_id request rechecks membership/binding/filesystem identity; later cd/source close cannot retarget. Missing context needs explicit Reopen; close/retirement releases binding without deleting run-local batches. [Owner](crates/cockpit-core/src/viewer.rs). Rationale: Bindings must not silently expand filesystem authority.

- Bound Library keeps its issued root and comments/search/media authority; standalone global Library is unbound/comment-free. Read live Library and authorized checkout roots with bounded no-follow identity checks; unsafe/stale/oversized/unsupported content fails closed. Comments identify absolute paths across roots. [Owner](crates/cockpit-core/src/context.rs). Rationale: Matching relative filenames do not identify a source.

- Review is read-only local Git staged/unstaged/branch/untracked state with explicit revisions and side-aware anchors, never Git/provider-comment mutation. Reuse needs current source/repository/comparison/revision/merge-base match and a loaded versioned viewer-bound snapshot; skip snapshots with untracked files, whose metadata misses same-size/mtime rewrites. [Owner](crates/cockpit-core/src/review/cache.rs). Rationale: Cheap tokens must not authorize stale Review bytes.

- Stream Git revision output into digests under normal process timeout instead of retaining large diffs; malformed current Review caches remain errors. [Owner](crates/cockpit-core/src/process.rs). Rationale: Bounded computation cannot hide current-format corruption.

- Viewer-owned whole-file/selected-line comments across files retain immutable source/line identity and authorization for saves, reattachment and delivery. Batches/paste receipts survive view changes only within owner run; stale/unknown outcomes retain drafts/receipts, and owner startup clears them. [Owner](crates/cockpit-core/src/comments/mod.rs). Rationale: Detached presentation must not erase current-run work.

- Paste only a revalidated prepared payload/source to explicitly selected eligible same-actual-tab agent, retaining real paths/original lines; preview is optional, submission is forbidden, and pending/rejected/unknown outcomes remain distinct for receipt reconciliation. [Owner](crates/cockpit-core/src/comments/paste.rs). Rationale: Paste is not automatic command execution.

- Request one sorted server index: tracked/nonignored untracked Git paths or bounded no-follow folder walk. Persist only path/metadata hints, never bodies/authority; fresh open re-enumerates root and every request authorizes it. Cache precedence is in configuration. [Owner](crates/cockpit-core/src/file_index_cache.rs). Rationale: Cached paths cannot grant access.

- Warm picker candidates on root open/window focus with 30-second throttle; show cached then fresh list, retry failures with backoff and revalidate every eight seconds while indexing is under two seconds. Slow indexing changes status, never aborts or dead-ends. [Owner](src/app/input/fileIndexCache.ts). Rationale: Eventual consistency must remain recoverable.

- Catalog read authorization may cache fresh 30 seconds/stale five minutes with refill and mutation-generation invalidation. Per-directory checkout discovery always examines original no-symlink/current-Git-boundary path; setup/plan/resume/teardown never use cached checks. [Owner](crates/cockpit-core/src/repository_cache.rs). Rationale: Mutation authority requires fresh checkout evidence.

- Map Markdown/Mermaid back to physical source lines including frontmatter, refresh file changes, search through bounded core ripgrep and safely open/refuse unsupported content; PNG/JPEG are bounded raster previews, PDF has no active renderer and HTML/SVG/remote assets never execute in host. [Owner](src/app/context/MarkdownView.tsx). Rationale: Presentation must preserve source and host boundaries.

- Library is a local view below tabs, not pane/tab/resource; without a session it has no Space actions or pane authority. Opening/closing leaves layout unchanged while membership/focus reconciliation continues; visible Library unmounts terminal renderers/subscriptions and close reattaches/resyncs selected tab. [Owner](src/app/shell/Workbench.tsx). Rationale: Library navigation must not mutate Herdr lifetime.

## Agent widgets

- Widgets are run-local in-memory dock content/tombstones, not terminals, Library files or Browser sessions; many IDs share a tab dock, eight live maximum. Same session/tab/ID show updates without source/tab selection, Herdr focus or input; only confirmed tab/final-terminal loss or owner shutdown retires them. [Owner](crates/cockpit-core/src/widget/store.rs). Rationale: Widget publication is presentation, not execution authority.

- CLI copies bounded regular no-follow file/stdin/choices bytes to existing WidgetService owner, never watches files or starts another owner. Resolve fresh endpoint/session/membership/source even for another explicit destination; external callers require pane/tab/Space plus session/socket, never guessed agent attribution. Source identity gates show/close/list/selection; native/browser contracts stay equivalent. [Owner](crates/cockpit-core/src/widget/target.rs). Rationale: Explicit destination does not invent a source.

- Identical bytes/kind are unchanged; HTML/choices changes replace even matching bytes. User Remove tombstone blocks ordinary show: reopen requires explicit user request, never retry/new-ID bypass, and starts unselected. Replacement preserves selection unless clear-selection, which advances revision even if bytes match; close is idempotent. [Owner](crates/cockpit-core/src/widget/store.rs). Rationale: Removal and selection are durable only for this run.

- Only user removal of a DOM-focused dock restores focus; agent close, retirement or snapshot removal never claims it. [Owner](src/app/widgets/WidgetDock.tsx). Rationale: Passive cleanup cannot steal keyboard ownership.

- Trusted HTML/scripts/events and ordinary HTTP(S) resources/network run in active opaque sandbox allow-scripts without same-origin/ambient host APIs. Host CSP strips base/http-equiv overrides and blocks IPC, objects, nested frames, workers, base URLs and forms; no separate webview/Chromium/image stream. [Owner](src/app/widgets/widgetDocument.ts). Rationale: This narrows host privilege, not CPU/memory/network/process exposure.

- Fragment-only self-frame links resolve against about:srcdoc on ordinary click/Enter; native scrolling/focus/:target/history and author cancellation remain. Modified clicks/downloads/other targets/external documents retain existing policies. Busy scripts may freeze native UI, not independent Herdr processes. [Owner](src/app/widgets/widgetDocument.ts). Rationale: Srcdoc base inheritance must not navigate the Cockpit host.

- cockpit.select records untrusted JSON for CLI pull only, never chat/paste/focus/terminal bytes. Match widget key/revision/frame source/incarnation nonce; cap four UI submissions/second, raw and canonical selection 16 KiB UTF-8, HTML 1 MiB/widget, owner HTML 64 MiB and snapshot metadata 8 MiB. [Owner](src/app/widgets/widgetStore.ts). Rationale: The narrow bridge needs identity and resource bounds.

- Return full nullable retained selection with content; cockpit.selection/hasSelection are mount/replacement snapshots, not live chat. selection reads immediately or waits 300 seconds by default, explicit timeout 1–3600; handle none/timeout/dismissed/retired and validate shape/allowed values without execution or interpolation into commands/instructions. [Owner](crates/cockpit-core/src/widget/store.rs). Rationale: Selection is data, not agent authority.

- Route bounded iframe Cockpit/Herdr prefixes and advertised custom shortcuts through existing workbench router, never synthetic typing/paste. [Owner](src/app/widgets/WidgetFrame.tsx). Rationale: Shortcut forwarding must not become a terminal input channel.

## Space Notes

- Notes is a local workarea, not Herdr pane/Library item/checkout sidecar/widget; header/Commands changes presentation only. Scratchpad/Todos/Kanban/Decisions/board comments share typed CLI/HTTP/native operations; HTTP checks Origin, and change tokens are not CAS revisions. [Owner](crates/cockpit-core/src/notes.rs). Rationale: One core contract keeps Notes behavior equivalent.

- Store UUID-scoped ordinary Markdown at the configured durable Notes root, outside worktrees/Library/ephemeral state; opening never implicitly creates/attaches. Bind live endpoint boot/session/Space, never labels/paths; restart needs explicit reattachment, transfer confirmation. Setup/teardown/browser reset do not manage content. [Owner](crates/cockpit-core/src/notes/registry.rs). Rationale: Persistent content identity outlives Herdr resources.

- Resolve a real current pane once, then pin Notes UUID and configured root; pinned content needs neither Herdr nor owner, while current targeting still needs its real pane and never grants arbitrary path access. Stable-ID revisions or full-document line refs are defined in configuration. [Owner](crates/cockpit-core/src/notes.rs). Rationale: A task must not drift to another Notes identity.

- One todos.md owns list/board: hidden stable IDs and optional Backlog/Doing metadata establish membership, checkbox Done; reopen restores open lane, moves retain source order without ranks, and unboard keeps todo/comments. [Owner](crates/cockpit-core/src/notes/todos.rs). Rationale: The board is not another task database.

- Decision replacement creates a linked new record without rewriting predecessor; recorded timestamps/comment creation/author metadata survive edits, and optional author labels are unverified text. [Owner](crates/cockpit-core/src/notes/decisions.rs). Rationale: Edits cannot fabricate historical authorship.

- Use bounded no-follow reads, stable advisory lock inodes, revision-checked surgical edits and atomic publication; preserve unrelated bytes and reject stale/malformed/ambiguous boundaries. External writers ignoring locks can race the final check, so no filesystem-wide CAS is promised. [Owner](crates/cockpit-core/src/notes/fs.rs). Rationale: Safe splices require participation, not inferred exclusivity.

- Keep UUID/record-scoped drafts/editor state across view/Space change and closure with bounded best-effort storage and visible failures. Conflicts require resolution; unknown writes require saved-state read and explicit may-duplicate acknowledgment, never automatic replay. [Owner](src/app/notes/useNotes.ts). Rationale: Navigation must not discard unresolved edits.

- Pointer/keyboard board moves capture identity/revision and cancel on source change/focus loss/closure/Space change without retargeting. Keep real dnd-kit listener cancellation, capture keyboard lane intent before deferred listener/collision refresh, and leave pointer drops collision-owned. [Owner](src/app/notes/notesSensors.ts). Rationale: Deferred sensors must not mutate stale sources.

- Reject stale identity/revision, absent membership, outside targets and same-column no-ops; never rank/reorder todos.md, and announce intended/confirmed results only through Notes live status. [Owner](src/app/notes/boardState.ts). Rationale: Gesture previews cannot override canonical order.

- Retain dark CodeMirror/token Markdown, 2px light caret, explicit focused/unfocused selection and safe GFM preview; reduced motion disables blink without changing focus ring, source/preview state, drafts or save shortcuts. [Owner](src/app/notes/MarkdownEditor.tsx). Rationale: Editor polish must preserve visible editing state.

- Title editors fit full content on value/width changes without inner scrolling/selection theft; lanes keep independent scroll and recovery errors use an in-flow band below panels, zero-height when empty, never obscuring controls. [Owner](src/app/notes/TodoTitle.tsx). Rationale: Layout must keep editing and recovery reachable.

- Task-detail threads retain durable comment edits/conflict recovery, confirmed deletion and external-append scroll behavior. [Owner](src/app/notes/TaskDetail.tsx). Rationale: Thread editing must preserve recovery and reading position.

## Inline browser

- One tab-local Browser association owns independent Chromium views/input/captures/drafts/feedback, not a Space sidecar/pane. Tab/zoom/Library hiding releases capture resources, not session; close/tab-final-terminal retirement/owner shutdown confirms stop and removes only derived identity-proven profile/workspace/config/work. Cookies/logins/site storage are disposable; failures remain inline/retryable. [Owner](crates/cockpit-core/src/browser/service.rs). Rationale: Cleanup must stay within the exact association.

- Leaf creation/reopening uses OpenFresh: stop/clean surviving session, never adopt, then start configured validated URL/default about:blank; CLI Open and existing-leaf Reconnect retain attach/new-page semantics rather than restoring a closed leaf. [Owner](crates/cockpit-core/src/browser/service.rs). Rationale: Fresh opening means a new disposable association.

- Browser profiles/work, viewer batches/paste receipts and Review caches last one owner run. Only exclusive browser owner resets before socket/dependent services; observers clear nothing. Stop proven leftovers, clear browser except owner.lock/owner.sock plus comments/review, with stable lock inode/no-follow/same-device deletion; uncertain shutdown fails startup without wiping. [Owner](crates/cockpit-core/src/ephemeral.rs). Rationale: Reset authority belongs only to the proven owner.

- Never reset Library/Notes/vault/config/project operations/real Herdr resources; close/fresh-open/retirement/shutdown discard outgoing association drafts/captures/feedback/receipts, with current tab/current CLI targeting and no detached saved-work guard/recovery surface. [Owner](crates/cockpit-core/src/ephemeral.rs). Rationale: Ephemeral cleanup is not durable-content teardown.

- Unsent notes/annotations belong to their page/document; navigation retires that page’s drafts but keeps other live-tab/prepared-capture references within current association/run. Preserve target/document/viewport/ownership/pinned-frame/immutable submitted-image checks and unknown-delivery reconciliation, never automatic replay. [Owner](crates/cockpit-core/src/browser/drafts.rs). Rationale: Live imagery cannot substitute for saved-frame evidence.

- Daemon-owned initPage policy derives running Browser.getVersion UA and changes only HeadlessChrome/ to Chrome/ before initial, CLI-created/popups and nested cross-site child-frame navigation, even hidden. Do not pin version/change executable/headless/launch/security/viewport/JPEG/attach semantics or claim general anti-bot/media/DRM/performance parity. [Owner](browser-runtime/browser-user-agent.cjs). Rationale: A UA adjustment has deliberately narrow claims.

- Accepted image-quality requirement remains implementation/acceptance-unverified ([requirement](planning/stability-and-gitlab-2026-09-20/ACCEPTANCE.md#live-browser-image-quality)): try sharp affordable capture, allow lower resolution during activity, pursue quality as activity eases; never freeze older sharp frames, poll continuously or weaken target/frame/lease checks. Measure Chromium capture separately from Linux WebKit display/input. [Owner](crates/cockpit-core/src/browser/cdp.rs). Rationale: Responsiveness does not authorize stale image evidence.

## Providers & setup

- Adapters expose supported read capabilities and explicit unsupported/unavailable errors, never silent substitution or remote writes; configured provider/artifact identity, not checkout Git origin, authorizes imports. Configured metadata informs plans without overriding repository choice or Herdr ownership. [Owner](crates/cockpit-providers/src/lib.rs). Rationale: Provider authority cannot become local filesystem authority.

- Explicit kind selects github/gitlab/gitea/jira/confluence, not ID/executable inference; forges require executable with optional login, Atlassian rejects executable/login and uses vault token only. Deployment/auth/base-path settings and defaults are in configuration; never automatically rewrite user config. [Owner](crates/cockpit-core/src/config.rs). Rationale: Adapter selection must be explicit and reproducible.

- Use pooled GET-only Jira Cloud v3 enhanced-search tokens/DC v2 offsets and Confluence Cloud v2 under exactly /wiki/DC v1 under preserved context path; auth kind never chooses deployment. No credential injection, CLI fallback/init/profile or remote write methods. [Owner](crates/cockpit-providers/src/site_http.rs). Rationale: HTTP reads cannot inherit unrelated CLI authority.

- Setup bounds/validates requested primary and linked artifacts before persistence and never follows references or chooses/clones repository from Jira URL/key. Optional failures retain successful assets and primary-available work; freshness compares provider metadata or canonical content. [Owner](crates/cockpit-core/src/projects/plan.rs). Rationale: Hydration failure must not erase valid setup context.

- SourceService is fetch-only with bounded metadata, short-lived RecentReads and collect_related; LibraryService alone persists/ selects. Hosts compose an acyclic graph; prevalidated setup commits assets before selecting IDs, selection failure keeps saved items and recovery reuses IDs without refetch. [Owner](crates/cockpit-core/src/sources.rs). Rationale: Fetch and durable selection have separate failure boundaries.

- Store one pasted Jira/Confluence token per provider ID/base_url in OS vault, keyed cockpit / <provider id> <instance>; URL changes cannot retarget token, explicit kind keeps account identity. Basic/Bearer are supported; glab/gh/tea retain CLI credentials and report unsupported. [Owner](crates/cockpit-core/src/credentials.rs). Rationale: Credential identity belongs to its configured instance.

- Credential boundary is write-only set/clear/presence/kind, without token/username/vault-detail readback or exposure in logs/errors/argv/Library/snapshots/env/config. No token yields source_credential_required, unavailable vault credential_vault_unavailable, without fallback; OAuth/passkey/WebAuthn sessions are not storable secrets. [Owner](crates/cockpit-core/src/credentials.rs). Rationale: Credential storage must not become disclosure or sign-in management.

- Token forms/labels/deployment defaults and process cache/timeouts are defined in configuration; reject Basic-username colon inline before submission without clearing token, restore opener/surviving ancestor and make no attachment state claim before credential status loads. [Owner](src/app/library/ProviderCredentialsDialog.tsx). Rationale: Form validation must preserve drafts and honest state.

- Send Authorization only to configured origin; JSON redirects stay same-origin with validated final on-site paths, binary media may cross-origin without credentials, both cap three hops/refuse HTTPS downgrade. Validate continuation endpoints and rebuild from token/offset instead of requesting returned URLs. [Owner](crates/cockpit-providers/src/site_http.rs). Rationale: Redirects and paging cannot widen credential authority.

- Confluence reads retain page identity/version/ancestors/labels/editor/attachments and locally convert storage XML on both deployments; explicit refresh may change old body but unchanged follows skip mass rewrite. Jira DC wiki strings convert conservatively with unknown markup left text, while ADF stays separate. [Owner](crates/cockpit-providers/src/confluence.rs). Rationale: Normalization must preserve supported source identity.

- Jira fields.attachment metadata is bounded, with malformed/beyond-256 entries diagnosed source_attachments_partial; both Atlassian providers capture metadata without default binary download. Explicit per-item/follow opt-in uses ID-based private no-follow staging, per-file/aggregate budgets, safe names, atomic publish and error cleanup. [Owner](crates/cockpit-providers/src/jira_attachments.rs). Rationale: Attachment reads must remain bounded and opt-in.

- On both deployments /rest/api/2/attachment/{id} must prove requested identity before on-site content download; stored source_url is untrusted. Supplied ID must match exactly, including when URL matches; null/malformed/mismatch is not missing. Only absent DC ID permits base-path /secure/attachment/{id}/file or /rest/api/{version}/attachment/content/{id} without suffix. [Owner](crates/cockpit-providers/src/jira_attachments/download.rs). Rationale: A link cannot override contradictory attachment identity.

- Remove downloaded deletes binaries only; confirmed replacement re-downloads edited bytes only after exact current hashes, opted-in follow retries failures and incomplete manifests cannot replace existing binaries. Jira download failures report per row without discarding saved issues; safe raster/HTML/SVG/PDF rules remain the viewer contract. [Owner](crates/cockpit-core/src/library/attachments.rs). Rationale: Download failure cannot erase successfully imported context.

- Linux Secret Service is supported; macOS Keychain is compiled but unverified. Live Data Center/Bearer, Jira attachment download and forge URL traversal acceptance remain unverified; fixtures are not live deployment proof, and glab host selection cannot express a port. [Owner](crates/cockpit-providers/src/gitlab.rs). Rationale: Claims must retain platform/provider evidence limits.

## Library storage, selections and follows

- Library is durable/global and managed read-only by convention, not sandbox. Add/refresh saves there first; Spaces select IDs/configured existing repository paths, never copies/pins/retry-update state. Next read sees refresh, removing selection deletes no files, and folder capture is distinct. [Owner](crates/cockpit-core/src/library/space.rs). Rationale: Relevance cannot create a second content authority.

- Keep human-readable source hierarchy at provider/host/.../leaf/Title.md with _files attachments, folder-relative copies and machine state only in .cockpit; preserve titles except unsafe characters, append stable ID on collision and explain layout in root README. Rename directories/descendant paths in one journaled commit, never copy moves. [Owner](crates/cockpit-core/src/library/layout.rs). Rationale: Users must navigate content without hidden product knowledge.

- Each provider item owns only its document/_files, not child items/user files; refresh/replacement/removal preserves them. Strict current index schema is 4; reject obsolete schemas instead of upgrading, retaining serialized legacy_migrated, relations_captured, references and move-intent fields. [Owner](crates/cockpit-core/src/library/layout.rs). Rationale: Removing upgrade consumers cannot redesign current records.

- Provider frontmatter retains schema_version/provider/resource_type/canonical_id/provider_instance/source_url/original_url/complete/source_revision/content_hash/generated plus type/status/priority/author/assignee/timestamps/comment counts and attachments. Canonical normalized Markdown, not raw payload, uses title/summary, optional Description and chronological Comments; timestamp/permalink cards preserve optional edited-time/location, demote provider headings and show partial counts. [Owner](crates/cockpit-core/src/library.rs). Rationale: Readable snapshots need explicit identity and faithful comment structure.

- sha256 content_hash covers provider-instance/resource/canonical identity, title, source revision, body, completeness and nonempty extras/attachments, not URLs/container presentation/diagnostics/fetch time; issue metadata never enters content_revision. [Owner](crates/cockpit-core/src/library.rs). Rationale: Presentation changes must not look like content changes.

- Refs are sorted/deduplicated Manual/Follow{follow_id}/Space{space_context_id}; Keep in Library is Manual. Change refs only through locked mutate_index plus birth ref, with selections/ref changes atomic; publish/update/journal replay preserve current refs and purge time rather than caller summaries. [Owner](crates/cockpit-core/src/library/store.rs). Rationale: Concurrent follows must not overwrite retention membership.

- No refs creates purge_after/Unreferenced; live-follow drops wait nonconfigurable 14 days, explicit last-ref removal is due immediately, and any new ref clears tombstone. Sweep after every add/refresh only when due and not leased/locally edited; manually pinned pages survive follow removal. [Owner](crates/cockpit-core/src/library/refs.rs). Rationale: Retention protects independent holders and user edits.

- Library/item leases own independent advisory-lock descriptions and explicitly unlock on Drop; closing File alone can leave forked pre-exec locks. Keep lock inodes stable, never unlink cleanup or let one shared lease release another reader/replacement writer. [Owner](crates/cockpit-core/src/library/store.rs). Rationale: Reservations must not outlive or interfere with their owners.

- Folder capture is typed absolute/home-relative explicit copy, no native picker/live/two-way link; canonical no-follow roots cannot overlap owned roots and source remains unchanged. Exclude/count nested .git, symlinks/specials/hardlinks/native executables/build dependencies; byte-sort and retain configured file/byte prefix as Partial. Explicit re-copy CAS protects edited Library bytes. [Owner](crates/cockpit-core/src/library/folder.rs). Rationale: A bounded capture is not an existing-checkout selection.

- Markdown links remain inside authorized root; source URLs open Library items and copy/comments/paste retain real paths. New context terminals receive Library root; existing ones use read-only context --current and returned item/checkout/repository paths, never environment retrofit. Writable notes stay separate. [Owner](crates/cockpit-core/src/context.rs). Rationale: Discovery cannot silently rewrite terminal authority.

- Confluence follows snapshot every readable page across top-level trees; Cloud folders are ancestor-only nodes. Manual metadata/body lookups stay concurrent with eight changed-page fetches, deterministic parent-first per-page crash-safe commits and directory moves from version/title/ancestor changes. [Owner](crates/cockpit-core/src/library/follow_plan/confluence.rs). Rationale: Hierarchy must survive partial publication and moves.

- Only complete enumeration plus individual confirmation may mark removed-at-source, preserving snapshots; partial/failed/cancelled enumeration proves no absence. Excluded pages stay excluded until whole space re-follow, and stopping follow keeps ordinary items. Global refresh changes live content, not Space-selected IDs. [Owner](crates/cockpit-core/src/library/follow_plan/confluence.rs). Rationale: Incomplete discovery cannot remove retained context.

- Normalize JQL whitespace/remove trailing ORDER BY; bare project means project = KEY. Follow ID hashes provider/instance/query with follow:; issues share provider/instance/type/key identity/path across follows. Resolve shows query/count, 100+ for full bounded first page and mode suggestion; relative dates default accumulate, never CLI default-project filtering. [Owner](crates/cockpit-core/src/library/jira_follow.rs). Rationale: A query defines global membership, not Space copies.

- HTTP Cloud-token/DC-offset listings use 100/page and library_space_pages cap; manual order updated DESC, delta fixed epoch-ms bounds updated ASC,key ASC. Preserve returned updated wall-time comparisons; fetch changed/removed/failed/unknown content. Accumulate probes newest revision and key-checks omitted tracked members. [Owner](crates/cockpit-core/src/library/jira_follow.rs). Rationale: Stable listing identity avoids mass refetch and missed members.

- Manual live refresh never drops on failed/truncated/cancelled/empty listing; empty-with-members remains Partial. Background absence also needs repeated complete inventories/confirmation. Accumulate never drops or stops refreshing tracked keys outside predicate; incomplete runs grant no removal authority. [Owner](crates/cockpit-core/src/library/jira_follow.rs). Rationale: Query disappearance is not confirmed source absence.

- Stop following keeps exclusive issues as Manual; remove query/items deletes only exclusive members and refuses local edits. Removing one issue excludes it from all referencing follows by Library ID; per-item/follow attachment opt-in stays independent of issue save. Global query refresh never changes Space-selected IDs. [Owner](crates/cockpit-core/src/library/jira_follow.rs). Rationale: Follow controls must preserve other references and edits.

- One reference_depth, without boolean/crawl aliases, accepts API 0–5 or library_reference_depth_invalid, UI Off/1/2/3. Default single Jira issue 1, forge issue/PR/new query/missing stored field 0 with no schema bump; Resolve retains stored depth, refreshing add may change/clear, Keep at 0 preserves it. Confluence page/space rejects >0, folders ignore depth. [Owner](crates/cockpit-core/src/sources/references.rs). Rationale: Traversal intent must be explicit and stable.

- Extract up to 64 references/asset from Jira parent/subtasks/links and description/comments keys/URLs, other-provider body URLs. Jira keys require word boundaries and same configured site, not OPS-12-fix/xOPS-1; resolve only configured Jira/Confluence pages/forge issues-PRs with direct-import authority, leaving space/unsupported/unconfigured URLs plain. [Owner](crates/cockpit-core/src/sources/references.rs). Rationale: Text references cannot widen provider authority.

- Fetch-only traversal is breadth-first, eight in flight, canonical identity dedup collapsing self/cycles and expanding reached providers uniformly. Internal nonconfigurable caps: single 32 related/8 MiB/60 s, query 100/16 MiB/180 s. Save seeds first; failure/cap/cancel reports unsaved Partial without removing seeds/successes or authorizing drops. [Owner](crates/cockpit-core/src/sources/references.rs). Rationale: Bounded traversal must retain successful context.

- Persist one shallowest-route included_by reason per holder with relation/step and deduplicate rendered Details reasons; seed shows related depth. Single refresh retraverses stored depth; related items get Manual plus seed-held reason, survive seed removal and complete unreached refresh loses reason only, never Manual. [Owner](crates/cockpit-core/src/library/related.rs). Rationale: Inclusion explanation is not exclusive retention ownership.

- Query-related items count toward follow membership with Follow plus follow-held reason; listed items become seeds and lose reason. Live drops unreached related only after complete seeds and related pass, accumulate never drops; unchanged seeds use stored references. Failure/cap/cancel/incomplete retains all members and one Partial row without marking whole follow partial. [Owner](crates/cockpit-core/src/library/related.rs). Rationale: Incomplete traversal cannot untag membership.

- Jira collections remain global, individual issues selectable directly; reference refresh never fans out to Spaces. Add targeting a Space selects what that add saved, including Confluence follow items; later follow refresh alters live Library only. [Owner](crates/cockpit-core/src/library/space.rs). Rationale: A selected ID set is not a follow-copy state.

## Library synchronization and presentation

- Changed-since Jira/Confluence discovery is required, not optional optimization; only long-lived native/serve starts one owner per Library root, no closed-app daemon/short-lived CLI sync. Fixed-window/default values are in configuration; complete discovery persists candidates before checkpoint, independent body failure/backoff survives restart. [Owner](crates/cockpit-core/src/library/sync.rs). Rationale: Checkpoint completion does not mean every body published.

- Daily complete inventories reconcile membership/hierarchy/tracked accumulation and seven-day rolling audits catch labels/attachments/body changes outside revisions. Persist frozen standalone cohort/cursor/cumulative observations across budget exhaustion; coalesce overdue work and never derive absence from incomplete chunks. Jira key-move identity/refusal remains, without numeric-ID migration. [Owner](crates/cockpit-core/src/library/sync.rs). Rationale: Budgeting and restart cannot fabricate absence or identity.

- Use Jira epoch-ms [lower,upper) and fixed Confluence lastmodified windows; unknown CQL timezone requires outward minute rounding and UTC ±14-hour safe-superset envelope. Equal revision/hierarchy skips body reads with inventories as backstop. [Owner](crates/cockpit-core/src/library/sync.rs). Rationale: Discovery must not miss changes through timezone assumptions.

- Share per-origin manual/background pacer across instances with configuration defaults; queued/active manual work precedes background, Retry-After/cooldown holds both, and learned spacing never relaxes stricter background policy. Preserve import limits, local edits, refs and exclusions. [Owner](crates/cockpit-providers/src/site_http/pacing.rs). Rationale: Rate-limit handling must not weaken configured safety.

- Quiet checks do not rewrite index/snapshots/history; incomplete attachment manifests preserve binaries. Recheck follow/exclusion authority under metadata lock after async fetch/download waits. [Owner](crates/cockpit-core/src/library/sync.rs). Rationale: A stale fetch cannot resurrect excluded members.

- Visible Library/context lists probe bounded first-page generation roughly every sixty seconds; unchanged keeps listing/object without remaining pages, changed reuses first page and atomically replaces all pages. Preview follows primary item identity through rename and rereads only its changed snapshot; manual events still refresh bytes at same indexed revision. [Owner](src/app/library/useLibraryOperation.ts). Rationale: Unrelated generations must not reset preview selection.

- Keep automatic notifications distinct from manual refresh, cancel stale responses and never retarget bound file/comment authority. [Owner](src/app/library/useLibraryOperation.ts). Rationale: Background freshness is not source reauthorization.

- Group provider instance→container→item by provider ID and instance, not host; cross-provider referenced items stay in their own containers. Follow row uses query/space name with overflow tooltip and Following/count second line. Same-container Jira subtasks nest via stored parent reference/listing-only parent_item_id, otherwise top-level; disk parent_item_id stays empty and siblings newest-key-first; missing captured parent appears only after refetch. [Owner](src/app/library/libraryState.ts). Rationale: Visual nesting must not change on-disk identity.

- Tree has one roving tab stop (last focus, selected, first): arrows/Home/End navigate, right expands/enters, left collapses/parents, Enter opens/toggles, Space clicks, Shift+F10/Menu opens menu; navigation from body restores tab stop after disabled controls/closed menus. [Owner](src/app/library/LibraryTree.tsx). Rationale: Refresh must not strand keyboard navigation.

- Keep identity left and Refresh/Space/menu/Details right in title row, then quiet facts/state lines; Details shows source/local metadata with shared status/provider marks. Accept epoch-ms/ISO display timestamps and keep Library launcher between Browser/Commands with registry hints. [Owner](src/app/library/LibraryItemHeader.tsx). Rationale: Stable hierarchy makes actions and state legible.

- TreeSplitter writes width directly once/frame without React rerenders while dragging; measure actual grid width only at drag start/arrow keys, with narrow viewers capping stored width at 60%. [Owner](src/app/viewer/ViewerLayout.tsx). Rationale: Large trees require bounded resize work.

## Subscription limits

- Read one omp usage --json --redact --no-extensions report for Codex/Claude/Copilot including Business. OMP owns auth/accounting; Cockpit never acquires credentials/signs in/switches/redeems/invalidates cache or separately collects GitHub quota, though OMP may refresh its own tokens. [Owner](crates/cockpit-core/src/quota.rs). Rationale: Quota observation must not become authentication management.

- Copy copilot:premium AI-credit quantities without scaling, including requests label; ignore unrelated counters/unsupported units. OMP JSON lacks billing flags, so do not independently classify legacy request-billed accounts. [Owner](crates/cockpit-core/src/quota/parse.rs). Rationale: OMP remains the accounting authority.

- Status returns immediately and schedules demand work, no host timer. Native/browser cache-root peers share private allowlisted quota/v1 schema 2/one OMP source, cross-process lock and persisted pre-command lease; unsafe/unwritable cache fails closed, never process-local collection fallback. [Owner](crates/cockpit-core/src/quota.rs). Rationale: Shared leases prevent duplicate commands across hosts.

- Collect at most five-minute idle or one-minute after completed success with visible selected-session working agent. Never shorten five-minute crash lease or 5/10/20/40/60-minute failure backoff; optional success/lock absence and older no-positive-success snapshots retain existing deadlines. OMP five-minute jittered cache still bounds freshness without invalidation. [Owner](crates/cockpit-core/src/quota.rs). Rationale: Working demand cannot erase crash or failure protection.

- Persist/transmit only anonymous allowlisted data with source observation times, not command completion; errors or age >15 minutes mark stale, >24 hours unavailable. Missing windows/unknown/zero/unlimited remain distinct. [Owner](crates/cockpit-core/src/quota.rs). Rationale: Unavailable data must not become zero usage.

- Reuse 28 px bottom strip, not another footer; show all reported windows with highest anonymous-account usage per tier/window, Copilot to two decimals including 0.05%. Narrow layouts retain selected-provider windows/other-provider count; details include every account’s values/reset/age/failure and unscaled credit counts. [Owner](src/app/limits/SubscriptionLimits.tsx). Rationale: Compact presentation must not hide reported windows.

- Used meters/values turn amber strictly >80%, red >95% independently of provider levels (80 normal/95 amber); stale hatching stays muted. Hover previews without focus, leaving/Escape dismisses; click/Tab-Enter/Commands pins details with opener restoration and no dedicated chord. [Owner](src/app/limits/SubscriptionLimits.tsx). Rationale: Severity and focus must retain precise semantics.

- Visible clients poll idle every sixty seconds, working fifteen, collecting three; hidden clients pause. Working evidence must be live in selected Herdr session, not stale demand. [Owner](src/app/limits/useSubscriptionLimits.ts). Rationale: Polling is demand, not independent collection authority.

## Supervisor and sidebar presentation

- Tasks defaults to wide lanes or narrow stacked disclosures; optional full-workarea Graph joins canonical current_run_id assignments and explicit parentage, not labels/location. Internal native subagents have no panes, unassigned/nested work stays visible, completed tasks count but hide; Graph uses 240×48 nodes/roving preorder-tree keys. [Owner](src/app/supervisor/SupervisorView.tsx). Rationale: The graph must represent actual relationships.

- Dependencies is a separate canonical DAG, not worker hierarchy/checklist graph; keep accepted prerequisites as context, independent runnable components and actionable blocked/invalid paths, with follow-up provenance distinct and local cross-view links. [Owner](src/app/supervisor/SupervisorDependencies.tsx). Rationale: Dependency navigation must not redefine task truth.

- One Decide/Recover/Notice projection drives counters/queue/badges/dim-only filters with source/real age; only root NeedsInput enters Decide, not routine supervisor approvals/worker questions, and blocked is attention overlay. [Owner](src/app/supervisor/attention.ts). Rationale: Operator attention must not become an approval bottleneck.

- Details are review-first Overview/Activity/Actions with Result/progress/path before description; Operator intervention collapses unless override/conflict, Diagnostics retains exact plans/bindings/receipts/guarded-ID repair. Measured attention/detail modes and local owning-scrollport reveal preserve independent offsets without terminal focus. [Owner](src/app/supervisor/useSupervisorLayout.ts). Rationale: Review navigation must remain observational.

- Retain root-scoped drafts, unknown identities, selection/filter/layout/scroll across navigation/hide/failure; polling never retries, authorizes, submits or discards. Only explicit Open terminal uses fresh identity/membership/focus ACK. Closed-tracking counts load bounded/identity-fenced only while open and label Updated. [Owner](src/app/supervisor/useSupervisor.ts). Rationale: Refresh cannot override unresolved user intent.

- Keep Herdr hierarchy/order/urgent rollup/lowercase labels/two-line rows/branch line/repository chevron. Only deliberate differences: shortened worktree labels with full name/path tooltip, State · agent second line and state-tinted SVG disc badges with five distinct shapes. [Owner](src/app/sidebar/Spaces.tsx). Rationale: Presentation polish must not alter Herdr semantics.

- Sidebar arrows/Home-End/left-right/Menu move DOM focus only; Enter/Space/click requests Herdr focus and selection follows ACK. Pending target uses neutral marker, and aria-disabled during mutation preserves keyboard focus. [Owner](src/app/sidebar/useRovingList.ts). Rationale: Navigation intent is not acknowledged selection.

- Below header, Spaces/Agents each reserve one-third of area and share remaining third by entry counts, scrolling independently; no entries splits equally. [Owner](src/app/sidebar/Sidebar.tsx). Rationale: Both lists need usable space under varying counts.

## Engineering and delivery boundaries

- Keep core Rust rules independent of Tauri/HTTP/WebSocket/socket framing; hosts compose narrow Herdr/provider/OS-vault traits. Share typed client requests/streams with startup adapter selection and validated response identity; stable typed error codes/useful messages stay at failure owner. [Owner](crates/cockpit-core/src/lib.rs). Rationale: Transport changes must not duplicate business rules.

- Serve shared frontend/versioned HTTP-WebSocket API from explicit foreground loopback origin only, with native access restricted by Tauri capabilities; browser has no separate remote authentication. [Owner](crates/cockpit-host/src/server.rs). Rationale: The trusted workstation boundary is not a remote service.

- Keep native dragDropEnabled false so shared HTML dragover/drop drives tab sorting; do not consume native file-drop events. [Owner](src-tauri/tauri.conf.json). Rationale: Native interception can swallow frontend tab drops.

- Mark opaque Linux main GTK window app-paintable before show without changing decorations/scratch buffering/GPU/cadence; reassess for transparent webview. CPU comparison must retain geometry/scale and prove terminal delivery plus xterm render cadence, not occlusion/fewer frames. [Owner](src-tauri/src/startup.rs). Rationale: Less rendered work is not causal performance improvement.

- Portable skills install only at explicit home/project targets, equal bytes unchanged and differing content requires replace; refuse symlinked/foreign-owned descendants/nonregular files. Serialize per skill and revalidate held identity, retaining final same-UID writer race and independent outcomes; inspect saved results on exit 20 rather than blind replacement retry. [Owner](crates/cockpit-host/src/bin/cockpit/skills.rs). Rationale: Skill installation must preserve unrelated user files.

- Use pinned Rust/Bun, Rust-derived generated TypeScript rather than handwritten wire schemas, affected checks and final-scope formatting. Reproduce runtime failures through actual user action/authoritative result, not mocked success; inspect path-limited staged diff before commit, separately authorizing installation/publication/user-session effects. [Owner](CODE_GUIDE.md). Rationale: A source change does not authorize deployment.

- Herdr automation uses unique disposable sessions/isolated XDG/config/fixture repo/resource ledger, never default session/manual gateway; verify exact executable/session/socket/ownership, point browser/native to same fixture and preserve evidence before stopping only recorded processes. [Owner](scripts/verify/resource_guard.py). Rationale: Experiments cannot implicitly mutate persistent resources.

- Pacing tests specify BackgroundPolicy, preserve stricter configured spacing, fill only configured in-flight capacity and bound cancellation permit waits. [Owner](crates/cockpit-providers/src/site_http/pacing.rs). Rationale: Tests must expose regressions without hanging.

- Missing metric providers are inconclusive exit 2, never passing complexity/coverage; source-targeted mutation is opt-in, not ordinary tests/gate/CI. Select verification by affected native/browser/Herdr/provider/lifecycle contract, with real affected-path smoke where required. [Owner](quality/README.md). Rationale: Unavailable evidence cannot become green acceptance.

- Errors stay inline at affected resource with last-known state/useful text/retry-resync while unrelated resources remain usable; toasts supplement only. Preserve practical keyboard/focus/meaningful-label/noncolor accessibility without claiming formal distribution certification. [Owner](src/app/shell/RecoveryPanel.tsx). Rationale: Recovery and best-effort access must remain discoverable.

## Deferred scope

- Global OMP settings/auth management, Cockpit provider sign-in/OAuth/passkeys/glab-gh-tea token injection, remote access/multi-user auth, agent history/resumable chats, separate inbox popup/settings UI and complete cross-provider read/write matrix remain outside current scope. Per-process supervisor/worker launch and supported provider reads remain supported. [Owner](CODE_GUIDE.md). Rationale: Implemented slices do not imply general platform support.

- Unbounded previews, arbitrary user-entered shell execution and raw socket browser forwarding remain unsupported; Herdr-advertised configured commands are supported. [Owner](CODE_GUIDE.md). Rationale: Configured commands do not authorize an unrestricted shell.

