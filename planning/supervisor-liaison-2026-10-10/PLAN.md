# Supervisor as liaison: improvement plan

Status: proposed implementation, not implemented or runtime-verified. Based on the [cleanup retrospective](../../docs/retrospectives/2026-10-10-cleanup-orchestration.md), its Firstmate discussion, and the current repository. [UX.md](UX.md) supplies the interaction contract. No live sessions, settings or resources were modified to make this plan.

## 1. Outcome

Start a supervisor, give it work, and let it delegate. Workers implement, use their own subagents, make ordinary decisions and perform affected checks. The supervisor coordinates outcomes and integration—not every tool, plan revision, message receipt or compiler slot. Restarting a machine, Cockpit, an OMP conversation or a compatible CLI must not strand the work. Completed managed workers and their safely disposable resources clean themselves up.

This is the entire retro's scope, with relaxed bindings and recovery prioritized. It is **not** a recovery-only patch, a new approval framework, a general remote-agent platform, or a rewrite of Herdr. Reuse canonical task Markdown, the existing orchestration store/private runtime owner, current worker placement, and Tasks/Graph/Dependencies.

## 2. What the evidence says

| Current mechanism | Evidence | Consequence / change |
|---|---|---|
| An OMP tool call refreshes CLI/run state and checks worker execution before the tool runs | `integrations/omp/extension.ts:30-42`; `integrations/omp/cliCall.ts:70-97` | Remove this global authorization hook, not merely cache it. Ordinary local tools must work offline. |
| Authority is tied to native session/process, with only same-process conversation rollover | `crates/cockpit-core/src/orchestration.rs:719-913`; `mutate/runs.rs:115-146` | Separate durable job authority from its replaceable execution attachment. |
| Workers need Prepare → Ready exact plan → Execute | `mutate/grants.rs`; `messages/report.rs:140-212`; protocol `RunStage` | Replace with one start operation. Startup readiness is a runtime observation, not permission from another model. |
| Working task edits require the Ready initialization receipt | `mutate/tasks.rs:268-299` | Allow ordinary advisory-plan/task updates on the same worker; no successor actor. |
| Model instructions require inbox pull/process/ACK | `integrations/omp/wake.ts:9,114-146`; `CODE_GUIDE.md:273` | Move transport consumption/cursor bookkeeping to the extension/runtime; only decisions/results need model attention. |
| The private owner starts the dispatcher already | `crates/cockpit-host/src/orchestration_runtime.rs:90-123` | Extend this owner, rather than inventing another daemon. |
| Automatic accepted-worker process/pane retirement already exists | host `orchestration_runtime/retire.rs`; core `mutate/retirement.rs` | Preserve and extend this useful primitive. It is not the same thing as resource cleanup. |
| The root prompt says “Never tear down resources”; root tools provide no task-resource cleanup operation | `integrations/omp/extension.ts:67`; `integrations/omp/tools/manage.ts` | Remove the prohibition and provide supported cleanup with standing creation-time authority. |
| Worktree removal depends on a matching live/historical Space association and rejects multiple matching receipts | `crates/cockpit-core/src/projects/teardown.rs:231-305`; `project_teardown.rs` | Reconcile identity drift, missing Spaces and equivalent duplicate records; Git cleanup must not need a live Herdr incarnation. |
| Shared Cargo scheduling is cleanup-plan/harness policy, not a product queue | retrospective; current runtime dispatcher; `planning/code-cleanup-2026-10-09/PLAN.md` | Add a small resource queue; do not pretend it already exists or ask a model to manage build-slot approvals. |

Core paths abbreviated above are relative to `crates/cockpit-core/src/orchestration/`; host runtime submodules are relative to `crates/cockpit-host/src/`. Some historical cleanup receipts require additional adapter inspection during implementation; unsupported upstream behavior is not assumed.

The retro recorded 25 workstreams plus three integration runs, roughly 26h47m to source delivery, 469 worker messages and 2,091 root tool calls. These are observations, not performance targets or controlled benchmarks. The shutdown exposed brittle identity; excessive ceremony and verification were already costly before it.

## 3. The simpler primitives

### Durable job, replaceable attachment

Keep a durable root/run/task identity and checkout ownership. OMP session IDs, executable paths, PIDs, launch attempts, boot IDs and Herdr endpoints describe **attachments**, not the job's continued right to exist.

Use a local, role-scoped job credential for orchestration calls, issued by the existing owner: root can coordinate its subtree; worker can update its own task, report, use local subagents and request resource slots. Do not pass root authority to workers or subagents. Store credentials privately, never in task Markdown, progress, logs or user-visible diagnostics. This is same-user local coordination, not a hostile-process security sandbox. Preserve browser/native operator origin checks and cross-root authorization boundaries.

