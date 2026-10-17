# Configuration

This reference owns configuration values, examples and invocation precedence. [DECISIONS](../DECISIONS.md) owns security, ownership, publication and uncertainty rules; [CODE_GUIDE](../CODE_GUIDE.md) maps their implementations. Native installation and prerequisite recovery remain in [native-install](native-install.md).

## Configuration file and roots

Core loading uses an explicit configuration path, then `COCKPIT_CONFIG`, then `$XDG_CONFIG_HOME/cockpit/config.toml` (`$HOME/.config/cockpit/config.toml` when XDG is unset). A missing implicit default means defaults; an explicit unavailable file fails. Herdr-addressed agent CLI commands accept `--config`, then `COCKPIT_CONFIG_PATH`, before that core resolution. GUI and `serve` retain their own invocation discovery. The loader is [config.rs](../crates/cockpit-core/src/config.rs); root resolution is [roots.rs](../crates/cockpit-core/src/config/roots.rs).

Each root environment variable overrides the corresponding top-level TOML key; otherwise the XDG default applies. Unset XDG directories fall back beneath `$HOME` as shown below. Roots are absolute without parent traversal. Library cannot overlap cache, state or worktree roots; Notes additionally cannot overlap the Library or configured repositories. Keep durable Notes outside disposable checkouts and ephemeral pane-state roots.

| TOML key | Environment override | Default | Unset-XDG base |
| --- | --- | --- | --- |
| `cache_root` | `COCKPIT_CACHE_ROOT` | `$XDG_CACHE_HOME/cockpit` | `$HOME/.cache` |
| `worktree_root` | `COCKPIT_WORKTREE_ROOT` | `$XDG_STATE_HOME/cockpit/worktrees` | `$HOME/.local/state` |
| `state_root` | `COCKPIT_STATE_ROOT` | `$XDG_STATE_HOME/cockpit/operations` | `$HOME/.local/state` |
| `library_root` | `COCKPIT_LIBRARY_ROOT` | `$XDG_DATA_HOME/cockpit/library` | `$HOME/.local/share` |
| `notes_root` | `COCKPIT_NOTES_ROOT` | `$XDG_DATA_HOME/cockpit/notes` | `$HOME/.local/share` |

Repository discovery uses invocation roots, then the platform path-list `COCKPIT_REPOSITORY_ROOTS`, then TOML `repository_roots`, then the current directory. It enumerates supported repositories under those roots, not a hard-coded project path. `branch_template` defaults to `cockpit/{repo}/{task_id}` and `checkout_template` to `{repo}-{task_id}`. Issue metadata can supply task type/identifier; review artifacts can supply a source branch. Explicit user branch/location values override derivation.

File-picker cache entries are persisted path-only hints under `cache_root`; opens still re-enumerate the authorized root. [file_index_cache.rs](../crates/cockpit-core/src/file_index_cache.rs) owns this cache, not file content or authorization. Library limits are bounded in [limits.rs](../crates/cockpit-core/src/config/limits.rs), separately from synchronization settings:

```toml
library_root = "/data/cockpit/library"

[limits]
library_max_items = 20000
```

Discovery and Files limits are distinct from document-read and Library limits. The defaults and inclusive ranges below come from [limits.rs](../crates/cockpit-core/src/config/limits.rs); [discovery-limits](discovery-limits.md) explains breadth-first budget accounting. Changing a document limit cannot remove a discovery warning. Browser hosts accept `--repository-root`; `cockpit serve --help` describes invocation overrides.

| Limit | Default | Inclusive range |
| --- | --- | --- |
| `catalog_entries` | 16384 | 1–100000 |
| `catalog_depth` | 3 | 1–32 |
| `operation_timeout_ms` | 30000 | 1–600000 |
| `context_directory_entries` | 1000 | 1–10000 |
| `context_tree_depth` | 32 | 1–64 |
| `context_preview_bytes` | 1 MiB | 1 KiB–8 MiB |
| `context_preview_lines` | 5000 | 1–20000 |

An illustrative discovery configuration (the entry budget is nondefault):

```toml
version = 1
repository_roots = ["/path/to/projects"]

[limits]
catalog_entries = 10000
catalog_depth = 3
```

## Endpoint and CLI selection

Herdr-addressed `widget`, `context`, `library add`, `browser`, `notes` and orchestration commands share [endpoint.rs](../crates/cockpit-host/src/bin/cockpit/endpoint.rs). Explicit `--herdr-session`/`--herdr-socket` selectors take precedence; otherwise launch `COCKPIT_HERDR_SESSION` pairs with the pane's `HERDR_SOCKET_PATH`, then native Herdr inheritance. Socket layout identifies `herdr.sock` as `default` and `sessions/<name>/herdr.sock` as a named session. An explicit Herdr session never borrows another session's inherited socket; a stale selected socket fails rather than falling back. This physical session is separate from the agent conversation's `COCKPIT_SESSION_ID`.

Agent launches receive an absolute selected `COCKPIT_CLI_PATH` and configuration, not a guessed worker-PATH replacement. Host role uses its actual executable; Native prefers sibling `cockpit-cli` rather than the installed GUI, with the distinct development `cockpit` host fallback retained. Bare `COCKPIT_CLI_EXECUTABLE` overrides and native PATH fallback resolve in the owner's environment before worker cwd/environment changes and must identify an executable regular file; otherwise `cockpit_cli_unavailable` is explicit. The installed pair needs no override. Agent coordination uses issued private credentials with optional actual conversation metadata; physical commands separately require fresh evidence.

