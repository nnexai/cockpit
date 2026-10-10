# Cockpit cleanup/refactor retrospective — 9–10 October 2026

## Executive assessment

**The large-model-plan → orchestrator → parallel-workers approach was reasonable. The execution model made a mostly structural cleanup behave like a high-assurance release and an orchestration research project at the same time.**

The battery interruption exposed a recovery failure, but it did not cause all the inefficiency. Shared-build contamination, repeated evidence repair, and a busy approval/report loop were already present before shutdown. After recovery, the supervisor repeatedly solved verification-infrastructure problems instead of finishing the refactor. The owner ultimately had to redefine “enough proof.”

The recommendation is not to add another checklist. It is to **remove routine gates, make durable work independent of agent/process identity, and move ordinary scheduling and resource management out of model conversations.** Keep strong protection at destructive and genuinely ambiguous external-effect boundaries, not around every local development step.

This is an analysis and design recommendation, not an implementation or a claim that these changes have been tested.

## Scope and evidence

The focal cleanup started on 9 October and was reported delivered to main on 10 October. I inspected the original planning conversation, both cleanup supervisor journals, durable messages/results for all 25 workstreams and three integration runs, recovery evidence, and the earlier 7–8 October guidance retrospective. Focused scouts supplemented the plan and recovery analysis. Individual worker journals were sampled for recovery evidence, not exhaustively read line by line.

All times below are UTC. Counts come from retained records, not estimates of model compute. A session's age, a last-updated timestamp, or a long wait does not establish active work time. The battery death is owner-reported; the journals establish shutdown, lost bindings and manual recovery, not the exact power-loss time.

### Recorded scale

| Measure | Observed value | Interpretation |
|---|---:|---|
| Planned workstreams | 25 | Includes more than file splitting |
| Cleanup worker runs | 28 | 25 workstreams plus three integration runs; excludes internal subagents |
| First worker creation → supervisor completion announcement | 26h 47m | Includes interruption and waiting; not 26h 47m of coding |
| Messages into those workers | 469 | 339 instructions, 54 answers, 37 preparation briefs, 37 work briefs, two cancellation requests |
| Reports from those workers to the supervisor | 629 | Includes internal-child reports attributed to worker runs; not 629 completed jobs |
| Direct supervisor tool calls through completion, across the two journals | 2,091 | Excludes nested workers; includes reads, mail, dispatch and ordinary tools |
| Of those, inbox tool calls | 612 | A coordination-volume signal, not proof every call was unnecessary |

The volume does not itself prove waste. The content does: completed code repeatedly waited for new proof, new grants, or a repaired test harness.

## What happened

| Time | Recorded event | Significance |
|---|---|---|
| Oct 9 05:41 | Plan committed as `92290c6` | Detailed ownership and dependencies were available before execution |
| 05:46 | First cleanup workers created | Start of measured execution window |
| 06:02 | Owner says integration worker did not receive instructions | Delivery/initialization friction appeared immediately |
| 06:32 | Supervisor broadcasts Cargo hold after cross-worktree DTO contamination | Build architecture created a new serialized coordination problem |
| 06:46 | Owner reports removing cache/tmp because of almost 200 GB of verification data | Storage pressure was already affecting work, before shutdown; this is not a measured 200 GB generated solely by this cleanup |
| 11:31 | Owner: “machine got shut down. resume” | First explicit shutdown/resume marker |
| 12:42–12:54 | Recovery instructions issued to retained workers | Source preserved, but execution held pending new binding, Ready and grants |
| 13:12–13:19 | Replacement supervisor binds; fresh Execute grants and renderer permissions issued | Recovery required explicit reconstitution of the control plane |
| 13:14 | Owner identifies workers as “stranded” and asks that existing worktrees be reused | Runtime continuity did not follow durable task continuity |
| 19:06–19:12 | Integration transferred to a successor in the same checkout because its Working plan could not be amended | A non-conflicting scope update required actor replacement |
| Oct 10 03:46–05:39 | Private LSP approvals, namespace attestation, native harness repair and new runtime constructors remain on the critical path | Verification itself had become substantial engineering work |
| 05:44 | Owner calls this a “multi day verification slop fest” | Explicit correction to the execution objective |
| 05:44–06:40 | Expanded proof work stopped; four workstreams produce successful Results | Smaller affected-consumer checks unlock delivery |
| 08:33 | Supervisor reports all 25 streams accepted and clean main at `f87d39d` | Refactor completion, distinct from later installation/push/resource cleanup |