Routine API authorization checks job/role and current checkout claim, not an exact native-agent incarnation. Optional caller provenance remains useful diagnostics. Fresh physical process/pane/endpoint checks remain mandatory **at the point of stop, close or deletion**, not for each source edit or inbox read.

A compatible CLI is a transport client. Resolve it from the currently available installation, with an explicit protocol-version check and one useful incompatibility error. A deleted launch-time executable must not permanently disable the job. The execution connection can reconnect or change main conversation without replacing the task. No user installation/global OMP settings change is implied.

### One start, one checkout writer

The liaison creates a task-level worker start request with scope, placement, prerequisites and normal cleanup authority. The existing dispatcher sets up and launches it once prerequisites and checkout ownership permit. No Prepare/Execute grants or exact-plan permission remain.

Preserve atomic store/CAS and dispatch idempotency. A checkout claim is exclusive: expiration or missing heartbeat alone does not permit a replacement writer. Reattach to a living worker; otherwise prove the old writer exited, or perform verified cooperative stop/quiescence before handover. On reboot, distinguish a proven boot change on the same machine from an unreachable/different machine or merely changed endpoint. The latter is not proof of exit. Revoke the previous attachment's orchestration claim before activating its replacement.

This prevents managed duplicate writers; it cannot sandbox arbitrary shell writes by an uncooperative same-user process. Therefore local editing can continue through temporary Cockpit disconnection, and reassignment of that checkout must wait for actual handover—not lease expiry.

### Mutable scope, meaningful decisions

Canonical task description/checklist and worker work plan are advisory, mutable records with normal CAS. Ordinary refinement stays on the same worker. Existing Follow-up sends an instruction and updates the task scope in one durable operation; worker-authored refinements update scope without sending an instruction back to itself. Keep explicit user answers only for outcome-changing ambiguity, a real unresolved user tradeoff, or new external/destructive commitments outside standing authority.

Do not make every instruction an answer, or every delivered answer a resolved decision. Transport receipt and semantic response remain distinct. Accepted task history remains protected; a genuinely new outcome becomes new work rather than silently rewriting an accepted result.

### Runtime bookkeeping, model coordination

The owner reconciles launch/reconnect, prerequisites, transport delivery, worker retirement and cleanup. The extension consumes messages durably, advances transport cursors and delivers actionable instructions/questions/results. A wake is a coalesced latest-state digest, not an obligation to perform a list/ACK conversation. Explicit question answering, result review and source acceptance remain semantic actions; routine ACK becomes invisible bookkeeping.

During disconnection workers keep editing/checkpointing/committing locally. Persist outgoing results/instructions with stable operation IDs and replay once the owner returns. Delivery deduplication must survive interruption between write, delivery and cursor advance. Replayed results still have to satisfy task-version and checkout ownership checks; offline progress must not silently authorize a conflicting takeover.

Use revision notifications for changed state. A bounded cross-process/liveness fallback may remain, but do not rebuild whole snapshots or wake the model for unchanged observations. Latest progress replaces latest progress; preserve decisions, scope changes and result history, not every transient “still working” report.

### Source completion, integration and proportional verification

A worker's source-complete result names committed output and focused affected checks. Accepting it means its output is ready for integration, not that every product surface has already been smoke-tested. Record integration separately: `pending`, `integrated` with target commit/method/output mapping, or `not_applicable` for a non-code deliverable. Integration carries the changed-contract verification outcome; it is not fabricated from source acceptance.

For mechanical extraction, workers perform cheap relevant checks and return a patch/commit. The integrator builds the combined changes and exercises affected runtime paths once per integration batch. Public-contract changes require affected contract checks; native-only changes need a native run; externally serviced changes need a live service run. A pure source split does not warrant a new native-caller proof, private LSP experiment, namespace attestor or NoModel harness.

Add a narrow deterministic resource queue for shared checkout/build-artifact use. Workers request/release a resource key; the owner selects FIFO-ready requests and handles verified abandoned holders. The worker/integrator executes its own build: this is **not** a runtime accepting arbitrary shell build commands. Share only artifacts known to come from the appropriate source/input identity; source worktrees with incompatible outputs cannot blindly share Cargo artifacts. Prefer one integrator build over private frozen binary pairs per extraction worker. A stale holder cannot be stolen while its build process is still running.

Batch small adjacent extractions into one worker task/checklist; split only where parallel ownership or a useful independent outcome warrants it. Function-size counts guide planning, not pass/fail acceptance. Do not change unrelated quality/mutation tooling on the strength of an unverified claim about its thresholds.