New context terminals receive `COCKPIT_LIBRARY_ROOT`. Existing terminals are not retrofitted: `cockpit-cli context --current` reports the originating Space, Library root, selected `items[].path`, `checkout_path` and `repository_paths` for direct reads. Explicit targeting uses `--herdr-session ID --herdr-socket PATH --space ID`; `--config PATH` selects configuration. Writable task notes belong in Space Notes, not managed Library content.

`cockpit-cli library add --current [--provider ID] [--follow] INPUT` saves to the global Library and selects what that add saved in the originating Space through the app's existing add operation. `--current` requires a Herdr pane (`HERDR_ENV=1`); alternatively use `--space ID --herdr-session ID --herdr-socket PATH`, without `--current`. `--config PATH` selects configuration (otherwise `COCKPIT_CONFIG_PATH` applies). Inputs are provider URLs, Jira JQL or Confluence spaces with `--follow`, and absolute or `~/` folder paths; relative folder paths are not resolved against the working directory. Already-saved items are selected without being overwritten. The command uses tokens already stored through Cockpit, reads providers but never writes to them, and offers no credential entry, select-only, remove or refresh command. Pacing is per process: the short-lived CLI does not share the running host's in-memory pacer or start background sync.

The command waits for completion and prints JSON containing `target`, `space_label`, `pane_id`, `library_root`, `operation` and selected `items[].path`. Exit 0 means every operation phase is Done; Partial, Failed or Cancelled phases exit 1 with the report still on stdout. Preflight/start errors exit 1; usage errors exit 2. Interrupting does not roll back saved items; the abandoned operation is marked interrupted on its next read. `cockpit-cli library add --help` describes the bounded input and targeting options; `context` remains read-only.

## Supervisor launch and attempt authority

The existing configuration file selects the supervisor/worker harness; startup has no per-launch harness picker. These are trusted local executable/argv values, not agent-provided shell text:

```toml
[orchestration]
harness_kind = "omp"
command = "omp"
extra_args = []
max_parallel = 4
process_scope = "auto"
inbox_prompt_hints = false
# Optional custom initial-prompt argv; exactly one whole "{prompt}".
# initial_prompt_args = ["--prompt", "{prompt}"]
# OMP defaults to ["--hook", "{hook_path}"]; [] explicitly disables its hook.
# hook_args = []
# OMP defaults to same-chat ["--resume", "{session_id}"]; override for other harnesses.
# resume_args = ["--resume", "{session_id}"]
```

| Field | Default / contract |
|---|---|
| `harness_kind`, `command` | `omp`; literal harness identity/executable, no shell interpolation. |
| `extra_args` | Empty; literal fresh-launch arguments. |
| `initial_prompt_args` | Absent: OMP gets the short bootstrap as a positional argument; other harnesses get no initial-prompt argument. When configured, exactly one argument is the whole `{prompt}`; embedded/missing/duplicate placeholders fail. |
| `hook_args` | Absent: OMP uses `["--hook", "{hook_path}"]` for its materialized per-launch hook; other harnesses have no hook arguments. `[]` explicitly disables the hook. A nonempty list requires exactly one whole `{hook_path}`. |
| `max_parallel` | `4`, inclusive range `1–64`; bounds concurrently live task attempts under a root, not canonical task count. |
| `process_scope` | `"auto"`; new DirectProcess incarnations attempt a harness-neutral owned systemd user scope on supported Linux/cgroup-v2 hosts. `"off"` launches directly with no scope receipt. Unsupported/probe-failed auto launch remains uncontained, not blocked. |
| `resume_args` | Absent: OMP uses `["--resume", "{session_id}"]`; other harnesses have no same-chat resume launch. An explicit list overrides this adapter. Resume replaces fresh-start `extra_args` and requires exactly one whole `{session_id}` argument with an actual registered conversation ID. |
| `inbox_prompt_hints` | `false`; explicit opt-in counts-only supported terminal prompt hints, not required inbox delivery or bootstrap. |

Placeholder replacement is whole-argument only; there is no general interpolation, shell expansion, guessed `--continue` or fallback harness. `harness_kind` is nonempty printable, at most 512 bytes; `command` is nonempty printable, at most 4096 bytes and not an option. Literal argv lists are bounded to 16 arguments, each nonempty printable and at most 4096 bytes; NUL is rejected. The explicit empty `hook_args` disable value is allowed. Default OMP launch retains a stable per-run `--session-dir`, never a fabricated conversation ID. Resolved configuration persists with durable launch intent; routing remains task placement, not harness selection.

Closing a supervisor terminal stops its agent and leaves it stopped; background observation and restarting Cockpit do not relaunch it. To start it again, select its original Space and choose **Resume supervisor** in the tab strip or Commands. Cockpit reopens the registered conversation and retains its root and tasks; repeated requests join the same restart rather than launching another supervisor. OMP needs no `resume_args` setting. Previously saved OMP roots blocked by `harness_resume_unsupported` become retryable after updating Cockpit. Other harnesses still require their explicit resume argv. Physical-exit and live-writer protections remain in force. A still-running supervisor can reattach automatically without starting a new process.