The later cleanup also hit stale endpoint ownership, duplicate Space claims and retained fixture processes. That is a lifecycle consequence worth learning from, but it is not counted as unfinished refactor implementation or extra coding time here.

## Findings

### 1. Recovery preserved the files, but not the ability to continue work

The valuable part worked: original tasks, worktrees, commits and partial source survived. Recovery repeatedly told workers not to replay settled checks solely because of reboot.

The missing part was an ordinary path from “same task, preserved checkout, new process” to “continue.” Durable Working state and Execute grants did not give resumed agents a valid native-main SDK binding. The replacement supervisor inherited a substantial inbox backlog; workers waited for re-reviewed plans, compiler slots and renewed render permissions. The 11:31 resume request precedes the fresh 13:19 continuation grants by about 1h 48m. That interval contains recovery work and cannot all be attributed to idle time.

The retained recovery plan identifies the mechanism more precisely: the root and nine workers recorded the old Herdr endpoint and kernel boot. Reconcile produced identity conflict rather than automatic recovery. Retry preserved the checkout and task but cleared live binding, marked old briefs stale and sent workers back through initialization → Ready → Execute. The root needed an external controller because recovering its own pane would terminate the conversation doing the recovery. These are historical source-analysis findings in that recovery plan, not a new live fault-injection run.

**Design diagnosis:** durable job identity, ephemeral execution identity, permission and messaging are too tightly coupled. A process restart became a workflow restart even when the code objective had not changed. Same-live-process session rollover is not sufficient for a machine reboot.

**Better primitive:** a stable job with a replaceable execution lease. On restart, bind a new incarnation to the existing job/checkout, invalidate old process-specific control, and present the retained checkpoint and latest intent. Resume ordinary local work within the already accepted task scope; renew exceptional external capabilities only when they are actually needed. An offline supervisor should prevent conflicting reassignment, not invalidate every worker's safe checkpoint/report/commit path.

This must still prevent two writers claiming the same checkout. It does not require pretending an old PID is alive or blindly replaying an uncertain external operation.

### 2. Verification was inherited from touched subsystems, not selected by the change

The plan explicitly asked for behavior-preserving decomposition, but also prescribed a full integration gate per wave: frontend tests/typecheck/build, workspace Rust tests, native check and merged smoke. Worker briefs added affected-path runtime obligations. These are not identical checks, but the aggregate was much heavier than structural changes usually need.

Concrete examples:

- WS-08, a frontend App split, retained its original product commit and frontend checks while rebuilding a host and repeating browser acceptance to repair source/binary provenance.
- WS-09, a BrowserPane split, repeated browser and packaged-native annotation/navigation/paste/close proof on unchanged product code after evidence repair.
- WS-16, a Review module split, built host/native and exercised staged/unstaged/untracked snapshots and large UTF-8 pagination in both surfaces.
- WS-24, CSS consolidation, reached a 987-line private native adapter plus failed-session UUID recovery and constructor replanning before its final lean browser smoke.
- Integration pursued a populated NoModel supervisor, disabled-provider catalogs, cross-user-namespace `/proc` access and an external host attestor. Those are real technical questions, but they are not file-splitting acceptance criteria.

There were legitimate reasons for care: WS-03–06 removed compatibility/contracts; WS-14 unified refresh behavior; WS-20 generated validators and exposed real differential-consumer failures. It would be wrong to classify all 25 streams as trivial moves or to dismiss all verification.

The mistake was applying subsystem-level assurance to every structural slice and treating a harness limitation as something that must be repaired before the code could ship.

**Better primitive:** verification belongs to the changed artifact and behavioral delta. A move/extraction normally gets compilation/typechecking plus existing affected checks; one shared integrated smoke can cover composition. Schema removals, generated validation and concurrency-sensitive changes get their actual consumer/negative cases. An unrelated native-test infrastructure failure is an explicit coverage limit, not a new refactor deliverable.

### 3. The build setup manufactured a bottleneck, then models became its scheduler