### Owned cleanup, independent of the root conversation

At worker creation record owned-versus-borrowed resources and grant normal cleanup for that task. Root creation authorizes its managed subtree; it does not authorize deleting arbitrary resources. The owner runs cleanup at completion/cancellation, startup, reconnection and relevant state changes, even when the root model is absent. The root can request the same idempotent cleanup operation; optional UI **Finish cleanup** retries it without another typed confirmation.

Separate effects and receipts:

1. **Worker/process/tab:** retire accepted idle or safely quiesced cancelled/failed managed workers. Preserve drafts, user activity, live subagents and foreign/other terminal consumers. A tab is not disposable merely because one worker finished.
2. **Worktree and branch:** remove only owned resources after their source is demonstrably integrated and no additional dirty/unmerged work or live writer remains. Source acceptance alone is insufficient. Failed/cancelled incomplete output is retained; do not equate “unused” with “safe to delete.”
3. **Space:** remove an owned task Space only if it has no user notes/drafts/browser state/other consumers that must survive. Never remove a borrowed project Space. Treat an already-missing owned Space as absence, not a blocker to independent Git cleanup.
4. **Scratch/fixtures:** register a disposable fixture as one runtime-owned resource set with its normal shutdown operation, so the owner—not the model—stops its verified owned processes and removes its scratch. Record per-effect outcomes inside that set; protect foreign processes, user state and shared/live consumers. Remove scratch only after its managed consumers are quiesced, with verified identity/containment. An arbitrary untracked path is not “scratch.” Explicitly owned generated scratch may be removed before rechecking worktree cleanliness; user/unclassified files remain.

For adapted cherry-picks, plain ancestry cannot prove integration. Record the accepted output-to-target mapping and integration verification explicitly. Equivalent patches may use deterministic patch equivalence; adapted output requires the integrator's explicit verified mapping. Recheck for commits or changes added since that receipt before deletion. Never force-remove to make a task green.

Reconcile old endpoints/boot receipts against the current resource. Equivalent ownership records may be consolidated by resource identity; genuinely conflicting provenance stays retained with one reason. Inspect Herdr's actual missing-Space/worktree adapter semantics before selecting its removal path; do not invent an unsupported API or fall back to unreviewed raw commands. Use the supported local Git operation for Git-only cleanup if the current adapter needs a live Space, with the same ownership/concurrency safeguards.

Persist per-resource cleanup intent and outcome. After an interrupted external effect, observe actual state and reconcile before resuming; never blindly replay stop/remove. One blocked resource does not prevent independent resources from cleaning up. Keep a compact durable task/output/cleanup record after retiring the resources.

## 4. Dependency-ordered delivery: five coherent waves

These are end-to-end implementation batches, not five model approval gates. Each wave must work for its named scenario before integration; worker micro-extractions are internal checklists.

| Wave | Change and owners | Observable acceptance |
|---|---|---|
| **1. Durable execution and unattended resume** | Protocol/store: job credential, replaceable attachment and checkout claim; core `orchestration.rs`, `caller.rs`, `mutate/runs.rs`, `reviewed.rs`; host CLI `caller.rs`/`context.rs`, runtime/dispatcher; OMP `identity.ts`, `cliCall.ts`, main binding. Remove the global `extension.ts` tool-call gate. Add runtime resume and current CLI/version routing. | Compatible CLI replacement and conversation change keep the same task; temporary offline local work continues; owner/boot restart resumes the same checkout without duplicate writer. |
| **2. One start and autonomous liaison** | Protocol actions/stages; core `mutate/grants.rs`, `dispatch_records.rs`, `messages/report.rs`, `mutate/tasks.rs`, prerequisite/intents handling and projection; CLI args/subcommands; OMP tools and injected guidance. Remove Prepare/Execute and exact-plan locking; enable mutable scope and worker-local subagents. UI Start reuses/resumes a selected root; secondary Start another is explicit. | One request spawns workers through source results with no routine grant/confirmation; Working scope updates use the same actor; a real outcome-changing question still reaches the user. |
| **3. Quiet transport and integration-sized verification** | OMP `wake.ts`, inbox/tools, durable local outbox; core message/inbox publication and host wait/notification paths; existing owner resource queue; result/integration DTOs, `mutate/intents.rs`, task projections; worker/root brief templates. | Progress updates coalesce; a delivered question is not auto-answered; disconnect/replay deduplicates; integration builds once for a mechanical batch while workers report focused source checks. |
| **4. Autonomous resource cleanup** | Extend core retirement and project teardown/ownership records; host `orchestration_runtime/retire.rs`, dispatcher and project adapters; expose subtree-scoped cleanup CLI/tool/transport operation; remove “Never tear down resources.” Add integration mapping, per-resource reconciliation, runtime-owned fixture sets with normal shutdown, and startup cleanup. | Root offline: accepted workers retire; integrated owned worktree/branch/Space/scratch clean and owned fixtures shut down as a set; missing Space, stale receipts, equivalent duplicates and adapted cherry-picks are handled; user/failed/unmerged work survives. |
| **5. Surface cutover and existing-job continuity** | Supervisor hooks/actions/attention/lanes, Start/Resume flow, teardown/recovery surfaces and diagnostics per [UX.md](UX.md); generated TS and client decoders; current docs and portable OMP guides. Complete callers/tests removal and active-record conversion described below. | Start once → delegate → integrate → clean, then restart/resume, on the actual browser/native surfaces without routine prompts, focus theft or lost drafts. |