Core task/schedule/replacement/credential/messaging/cleanup authority is harness-neutral; OMP auth export, session files and native detection are not prerequisites. Herdr-supported agent interactions, credential CLI and optional harness hooks are allowed adapters. OMP's thin public-API hook is the first optional adapter, not exclusive support or a requirement for Claude Code, Copilot, Codex or other configured harnesses. It is per-launch, with no custom tool gates, global installs/settings, raw terminal typing or draft replacement. Extension-free credential CLI remains complete. A different harness can use configured literal prompt/hook argv only when that hook API is compatible; otherwise disable the OMP hook and use CLI. No obsolete `omp_extension`, `COCKPIT_OMP_EXTENSION` or orchestration `model` alias is supported. [OMP quota executable](#omp-quota-executable) selection remains separate.

Project/checkout identity uses canonical local filesystem paths, device/inode and Git common/worktree directories, not a host machine identity. One orchestration store serves one trusted workstation; cross-workstation `state_root` sharing is unsupported. Checkout claims conflict by key/path overlap regardless of host. Missing OS machine identity does not block supervisor startup, project resolution or ordinary claims; physical process/reboot/build/cleanup proofs that require it fail closed.

The CLI descriptor reports `orchestration_protocol: 4` and `store_schema: 4`. Schedule/generation metadata is additive and defaulted within schema 4; no converter is required. Incompatible schemas, including schema 3, fail with `orchestration_state_incompatible` without migrating, deleting or rewriting their bytes. A fresh `state_root` creates separate state, not permission to discard old resources.

### Credential activation and replaceable workers

`COCKPIT_JOB_CREDENTIAL_FILE` supplies private attempt authority; `COCKPIT_CLI_PATH` names the paired executable. No skill-read or conversation-identity hunt is required before ordinary coordination:

```sh
"${COCKPIT_CLI_PATH:-cockpit-cli}" run context --json
"${COCKPIT_CLI_PATH:-cockpit-cli}" run brief --json
```

The first credentialed context call activates the issued capability and atomically claims an authorized vacant worker generation. Current run/root/attachment epoch and pinned task generation fence later calls. Actual session metadata or explicit `--harness`/`--session-id`, when supplied, is recorded/revalidated and conflicting independent sources remain errors. An actual conversation is optional for normal coordination and required for same-chat resume; never infer an ID from a filename, pane label or launch acknowledgement.

For local orchestration management without a usable job binding, pass `--operator` explicitly and name the target root/task/run. This mode ignores ambient private credentials, run/attachment IDs and conversation metadata; no OMP settings changes or ownership repair are required. Endpoint/session selectors still identify the actual resource. Task writes and worker scheduling/replacement require `--root`; job-self reporting/context/inbox commands remain credential-scoped rather than impersonating a worker. For example:

```sh
cockpit-cli run --operator --root ROOT_ID --herdr-session SESSION_ID cleanup-finish --request-file REQUEST_JSON
```

The cleanup request names `operation_id`, `root_id` and optionally `run_id`. Operator authority does not invent successful Results, passed checks or completed external effects.

Tasks/schedules remain independent of attempts; schedules survive root credential/attachment rotation through immutable receipt/exact root/current binding validation. Revoked callers/removed schedule replay stay rejected. Atomic replacement preserves old work/claims; typed source evidence selects GitHead isolated worktree or a private no-copy Directory attempt retaining original project/context. Same-checkout reuse requires precise reboot or exact drained scope plus registered consumer proof/fresh Herdr inventory—not universal absence/main-process exit. Cancel leaves live attempts untouched.

Same-project/repository accepted Code dispatch freezes an immutable candidate base SHA containing original output head/commits OR the exact current validated Integrated target/mapped commits with passed target checks. An explicit qualifying base is allowed; checked status or later HEAD changes cannot substitute stale inputs. Missing materialization stays queued with deduplicated `dependency_input_not_materialized`, waking on Git refs/integration even after old resource retirement. Publication rechecks accepted Result/task/source/integration and candidate fences. Cross-repository/conceptual/manual/no-source edges do not demand unrelated commits. NonCode actual Result/artifact references remain retained/read-only, not inferred published or deleted.

Mutable ExistingSpace/Open targets are not retargeted to satisfy inputs. After setup, the actual fresh clean HEAD and same-repository original/integration proof reserve `base_commit`; immediately before latch release, HEAD/clean/input proof is checked again and publication CAS fences the reserved SHA/current accepted inputs. A changed checkout or prerequisite cannot silently release a stale writer. Scope capture/startup uncertainty is bounded by the 20-second startup path, not an unsupported 60-second idle probe loop.

Read direct prerequisite scope and valid accepted source references without publishing or mutating:

```sh
cockpit-cli task inputs TASK_ID --json
# Worker: omission or --self selects its assigned task.
cockpit-cli task inputs --self --json
```

`task inputs` is observational even when direct prerequisites are blocked, missing, ambiguous or unchecked: references remain null/diagnosed rather than requiring the graph to be runnable. Top-level `dependency_state` exposes existing `{state, unmet, problems}` readiness separately from `dependencies[]` references; Scheduler's launch gates remain authoritative. Accepted data is compact: run/attempt, Result ID/kind/outcome, source/integration, checkout/project and minimal setup—not plans/briefs/attachments/effects. Root/session/task/document fences remain; changed read identity returns `task_inputs_changed` for reread, while malformed accepted source fails explicitly. Artifact strings never prove paths/hashes/availability/publication/consumption; real output access/retention remains required.