The plan mandated one shared Cargo target because disk was constrained. Cross-worktree artifact reuse was observed within the first hour. The response introduced private workspace wrappers, a continuously held lock, freeze/hash manifests, compiler-root attribution, worker quiescence censuses and individually granted slots.

The provenance concern was real: a passing command against another checkout's artifacts is not good evidence. The cost came from making every worker build and prove a private host/native pair, then having the model coordinate access to mutable build output.

**Better primitive:** one ordinary build queue owned by a deterministic executor, with isolated output identity. Keep dependency caches shared where safe, but make workspace artifacts/run binaries unambiguous. For structural work, workers can return source-complete patches and focused cheap checks; an integrator builds the combined artifact once. Frontend-only slices should use a known-compatible backend rather than each manufacturing a new backend provenance chain.

If storage cannot support the chosen fan-out, reduce build concurrency or batch source edits. More workers do not accelerate a single serialized compiler resource.

### 4. Frozen exact plans made adaptation unnecessarily expensive

At 19:06, the supervisor recorded that Ready, SendBack and Reconcile could not amend the integration worker's Working plan. It used a successor targeting the existing Space and transferred writer ownership. Preserving the checkout was correct, but changing the actor to update a plan was not intrinsically necessary for this task.

Later, ordinary steps such as local dependency copying, private reference startup, harness repair and fixture access accumulated source-only proposals, exact hash reviews and one-shot grants. These controls often prevented the model from choosing the next harmless implementation step without another round trip.

**Better primitive:** authorize the task's outcome and workspace, not an immutable transcript of its implementation. Keep plans advisory and amendable by the supervisor in place. Reapproval should mean a new destructive action, new external service/resource or conflicting ownership—not a changed helper, local command sequence or newly discovered caller.

A job should not need replacement because its original plan forgot the next integration step.

### 5. Mail and acknowledgement became much of the supervisor's work

The cleanup's retained records contain 1,098 worker-directed messages and worker-to-root reports. The two root journals contain 612 inbox calls through delivery. Reports were often followed by an instruction acknowledging the report, clarifying scope or supplying another narrowly reviewed grant. The root was acting as a scheduler, policy interpreter, evidence examiner and recovery engineer simultaneously.

ACK semantics are useful for questions, handoffs and ownership transfer. Requiring model attention for every status/slot/proof milestone turns parallelism into a serial conversation with one busy model.

**Better primitive:** separate latest-state progress from durable requests requiring a decision. Coalesce status updates; schedule ordinary slots and readiness transitions automatically. Reserve conversational mail for blockers, changed scope, results and genuine decisions. The supervisor should decide what to do next, not manually authorize each step the executor already knows how to do.

### 6. Hard function-length targets created late additional implementation

The final integration initially measured 66 functions over 150 lines, against a target of 40. It then delegated ten code slices; the final recount reached 40, with two over 300. That was real additional refactor work, not simply wasted verification.

The plan's metrics were useful for finding hotspots. As a hard acceptance gate, however, they rewarded extraction even when total callable count grew and reopened the integrated result late. The report honestly records 4,924 → 4,970 callables for that final repair, not a reduction in total functions.

**Better primitive:** use size metrics as a planning signal and review aid, not a universal reason to reject otherwise completed work. Evaluate responsibility boundaries and actual readability. Do not force another fan-out just to cross an arbitrary global threshold unless that threshold is itself the owner's essential outcome.

## The owner's correction provides the strongest comparison

At 05:44 the supervisor explicitly stopped bespoke fixture/NoModel/tracing/namespace/private-compositor proof work. It retained existing failures and passing checks, requested small actual consumer smokes, and accepted honest unrun UI limits.

| Stream | Successful Result after the owner's correction | What changed |
|---|---:|---|
| WS-20 generated validators | 3.2 minutes | Existing code/checks plus minimal affected parser-consumer smoke; large UI matrix left unrun |
| WS-22 host transports | 14.6 minutes | Existing transport checks/builds plus small consumer smoke, not full lifecycle proof |
| WS-23 CLI split | 20.0 minutes | Relevant existing CLI/core checks and bounded smoke |
| WS-24 CSS | 56.1 minutes | CSS completion, one frontend build and actual Commands browser smoke |

This is not a controlled benchmark or proof of a particular speedup: much implementation and checking had already happened. It is direct evidence that expanded proof obligations, rather than missing core code alone, were blocking completion. It also demonstrates that relaxing acceptance did not mean fabricating a full native pass.