Dependencies: 2 uses 1; 3 uses 1–2; 4 uses 1 and the integration fact in 3; final surface/continuity acceptance follows all four. Independent UI/static draft work can happen early, but do not expose buttons backed by unimplemented contracts. Resource cleanup safety must not be postponed past delivery of automatic cleanup.

## 5. Cutover inventory and continuity

Implementation must begin with LSP references for exported changes, then migrate every caller. This inventory is a map, not a claim of exhaustive reference results:

- Rust DTOs: `crates/cockpit-protocol/src/orchestration.rs`; project cleanup DTOs; descriptor generation and `src/protocol/generated/v1.ts`. Generate, do not hand-maintain a second schema.
- Core: orchestration admission/caller/store; runs/grants/tasks/intents/retirement; launch/dispatch records; report/inbox; prerequisites/projection. Delete obsolete grant actions, plan equality authorization and same-process-only binding logic. Initialization status may remain but carries no permission.
- Host: `cli_orchestration/{args,caller,context,wait}.rs`; `orchestration_runtime.rs` and its dispatch/retire modules; `transport/operations/orchestration.rs`, project handlers/registry; Herdr launch/env and project adapters.
- OMP: `integrations/omp/{extension,identity,cliCall,wake}.ts`, main/subagent bindings, management/message tools, bundled orchestration integration, injected root/worker guidance. Remove per-tool permission queries and manual ACK instructions from **every** installed/bundled source, not only one entry file.
- Frontend: supervisor Start/actions/projection/attention/lane/status/hooks; project cleanup dialogs/pending panel; browser/native client validation. Preserve terminal-chat work entry, Tasks/Graph/Dependencies and existing drafts/focus contracts.
- Tests: service/caller/grant/launch/retirement/project teardown tests; protocol/client decoder and OMP wake/identity tests; supervisor interaction tests. Delete tests of obsolete exact-plan gates and incidental wording, replace only with consumer-visible behavior/safety checks.
- Documentation: `CONTEXT.md`, `DECISIONS.md`, `CODE_GUIDE.md`, `docs/supervisor-surfaces.md`, affected CLI/configuration documentation and portable orchestration skills, plus a concise changelog entry. Retrospective and verification history remain historical evidence.

Do not wipe existing tasks/worktrees to obtain a clean demo. At the release boundary make an atomic, resumable conversion of active records to the current schema, preserving task IDs, committed outputs, checkout/resource ownership, unanswered questions, pending messages and uncertain-effect receipts. Existing Prepare/Ready/Execute state becomes startup/active status, not renewed approval. Reconcile live attachments before assigning a writer; unknown attachments/resources remain preserved. Keep historical messages as history, not executable legacy grants. No permanent old-action aliases or dual lifecycle implementation remain.

## 6. Verification proportional to these changes

For **this planning deliverable**: a dry-run interpretation of the cases below and a single independent high-severity review of authority, concurrency and data-loss risks. No product builds or live-resource mutations are required or claimed.

For **implementation**: focused contract tests for uncertain boundaries, then one disposable affected-path end-to-end lifecycle smoke using `python3 scripts/verify/ui_polish_runtime.py start|stop <root>`. Exercise the actual OMP root/worker integrations and browser surface; native transport/effect changes also need one native run against that fixture. Do not turn each source worker into its own proof-infrastructure project. Never run interruption/cleanup experiments on a persistent user session.