Directory replacements exclusively create private `state_root/orchestration/attempts/<run_id>` without source copies. `OwnedAttemptDirectoryIdentity` has exactly `creation_operation_id`, `root`, `device`, `inode`, bound to the immutable existing operation journal; no manifest/sentinel, redundant UUID or startup OS-machine-ID requirement. Space binding preserves this identity. `COCKPIT_SOURCE_PATH` names the original read-only-by-instruction root; outputs belong in the attempt directory. Remove only an exact accepted retired drained EMPTY directory under released-claim/physical-proof gates. Nonempty outputs stay `outputs_kept` until durable publication/move; NotApplicable never permits recursive deletion, superseded/cancelled/failed attempts stay kept and the borrowed source is never deleted.

Private attempt creation rejects source overlap before effects and journals `directory_requested` before exclusive fd-relative `mkdir(0700)`. Unreceipted EEXIST/interrupted creation is protected, never adopted or replayed. Exact retained receipts revalidate identity without changing any output files; the saved source Library selections stay references to existing paths, with no Git or file copies.

A root Directory/Open target without an explicit source Space resolves transport staging once before LaunchIntent from exactly one fresh authoritative Herdr workspace `focused` flag, persisted as `staging_workspace_id`. Empty/ambiguous selection fails with `staging_space_required`: “Open or select a Space, or pass --space.” This root-only transport choice never infers cwd/UI focus, retargets workers or creates an extra bootstrap shell. UI start still supplies explicit ExistingSpace; workers retain explicit project-source routing and source focus.

Orchestration owned worktree setup uses local Git/filesystem preparation and its existing creation journal; borrowed-directory preparation neither initializes Git nor creates/deletes the user directory. After moving the same waiting terminal, exact fresh one-tab/pane/terminal/cwd and endpoint/session/generation evidence binds the new Space. For linked Git checkouts with an existing primary project Space, binding registers the same moved Space through Herdr `worktree.open`: repository/checkout inventory and existing Space identities must agree before dispatch, and the response and fresh snapshot must prove reuse of the exact Space/tab/pane with matching repository provenance. The current Space label and focus are preserved. Saved source Library IDs/repository paths retain direct selection, without companion copies or a context shell. Bind replay preserves its original move-operation identity; unproven Git creation is never adopted/replayed. Ordinary user Open/Create setup is unchanged.

Herdr 0.9.5 `worktree.open` creates a primary Space and shell when none is open. Cockpit therefore preserves isolated Setup/Create and linked-only-source placement without registering in that specific no-open-primary case; it does not invent a parent or block launch. Such workers remain ungrouped under this API limitation. An open primary requires registration; malformed, ambiguous or contradictory registration evidence remains an error, not a silent fallback. Existing completed operations are not retroactively registered.

Space capture matches current setup workspace/session/checkout/move creator and repeats workspace/move/epoch CAS after async reads. Proved-exit WORKER resume retains original setup operation Some/None, generation, filesystem metadata and old receipts/data. Retained Space uses an exact guarded scoped tab in place, without FS setup/move. Genuine absence uses `bind_resumed_space` with new move/epoch binding on the verified EXISTING checkout: fresh source/destination peer/terminal/tag/cwd proof, original Git/branch or attempt-directory identity and current HEAD reinspection preserve prior commits/dirty files. Only the new Space becomes owned; borrowed None never invents FS ownership. Root recovery is unchanged.

Automatic orchestration OwnedWorktree branches retain configured/review-source derivation, then append immutable `<task-or-root-UUID>-run-<run-UUID>-a<attempt>` before plan persistence and normal Git validation. With the default `cockpit/{repo}/{task_id}` template and no artifact, this produces `cockpit/<repo>/task-<UUID>-run-<UUID>-a1`, not `task-task-…`. For a repository named `cockpit`, the leading `cockpit/cockpit/` is intentional namespace/repository expansion. This applies only when Create has no explicit branch, making concurrent attempts distinct even without issue/artifact metadata. Explicit branches, Open and ordinary user planning/templates remain unchanged; no name-based retry or truncation is introduced, and existing branches are not renamed.

The generic `agent-exec` launch wrapper waits on a private per-incarnation placement FIFO before running any harness/model/hook. A single monotonic 20-second deadline covers probe, scope entry and placement; timeout fails launch rather than bypassing authorization. FIFO parent/device/inode and nonce are receipt-fenced; a bounded length/SHA256-framed payload supplies only final placement/context keys and actual Herdr workspace/tab/pane IDs. It never rewrites PATH, authentication, harness settings, configuration or private credentials. Partial release is Unknown without blind replay. Literal scope arguments retain `--expand-environment=no`.

The placement overlay permits exactly `COCKPIT_WORKSPACE_ID`, `COCKPIT_CONTEXT_PATH`, `COCKPIT_LIBRARY_ROOT`, `COCKPIT_REPOSITORY_KEY`, `COCKPIT_ARTIFACT_URL`, `COCKPIT_SOURCE_PATH`, `HERDR_WORKSPACE_ID`, `HERDR_TAB_ID`, `HERDR_PANE_ID`. Unsupported process-scope hosts remain explicitly uncontained; the placement FIFO itself grants no scope proof or source-write sandbox.

Unknown move/latch effects produce one deterministic liaison Observation per launch incarnation; repeated acknowledgement, observation or error timestamps do not generate another. Interrupted intents use the existing settle deadline. A fresh occupant or generic recovery cannot hide unresolved placement. The liaison inspects and may replace the worker while preserving terminal, checkout, claims and source/destination; no blind move/latch replay, unproven same-checkout restart or dead UI retry is authorized.

Unknown terminal creation may reconcile only a unique exact staging/tag/endpoint/native-PID match with positive original owned-unit evidence; it cannot recover from labels or a generic fresh occupant alone. Repeated unknown move is never resent. Missing/incomplete physical proof stays retained for the liaison.