## Recommended direction: fewer primitives, not more policies

### Clarified product role: liaison, not micromanager

The owner subsequently clarified the desired supervisor as a **liaison for kicking off work**, pointing to [Firstmate](https://github.com/kunchenguid/firstmate) as inspiration. This sharpens the recommendation: worker autonomy is the normal arrangement, not an exception the supervisor repeatedly grants.

The liaison translates intent into a useful brief, selects workers, coordinates cross-worker dependencies and brings back outcomes or real user decisions. A worker owns its implementation, ordinary command choices, verification proportional to the change, local recovery and delegation to its own subagents. Its specialists can question and review its work without making the top-level liaison approve each exchange.

For example, extracting a helper, fixing an import, choosing affected tests, copying compatible dependencies or adapting a local test driver should not generate supervisor confirmation requests. A conflict over a shared file or compiler resource needs coordination, not a second approval of the implementation. Product ambiguity that changes the requested outcome, a genuinely new external commitment or a destructive operation outside standing authority belongs with the liaison/user.

This also separates **confirmation from escalation**. “I received your message,” “may I perform the next planned step?” and “please approve my updated helper hash” are not inherently decisions. Routine delivery receipts can be machine bookkeeping; routine progress can be a coalesced state update. The useful upward message is “here is the choice I cannot resolve within this task,” not “please confirm that I may keep working.”

#### What to borrow from Firstmate—and what not to assume

Firstmate's [README](https://github.com/kunchenguid/firstmate#what-it-is) explicitly describes one liaison, autonomous crew, plain outcomes and disk-backed restart reconciliation. Its [architecture](https://github.com/kunchenguid/firstmate/blob/main/docs/architecture.md#event-driven-supervision) places ordinary polling and benign-wake suppression in a non-model watcher. Its [session-start recovery procedure](https://github.com/kunchenguid/firstmate/blob/main/.agents/skills/session-start-recovery/SKILL.md) keeps slow network checks off the blocking digest path and keeps routine bootstrap confirmations silent. These are useful directions: keep the conversation available, persist work and interrupt the liaison only when judgment is needed.

Firstmate is not evidence that the entire implementation is lightweight. The architecture includes substantial gate, acknowledgement, escalation and recovery machinery, and its [project-management procedure](https://github.com/kunchenguid/firstmate/blob/main/.agents/skills/project-management/SKILL.md#delivery-posture) offers a full `no-mistakes` validation pipeline alongside `direct-PR` and `local-only`. Copying the whole policy stack could recreate the same verification overhead. Borrow the role separation and event-driven mechanics, not another hierarchy of approvals. This assessment is based on reading those published documents, not running or auditing Firstmate.

### First: make the normal path lightweight

- **Delegate once at task level.** Fold routine Prepare/Ready/Execute into a normal start for trusted local work. Retain a separate preparation phase where placement or scope is genuinely unknown; do not impose two-stage ceremony on every known workstream.
- **Accept source completion separately from integrated verification.** A worker can finish its patch and focused checks without launching a full private product environment. Integration owns combined builds and shared runtime proof.
- **Make plans mutable and advisory.** Preserve task/checkout ownership while amending scope in place; reserve new actors for actual replacement, not plan edits.
- **Stop generating proof infrastructure for a structural refactor.** Use existing check infrastructure, disclose gaps and defer unrelated harness improvements.
- **Batch smaller extractions.** Parallelize genuinely independent source work, but avoid a separate lifecycle/worktree/runtime pair for every small file split.

### Then: rethink the durable execution model

A smaller conceptual model is enough: **job, execution lease, checkpoint/result, and exceptional external-effect intent**. The job survives reboot; the execution lease identifies the current writer; the checkpoint describes work already done; only uncertain or destructive external effects require special reconciliation.

The scheduler/runtime—not the language model—should own lease renewal, restart attachment, ordinary build queues and progress coalescing. Losing Cockpit connectivity should not erase the ability to preserve local work. Destructive teardown and ambiguous sends/launches still need fresh identity checks.

Disposable verification resources should be grouped under one runtime-owned container or resource set with normal shutdown. Agents should not have to invent a process-by-process closure protocol for every fixture. Ambiguous foreign/live resources remain protected; this is not a recommendation for broad kills or deleting unmerged work.

### What not to do

Do not respond with more exact hashes, narrower grants, additional negative matrices, another recovery checklist or instructions to “be more autonomous” layered over primitives that forbid ordinary autonomy. The 7–8 October guidance review had already introduced affected-contract verification; the next plan and runtime workflow still recreated heavyweight acceptance. Prose alone did not solve the mechanism.

Do not remove all safeguards either. Preserve single-writer ownership, review of destructive operations, real consumer checks for contract changes, and reconciliation of genuinely unknown external effects. Relax repetitive ceremony around reversible local work instead.

## How to judge whether the redesign helps

Compare the next similar cleanup using existing receipts: source-complete → accepted latency; supervisor messages per completed slice; fresh build/runtime instances per integrated artifact; recovery time until useful work resumes; and how often a harness problem becomes product work. Separate mechanical and semantic slices and exclude shutdown/sleep from active-time claims.

The desired result is not “more green checks.” It is **shorter time to a reviewable integrated change, ordinary restart recovery, fewer supervisory round trips and explicit bounded verification coverage**. No numeric speedup is established by this retrospective.

## Evidence references

Repository references:

- [Original plan](../../planning/code-cleanup-2026-10-09/PLAN.md), especially §§3–6 and §8: contract changes, 25 streams, worker verification, per-wave gate and size targets.
- [Migration inventory](../../planning/code-cleanup-2026-10-09/migration-inventory.md): CURRENT versus migration-only behavior.
- [Architecture](../../CONTEXT.md), supervisor lifecycle and authority; [decisions](../../DECISIONS.md), supervisor orchestration: current documented Prepare/Execute, binding, reconciliation and retry contracts. These describe current design, not independent proof of historical behavior.

Local history, reproducible by native session ID and timestamp:

- Original planning session `01a11f0d-34a2-700c-b962-12d732a583e5`, 9 October 05:05–05:41.
- Cleanup supervisor before recovery `01a11f2e-dc4a-7541-96bb-359ccbca38b9`: owner shutdown request 11:31:31; early Cargo holds 06:32; owner cache/tmp warning 06:46:56.
- Cleanup supervisor after recovery `01a120cb-22f8-76c7-be40-9f080f0c0b84`: stranded-worker request 13:14:49; owner's verification correction 10 October 05:44:02; completion 08:33:44.
- Both root journals are under `~/.local/state/cockpit/operations/orchestration/omp-sessions/00ef1b2c-5254-4589-b57c-dcafefbc6398/`, in `1-5/` and `1-6/` respectively.
- Retained historical recovery diagnosis: `1-5/2026-10-09T05-42-07-178Z_01a11f2e-dc4a-7541-96bb-359ccbca38b9/local/reboot-recovery-plan.md`, especially lines 53–89: boot/endpoint mismatch, preserved versus cleared state, fresh Ready/Execute requirement and failed automatic reconciliation.
- Durable records: `~/.local/state/cockpit/operations/orchestration/state.json`, filtered to runs created on/after 9 October with labels containing `Cleanup` or `WS-`. Message counts use those 28 run IDs; upward counts require destination root `00ef1b2c-5254-4589-b57c-dcafefbc6398`. Snapshot inspected 10 October; this is a live mutable store, not a frozen benchmark.
- Useful message IDs: `integration-supported-successor-existing-worktree-quiesce`; `ws20-stable-source-slot-request-1`; `ws24-scoped-census-source-final-f4f3-pure-proof-1`; `integration947-instruction37-cwd-visibility-denied-no-actor`; `ws20-finish-consumer-smoke-and-commit-user-correction`.
- Worker Results for WS-08/09/11 explicitly distinguish unchanged product commits from verification-provenance repair. WS-13/16/17 Results retain affected native/browser scenarios and harness limitations. WS-20/22/23/24 Result timestamps provide the post-correction intervals above.
- Earlier guidance retrospective `01a11893-7870-748f-bd77-41d62795bc3d`, 7–8 October; reported guidance commit `20f4d62` precedes this cleanup.

No persistent session, worker, setting, binding or runtime resource was modified for this investigation. Historical product acceptance is attributed to its recorded Results and supervisor observations; it was not rerun here.