| Case | Required outcome |
|---|---|
| A — Start and delegate | One supervisor request starts two workers, their local subagents return source results, integration completes and eligible resources clean; no routine model/user confirmation loop. |
| B — CLI/session replacement and offline work | Compatible CLI or new main conversation retains job authority; offline editing/checkpoint/commit works; outbox replay is deduplicated; a delivered question remains unanswered until explicitly answered. |
| C — Battery/boot interruption | Resume the same tasks/worktrees from durable state; no renewed Ready/Execute; no duplicate writer. |
| D — Old writer still alive | Replacement denied until reattachment or verified stop/quiescence. Heartbeat/TTL expiry alone is insufficient. |
| E — Plan change | Ordinary Working refinement updates the same task/actor; outcome-changing ambiguity escalates once. |
| F — Many progress reports | Latest state replaces older progress; meaningful revision causes a coalesced wake; unchanged status does not repeatedly wake the model. |
| G — Shared builds | One integration build for a mechanical batch; queued incompatible writers/builds do not race; no model slot grants or per-worker frozen host/native pairs. |
| H — Accepted, not integrated | Process/tab can retire safely, but source branch/worktree remain until integration is proven. |
| I — Root offline and historical cleanup drift | Owned integrated worktree/branch/Space/scratch clean without root; missing Space, stale endpoint, equivalent receipts and adapted cherry-pick are reconciled. |
| J — Protected resources | Drafts, user changes, borrowed Spaces, live/foreign consumers and unknown scratch stay intact with a clear reason; unrelated safe resources still clean. |
| K — Cancelled/failed work | Checkpoints and incomplete output remain; safely quiesced owned processes can retire without discarding source. |
| L — Cleanup interrupted midway | Resume from intent/outcome receipts and actual state; no blind repeated stop/remove or false success. |

A+C+H+I form the main lifecycle smoke; targeted disposable fixtures/contract tests cover the dangerous boundaries without twelve separate full environments. UI focus/draft checks follow UX acceptance alongside that smoke. Use actual tool/process/cleanup observations as evidence, not source-string assertions or mock echoes.

## 7. Performance learning, not another gate

For comparable tasks capture existing logs/counters: time to first worker and source completion, routine control messages/root wakes, verification wall time and bytes, recovery interruption, and owned resources left afterwards. Compare a batched mechanical refactor before/after if a suitable fixture is available; do not claim causation from the historical wall-clock alone. Reporting is enough: no metrics dashboard, new approval thresholds or mandatory measurement ceremony.

The desired reduction is structural: no tool-level CLI checks, no Prepare/Execute round trips, no model ACK work, fewer small workers, one integration-sized build/smoke, and automatic recovery/cleanup. Do not compensate by adding a larger root instruction manual.

## 8. Firstmate: borrow the role, not the bureaucracy

[Firstmate](https://github.com/kunchenguid/firstmate), its [architecture](https://github.com/kunchenguid/firstmate/blob/main/docs/architecture.md) and [startup recovery skill](https://github.com/kunchenguid/firstmate/blob/main/.agents/skills/session-start-recovery/SKILL.md) are useful inspiration for a liaison with autonomous crews, durable restart context and non-model watchers that suppress benign events. Its [project-management skill](https://github.com/kunchenguid/firstmate/blob/main/.agents/skills/project-management/SKILL.md) also carries substantial harness/gate machinery. Borrow the separation of responsibilities and concise recovery digest; do not import another exact-plan, ACK, no-mistakes approval stack. These are source-reading observations, not a runtime evaluation of Firstmate.

## 9. Retro coverage check

| Discussion recommendation | Planned change |
|---|---|
| Liaison, not micromanager; workers use subagents | Sections 1/3; wave 2 |
| Relax CLI/native-instance bindings; battery recovery | Durable job/attachment and writer handover; wave 1 |
| Remove routine confirmations and per-tool gating | One start, role-scoped API calls; waves 1–2 |
| Mutable plans without successor workers | Task scope/Follow-up cutover; wave 2 |
| Event-driven/coalesced progress; ACK as bookkeeping | Quiet transport/outbox; wave 3 |
| Deterministic shared build scheduling | Narrow resource queue, not a shell executor; wave 3 |
| Source completion distinct from integrated verification | Required integration fact and cleanup eligibility; waves 3–4 |
| Proportional verification; avoid bespoke proof work | Changed-contract checks and one combined smoke; sections 3/6 |
| Batch mechanical slices; size counts advisory | Worker brief/default decomposition; waves 2–3 |
| Automatic cleanup; no supervisor refusal | Standing scoped authority and runtime owner; wave 4 |
| User sees outcomes rather than binding/gate repairs | Existing surfaces, simple Start/Resume and independent cleanup; wave 5/UX |
| Learn from Firstmate without importing micromanagement | Section 8 |