Scope wrapping preserves literal argv with `--expand-environment=no`; its bounded two-second probe uses a validated real-user-manager connection. Worker/root unit prefix derives from run kind, never description. Exact unit/InvocationID/cgroup/member capture stays uncontained if missing/raced; no retrofit. Accepted-worker stop requires retired owned receipts, canonical completion/code-or-noncode proof, no Held/Queued holder and sealing. Committed replacement authority separately permits narrow old Superseded retirement/stop while all old files remain Kept. Roots/cancelled/arbitrary scopes never stop; scope proof is not an escaped-daemon sandbox guarantee.

Only probe/systemd-run clients receive the validated actual `/run/user/<uid>` XDG runtime directory (0700, same UID, no-follow path/private socket checks). Before FIFO readiness/harness exec, the wrapper restores the original `XDG_RUNTIME_DIR` exactly—including empty, absent or non-UTF-8 values—using literal arguments/PID-preserving exec. Authentication, PATH, session-directory, D-Bus and unrelated harness environment remain unchanged; no global environment/configuration/fixture rewrite is performed.

Replacement retirement records the committed WorkerReplace operation and exact previous run/generation/successor, with no fake Result/task revision. Only the fresh exact old idle pane and same verified old scope may retire/stop, with no open descendants or Held/Queued resources. Foreign cwd consumers do not block isolated old-scope stop but do prevent claim release/deletion proof afterward; unresolved claims stay Quiescing/Kept. Never repeat stop without fresh same-InvocationID Populated evidence. All old checkout/worktree/branch/attempt-directory/output data remains Kept even if the claim releases. Under-ten-second retirement after actual idle is a target, not verified timing.

Layout acknowledgement can precede scope entry. Exact private 0600 nonce/UID/no-follow `.ready` status is published before FIFO reader opening; reader-ready plus explicit Uncontained clears the planned unit promptly, with no 60-second unsupported capture loop. Scoped still needs genuine kernel/process/cgroup receipt capture; missing/malformed status is never fallback proof. The helper's single absolute 20-second deadline includes its ≤2-second probe. Exact owned-marker cleanup follows latch completion/timeout; replaced/unknown markers remain diagnosable. Registration can precede physical receipt without changing owner/stage/conversation. Startup status/deadlines confer no physical authority or launch/model-timing proof.

Scope proof covers the exact owned runtime, registered descendants/consumer resources and fresh Herdr checkout-consumer inventory—not every possible writer. Arbitrary D-Bus services, Docker and deliberate self-cgroup migration can escape and require separate registered resources/proof. Automatic Git removal stays non-force and requires a clean accepted head with verified integration or fresh containment in the root's actual checkout; containment does not fabricate target checks. Scope drain is not a universal sandbox guarantee.

The Code stop gate requires a clean exact accepted source HEAD and current Accepted/Retired DirectProcess/OwnedWhenSafe receipts, not target integration; clean unaccepted commits do not qualify. Noncode requires exact accepted successful Report/source, Result/reason-bound NotApplicable, nonempty root-reviewed artifact identities and nonempty passed focused checks without limitations. Those artifact identities are attestations, not hashes or universal byte verification. Checkout preflight revalidates borrowed paths without Git or automatic deletion authority. Current checked task/revision/attempt and Result/source/generation are fenced under lock before stopping; `scope_completion_unproven` retains resources with a concrete liaison cause. ScopeRelease CAS/journals never authorize a foreign recreated invocation.

### Inbox and liaison workflow

The short inline bootstrap and `run brief` supply role/task context and commands. Workers execute their authorized stages autonomously; the liaison handles questions, review/acceptance, target integration and cleanup blockers. Human decisions occur in its terminal conversation. `inbox next` combines durable waiting, pulling and capture bookkeeping internally, without a separate ACK model turn:

```sh
cockpit-cli task show --self --json
# Extension-free / hook-disabled waiting only:
cockpit-cli inbox next --wait 600 --deliverable --json
```

`--deliverable` omits routine progress/initialized messages. Capture is not proof of model processing, resumed work or an Answer; Answers still link the exact current NeedsInput question. Unknown operations retain original identity/payload for receipt inspection rather than blind replay. The optional OMP hook coalesces queued delivery; extension-free harnesses use the same credential CLI path.

`COCKPIT_INBOX_HOOK=1` marks only effective materialized hook argv; `run context`/`run brief` expose `inbox_hook_configured`. This is configuration/consumer-coordination metadata, never active-runtime proof or semantic authority. Configured-hook agents stay responsive in the terminal and avoid competing manual long waits/capture; a concrete hook-unavailable report permits the full credential CLI baseline. Meaningful OMP busy delivery uses public safe-boundary `sendUserMessage(..., { deliverAs: "steer" })`, not whole-turn deferral, and never aborts tools, kills writers or submits drafts. Queue/capture does not prove model processing or first-response latency.

Only the final child exec drops inherited parent conversation selectors `PI_SESSION_FILE`/`COCKPIT_SESSION_ID`; it preserves resume argv, capabilities/configuration, PATH and actual Herdr IDs. The launcher supplies fresh `COCKPIT_HARNESS` and `COCKPIT_INBOX_HOOK` (`1` only for effective hook argv, otherwise `0`). This is per-child metadata isolation, not global clearing or a new authority requirement. The optional OMP hook supplies its actual public-API session ID without synthesizing/waiting for a journal; its session-file value is spool/incarnation metadata only. The hook itself never clears independent current sources, and normal CLI source conflict/validation still applies.

Every nonempty `inbox next` batch is fsynced to a private durable spool before internal capture bookkeeping, even without a hook or explicit `--capture-file`; empty timeouts need no spool. JSON returns `capture_file`, and later `run context` returns `inbox_capture_directory` for lost-stdout recovery. Capture still does not prove model delivery.

After an actual human answer in the liaison chat, the root records its own exact unresolved question with `run answer`; do not fabricate an answer or clear waiting by reporting Progress. Answering a worker question remains a separate descendant message:

```sh
cockpit-cli run answer --in-reply-to ROOT_QUESTION_MESSAGE_ID --text "Actual human answer"
cockpit-cli run message WORKER_RUN_ID --kind answer --in-reply-to WORKER_QUESTION_MESSAGE_ID --text "Answer for the worker"
```

The optional OMP hook silently captures same-sender/same-recipient root Answers and root NeedsInput/Result reports: it never echoes a human reply into another turn or wakes the liaison to answer itself while waiting for the human. Exact durable question linkage/history remains intact. Worker NeedsInput/Result and operator/parent-to-worker Answers still use meaningful delivery. Silent capture is receipt bookkeeping, not a model answer or acceptance.

Role-scoped examples (UUIDs and paths come from actual task/run context):

```sh
# Liaison: defer launch until prerequisites and capacity permit.
cockpit-cli task schedule TASK_ID --space-worktree SOURCE_SPACE_ID --brief-file instructions.txt
cockpit-cli task schedule-cancel TASK_ID
# CLI inspects current run/generation fences before the single replacement submission.
cockpit-cli run worker-replace RUN_ID --reason "The current attempt cannot continue"
# Worker: report actual source/check evidence, not a fabricated pass.
cockpit-cli run done --summary "Completed the requested change" --check 'contract::actual-command::passed'
```

`run done` is worker-only; a root receives `actor_forbidden`. The liaison finishes the batch conversationally, remains Active/available, and reuses the same project root for new work. Low-level root Result reporting is diagnostic only, not retirement.

For non-code research, `run done --artifact ACTUAL_REPORT_PATH` selects a NonCode Result even in a Git checkout. Without artifacts, Git completion still derives committed source evidence; Directory completion still requires actual artifacts. Keep research deliverables outside an owned worktree if they must survive its cleanup.

An omitted schedule target requires the root project's explicit source Space (`project_space_missing` otherwise): Git selects an owned SpaceWorktree, Directory an OwnedAttemptDirectory without Git. At dispatch, `--path` resolving to exactly one open Git Space with matching canonical checkout and repository identity also selects an isolated SpaceWorktree. Directory, ambiguous and mismatched paths are not silently shared. Prefer isolation even for read-only Git work so accepted worker terminals can retire safely; explicit `--space` remains deliberate borrowed reuse. Git replacement without a source Space must retain the original exact Setup/Create catalog repository/common-dir and frozen HEAD; a Directory root Space is staging transport, never an invented Git source. Generated operation IDs and internally read fences remove turns, not stale-state protection; durable capture/queue is not a model answer.

To correct a proven pre-effect `plan_failed` attempt, re-run `task schedule TASK_ID` with the corrected explicit target. The mutation atomically supersedes only a never-started, effect-free attempt, revokes its credential and fences its assignment generation while replacing the schedule. Started, uncertain, claimed or physically recorded attempts cannot use this correction. `run reconcile` remains for uncertain setup, not planning rejection.

Automatic pane retirement is capability-held for every primary-Git Space with `foreign_space_group_close_risk`; fresh snapshots cannot atomically exclude foreign grouped Spaces. This Cockpit pre-dispatch refusal records `primary_checkout_space` and cleanup hold `terminal_primary_checkout_space`, not `HerdrRefused`. The exact terminal/source group stays Kept and its close intent settles as Refused without replay. Linked/non-Git retirement uses only exact `pane.close` under its normal authority/proof gates, never `workspace.close`. Concrete holds go to the liaison rather than a dead retry UI.

For work authorized to run now, choose the stable task UUID and create then schedule it in the same tool invocation; ordinary task authoring never implicitly schedules backlog. Checklist step add/rename/check/move/remove and `dependencies-set` may omit `--revision`: CLI reads the current task revision before one fenced CAS submission, while explicit revisions remain honored. Dependency editing still requires `--doc-revision`; the internal read removes a model turn, not document/current-writer safeguards.

Scheduling/waits/cleanup wake from registered durable notifications and relevant Herdr/ref events, with explicit pending-operation deadlines and quiet transient backoff. No perpetual full-snapshot idle tick is required. Launch acknowledgement, fresh physical readiness, credential activation and first model response must be measured separately: under-ten-second launch and under-five-second communication are desired targets, not current verified timings.

## Durable Space Notes

Each Notes UUID owns `scratchpad.md`, `todos.md`, `decisions/<decisionId>.md` and `comments/<todoId>/<commentId>.md`; registry/lock metadata is under `.cockpit/`. Opening an unbound Space creates nothing; create and attach are explicit. POSIX UI recipes pin both root and UUID and escape shell-sensitive paths. In an uninstalled checkout use `target/debug/cockpit` instead of installed `cockpit-cli`.

```sh
cockpit-cli notes --current target
cockpit-cli notes --current target --create
cockpit-cli notes --current target --attach UUID

# Resolve once; use the returned UUID and the same configured root thereafter.
COCKPIT_NOTES_ROOT='/persistent/cockpit/notes' cockpit-cli notes --notes UUID scratchpad read
COCKPIT_NOTES_ROOT='/persistent/cockpit/notes' cockpit-cli notes --notes UUID todo list
COCKPIT_NOTES_ROOT='/persistent/cockpit/notes' cockpit-cli notes --notes UUID kanban list
COCKPIT_NOTES_ROOT='/persistent/cockpit/notes' cockpit-cli notes --notes UUID decision list
COCKPIT_NOTES_ROOT='/persistent/cockpit/notes' cockpit-cli notes --notes UUID comment list --todo TODO_ID
```

Use `--id ID --expected-revision REVISION` for stable-ID task mutations, or `--ref 'L<n>@sha256:…'` for an unadopted/ambiguous source task. Scratchpad, decision and comment edits use their read revision. Pinned content commands work without Herdr; `--current` requires a real current pane. Root/Space operations do not authorize arbitrary content paths. See [DECISIONS](../DECISIONS.md) for re-read after unknown outcomes and advisory-lock CAS limits.

## Providers and deployment

Each provider declares `kind`: `github`, `gitlab`, `gitea`, `jira` or `confluence`; neither ID nor executable basename chooses the adapter. Forge kinds require a CLI `executable` and retain external CLI login; Gitea can name an existing Tea `login`. Jira/Confluence use Cockpit's GET-only HTTP transport and OS-vault tokens, and reject `executable`/`login`. Cockpit performs no remote writes and never rewrites the user's configuration automatically.

```toml
[[providers]]
id = "jira"
kind = "jira"
base_url = "https://nnexai.atlassian.net"     # deployment defaults to cloud

[[providers]]
id = "confluence"
kind = "confluence"
base_url = "https://nnexai.atlassian.net/wiki"

[[providers]]
id = "wiki-dc"
kind = "confluence"
base_url = "https://confluence.example.com/confluence"  # defaults to data_center

[[providers]]
id = "gitlab"
kind = "gitlab"
base_url = "https://gitlab.com"
executable = "glab"

[[providers]]
id = "github"
kind = "github"
base_url = "https://github.com"
executable = "gh"

[[providers]]
id = "my-forge"
kind = "gitea"
base_url = "https://forge.example/gitea"
executable = "tea"
login = "my-existing-tea-login"
```

`deployment` applies only to Jira/Confluence: explicit `cloud` or `data_center` overrides host-based defaults. Case-insensitive `.atlassian.net` hosts default to Cloud; other hosts to Data Center. Authentication kind does not select deployment. Confluence Cloud requires base path exactly `/wiki`; Data Center retains a configured context path such as `/confluence` or `/jira`. No Jira initialization file or Confluence profile is used.

Jira Cloud uses REST v3 enhanced-search token paging; Data Center uses REST v2 offset paging. Confluence Cloud uses v2 under `/wiki`; Data Center uses v1 under its context path. [DECISIONS](../DECISIONS.md) covers selected-instance/canonical-URL authority, credential failures, safe redirects/paging, attachment identity and private publication. Explicit refresh may change locally converted Confluence Markdown; unchanged follow versions do not mass-rewrite pages.

## Provider token storage and entry points

Tokens are stored in the running user's OS vault: Linux Secret Service through [cockpit-secrets](../crates/cockpit-secrets/src/lib.rs); the compiled macOS Keychain backend remains unverified. [credentials.rs](../crates/cockpit-core/src/credentials.rs) owns provider-instance identity, status, validation, caching and timeouts. Tokens are write/replace/remove only, never read back through the UI. One vault item belongs to each provider ID and `base_url`; changing that URL makes the old token inapplicable. Forge CLI credentials remain external; their Cockpit token status is `unsupported`.

Current Library entry points use singular **Provider token…**: Library actions menu, provider row, Jira/Confluence item menu, Add-dialog sign-in failure, Jira attachments panel and an issue's missing-token state line. Commands uses plural **Provider tokens…** in its Library group. The dialog title remains **Provider tokens**.

Label owners are [LibraryViewerDialogs.tsx](../src/app/library/LibraryViewerDialogs.tsx), [LibraryTree.tsx](../src/app/library/LibraryTree.tsx), [LibraryItemHeader.tsx](../src/app/library/LibraryItemHeader.tsx), [AddContextSourceStep.tsx](../src/app/library/AddContextSourceStep.tsx), [AddContextOptionsStep.tsx](../src/app/library/AddContextOptionsStep.tsx) and [commands.ts](../src/app/shell/commands.ts).

Choose **Personal access token (Bearer)** or **Email and API token (Basic)**. New forms default to Basic for resolved Cloud and Bearer for Data Center, including explicit overrides; both remain selectable and replacements retain stored kind. A colon in the Basic email produces an inline error without clearing the token. Status says **Token stored** plus kind, **No token stored**, or **Keyring unavailable**. Closing a dialog restores the opener or nearest surviving ancestor; attachment headers wait for token status before claiming download state.

Vault calls time out at 20 seconds and values are cached per process; edits in Seahorse/KWallet appear after restart. Removal blocks reads until another token is stored, but existing Library items remain. Linux verification guidance uses an isolated `dbus-run-session` keyring; historical exercised coverage and platform limitations are in [verification-log](verification-log.md), not a promise of current native/macOS coverage.

## Library synchronization and traversal values

Jira/Confluence synchronization is enabled while native Cockpit or `cockpit serve` runs. `[library_sync]` is separate from `[limits]` and never raises existing page/item limits. Defaults:

```toml
[library_sync]
enabled = true
delta_minutes = 60
lag_allowance_minutes = 5
overlap_minutes = 30
inventory_hours = 24
audit_days = 7
related_hours = 24
background_min_interval_seconds = 10
background_in_flight = 1
hourly_request_cap = 300
```

Inclusive bounds are: `delta_minutes` 1–10080, `lag_allowance_minutes` 0–1440, `overlap_minutes` 0–10080, `inventory_hours` and `related_hours` 1–8760, `audit_days` 1–365, `background_min_interval_seconds` 1–3600, `background_in_flight` 1–32 and `hourly_request_cap` 1–100000. Validation is in [config.rs](../crates/cockpit-core/src/config.rs).

The coordinator starts after one minute and wakes every minute, coalescing overdue schedules. Jira uses epoch-millisecond `[lower, upper)` JQL windows. Confluence fixed `lastmodified` CQL envelopes round outward to minutes and widen by UTC ±14 hours, a discovery superset rather than an exact timestamp filter. Equal revisions/hierarchy skip bodies; default daily inventories backstop membership/hierarchy. Durable checkpoints live beneath the Library at `.cockpit/sync/state.json`; [DECISIONS](../DECISIONS.md) defines boundary, retries, accumulation, audits and incomplete-discovery safety.

Background pacing defaults to one request per 10 seconds, one in flight and a rolling 300-request hourly budget per origin, shared across provider instances within each process. Manual requests share that process's origin pacer with two in flight and one-second spacing. Interactive priority, server cooldown and `Retry-After` govern both lanes; a separate CLI process has its own in-memory pacer. Mounted visible listings check the first-page generation and recorded follow sync status roughly every 60 seconds; manual refresh remains distinct. Followed rows show an allowlisted reason for the last recorded failure or deferral and the last successful sync as **Last synced** when a time is known (visible with a reason/failure, otherwise in the tooltip and accessible name). Listing-only `sync` is optional and contains `last_success_at` (epoch-millisecond string or null) and `reason` (human-safe text or null); it exposes neither raw provider errors nor a next-attempt time. Missing or unreadable recorded state is unknown, not a successful sync; the row can still use its recorded successful add/manual refresh time. These status reads do not change scheduling or add a refresh-now action. Safety and publication rules belong in [DECISIONS](../DECISIONS.md).

`reference_depth` is 0–5 on add requests and stored items/follows, with `included_by` reasons. Traversal budgets are Single: 32 items, 8 MiB, 60 seconds; Query: 100 items, 16 MiB, 180 seconds. Confluence page adds and space follows stay at depth 0. Jira follows use the configured `library_space_pages` bound; key moves retain identity/refusal semantics, not numeric-ID migration. Live-follow removal grace is 14 days and edited Library items are retained. See [DECISIONS](../DECISIONS.md) for complete-enumeration, lock/ref and seed-preservation requirements.

Attachments are metadata-only by default; downloads are explicit. Jira's bounded attachment list reports `source_attachments_partial` for malformed or beyond-256 entries. PNG/JPEG previews are supported; PDF has no active renderer and SVG/HTML never execute. Attachment identity and removal behavior remain in [DECISIONS](../DECISIONS.md).

## OMP quota executable

`COCKPIT_OMP_EXECUTABLE` overrides `[quota] omp_executable`. With neither, Cockpit finds executable `omp` through `PATH`, then `$HOME/.local/bin`, `$HOME/.bun/bin`, `/opt/homebrew/bin`, `/usr/local/bin`, `/home/linuxbrew/.linuxbrew/bin`, then falls back to bare `omp`. Discovery never overrides an explicit selection, modifies `PATH` or runs shell startup files. Existing `PI_CODING_AGENT_DIR` authentication overrides are inherited by the child. Restart Cockpit after configuration changes; usage-source-not-found means launch failure, not a JSON parse failure.

## Native window and Browser settings

```toml
[window]
scale_factor = 1.0
decorations = true
```

`scale_factor` is finite, 0.2–10.0 inclusive, and controls WebView page scale. `decorations` controls title bar/borders. These defaults apply on every native platform; the browser client ignores them.

Install Node and Playwright CLI before opening Browser. The host embeds the capture helper and attaches the same CLI-managed Chromium session, not a second browser for the inline view. Environment overrides `[browser]` settings:

| TOML key | Environment override |
| --- | --- |
| `playwright_cli` | `COCKPIT_PLAYWRIGHT_CLI` |
| `chromium_executable` | `COCKPIT_CHROMIUM_EXECUTABLE` |
| `node_executable` | `COCKPIT_NODE_EXECUTABLE` |
| `browser_helper` | `COCKPIT_BROWSER_HELPER` |
| `playwright_core` | `COCKPIT_PLAYWRIGHT_CORE` |
| `default_url` | `COCKPIT_BROWSER_DEFAULT_URL` |

`playwright_core` names the package paired with the CLI. `default_url` is validated at load and defaults to `about:blank`; fresh leaf creation uses it, while CLI Open/existing-leaf Reconnect retain attach/new-page semantics. Missing/invalid prerequisites are checked lazily at attach and name the corresponding variable/key; [native-install](native-install.md) explains diagnostic distinctions and retry. Browser lifetime, current-run feedback and identity-proven cleanup rules are in [DECISIONS](../DECISIONS.md).
