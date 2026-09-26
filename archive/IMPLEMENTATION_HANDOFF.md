# Cockpit implementation handoff

You are Astra, the integration owner for the complete Cockpit implementation described here. Use Terra and Luna workers. When the user asks you to execute this handoff, implement the selected scope through a verified, committed release candidate. The planning session made no production changes. Its historical “planning only” statements describe that session, not your future execution authorization.

## First action and protected session

The user will downgrade Herdr to stable before you start. You run inside Herdr `default`. Preserve that session and its running server throughout execution. Never restart, kill, upgrade, downgrade, close, change layout/focus, or send test input to it. Never replace its executable or shared live configuration. All runtime mutations require an explicit, recorded, run-owned non-default session; reject missing, inherited ambiguous, and `default` targets before dispatch. Check the installed CLI's actual session targeting semantics before creating test resources.

The current Cockpit frontend expects protocol 22 and will initially be incompatible. Start with bounded read-only version/schema inventory, then **BOOT-01: restore actual stable transport compatibility before frontend-dependent smoke tests**. Changing only the version constant is insufficient. An incompatible frontend smoke and a live protocol-22 reproduction are not prerequisites. Use disposable sessions with the installed stable executable for subsequent tests. If stable is not installed as expected, do independent preparation and report the prerequisite; do not downgrade your own session.

Stable Herdr is the selected daily-use target. Protocol-22 work is preserved in reachable commit `7e8fe25546ce5fa9364cab100522af6d63343e4a`. Preserve that history. Mouse click/focus, application mouse coordinates, and scrolling must survive the transition. Commits `34459ab` and `582792e` contain earlier mouse work; inspect them for recovery, not wholesale file reversion. Terminal Kitty graphics are parked. Graphical context/media rendering remains selected.

## Read order and authority

Read repository `AGENTS.md` instructions that apply, then these files before assigning work:

1. [Final timeline](planning/next-level/15-execution-timeline.md), the authority for selected scope, stages, dependencies, and parallel lanes.
2. [Verification goals](planning/next-level/16-verification-goals.md), initially evidence rules and G00A/G00B/G01. Load later goals as their stage approaches.
3. [Terminal bootstrap and stability](planning/next-level/13-terminal-stability.md), including BOOT-01.
4. Latest relevant decisions in [DECISIONS.md](DECISIONS.md) and [CONTEXT.md](CONTEXT.md), then inspect actual source and installed stable API. Earlier protocol-22 capability research is historical evidence, not stable compatibility proof.

Use this handoff for orchestration, 15 for execution order, 16 for acceptance, and the owning story document for detailed behavior. Later explicit user instructions take precedence. Resolve ordinary design details autonomously and record decisions. Preserve selected behavior when documents differ; do not silently drop scope. Earlier requests for mock discussion are fulfilled by the accepted workflow revisions. No routine human checkpoint is required.

Load these documents when assigning their work, rather than loading the entire repository history:

| Work | Required references |
| --- | --- |
| Correctness repairs | [14](planning/next-level/14-existing-code-repairs.md) and [reproductions](research/next-level-existing-code-review.md) |
| Cleanup and deterministic metrics | [09](planning/next-level/09-maintainability.md), [11](planning/next-level/11-quality-gates.md), [tool research](research/next-level-quality-gates.md) |
| Contracts, storage, adapters | [01](planning/next-level/01-architecture-and-contracts.md) |
| Setup, ownership, recovery | [02](planning/next-level/02-project-setup-and-lifecycle.md) |
| Companion, snapshots, downloads | [03](planning/next-level/03-context-and-sources.md) |
| Viewer, comments, delivery | [04](planning/next-level/04-viewer-and-reference-comments.md) |
| Real extension pane replacement | [07](planning/next-level/07-extension-panes.md) |
| UI and input | [08](planning/next-level/08-ui-design.md), [10](planning/next-level/10-interaction-checkpoint.md), [accepted workflow mock](planning/next-level/mocks/workflow.html) |
| Rationale or disputed structure | [12](planning/next-level/12-final-design-review.md) |
| Parked building blocks | [05](planning/next-level/05-optional-building-blocks.md), only if explicitly selected later |

## Delivery contract

Complete all selected stories in 15, including full local graphical Review replacement and SRC-04/05 imports. Record parked stories explicitly. Follow S00 through S11; independent later work may proceed as specified, but an unmet prerequisite prevents dependent acceptance. Do not stop after bootstrap, cleanup, a worker report, or the first usable Context pane.

Keep Herdr authoritative for resources, geometry, focus, and control. Implement pane renderer replacement without extension IPC or Herdr-server modifications. Keep source snapshots independent through reflink or ordinary copy, never writable hardlinks. Reference delivery is same-tab paste with explicit outcomes, never Enter or automatic retry after unknown delivery.

Use the accepted keyboard/mouse workflow. Draft comments appear inline or below their file; a header count opens the collection by click or shortcut. No permanent comments bottom panel and no global Build/Review switch. Styling and source setup may be refined without another interview. Favor a personal tool with clear modules and local behavior changes over a configurable framework.

Preserve a source baseline before repairs. When quality tools become available, assess newly written bootstrap/repair code too; do not grandfather it into the legacy baseline. The policy in 11 governs CRAP, coverage, mutation outcomes, mapping completeness, exceptions, and deterministic exit status. Metrics supplement behavioral evidence.

## Persistent execution state

At startup create `planning/next-level/execution/<run-id>/`. Keep concise state and goal reports committed alongside implementation increments. Store large recordings/build artifacts outside Git, with durable locations and SHA-256 hashes in reports. Redact credentials and real user content.

Maintain these files after each integrated wave and before context compaction:

- `state.json`: initial source commit, current integration commit, selected/parked stories, stage, each story's status and goal dependencies, workers and owned paths/worktrees, tested source/test/policy hashes, evidence paths, failures, and external blocks.
- `NEXT.md`: exact next action, current reproduction, unfinished task ownership, relevant document sections, and commands needed to resume. Keep it short enough to read at every restart.
- `decisions.md`: changed assumptions, evidence, chosen behavior, and affected stories. Preserve acceptance intent; a failing result alone cannot justify weakening a gate.
- `resources.json`: protected session, allowed disposable sessions, executable identities, temporary repositories/configs, owning run, cleanup commands, and cleanup outcomes.
- `goals/Gxx.json`: goal ID, PASS/FAIL/INCONCLUSIVE, required subchecks, actual commands and exit status, identities/hashes, assertions, artifacts, and unresolved limitations.

Use `not_started`, `running`, `verified`, and `blocked` for work status. `verified` requires its goal evidence, not worker confidence. Subcheck NOT_APPLICABLE is allowed only for an explicitly permitted capability fallback in 16. It cannot turn a missing required runtime or provider into PASS.

At resumption read this handoff, `NEXT.md`, state, and current Git status before doing work. Check that reports still match the source and configuration. Workers do not inherit reliable memory of previous waves; every assignment must be self-contained.

## Worker and integration protocol

Astra owns contracts, sequencing, work allocation, integration, validation, and commits. Use Terra for stateful transport, concurrency, protocol migration, filesystem ownership, and cross-module changes. Use Luna at high effort for bounded pure modules, fixtures, documentation, and repetitive adapter work after one reviewed example exists. Assign independent review of risky changes to Terra. Worker model choice is not evidence of correctness.

Each task message includes the story and goal IDs, baseline commit, exact owned paths, required contracts and invariants, relevant document sections, concrete scenarios and expected outcomes, forbidden adjacent changes, and a bounded completion condition. Ask workers to report changed files, behavior, assumptions, unresolved cases, and verification commands needed. Avoid sending the full planning transcript.

Workers edit only their assigned files. They do not run shared formatting, builds, test suites, or commits; they return the necessary verification to Astra. Astra runs integration checks after the wave settles. Use isolated worktrees where ownership cannot be separated. Serialize terminal transport edits, manifests/lockfiles, generated contracts, and global migrations. Freeze the current stage's interfaces before parallel adapter/UI work; do not prebuild every future DTO.

For each wave:

1. Select a bounded dependency-ready increment and define observable completion before delegating.
2. Establish a failing characterization or meaningful negative control where applicable. Preserve user edits and unrelated staged work.
3. Integrate worker output, inspect actual production call paths, and resolve interface conflicts. Tests of unused helper code do not establish production behavior.
4. Run required checks once against the settled source, including actual browser/native/disposable-session evidence where the goal requires it. Record exact results.
5. Obtain independent review for risky lifecycle/input/storage changes; fix findings and repeat affected checks.
6. Update state, decisions, documentation and evidence. Stage the intended scope, check the staged diff, commit, and advance to the next ready increment.

After two unsuccessful attempts at the same failure, minimize the reproduction and reassess the contract or worker scope. Reassign or split the work instead of cycling broad patches. Do not weaken assertions, raise thresholds, suppress mutants, or substitute mocks to obtain a green report. Legitimate metric exceptions follow 11's explicit evidence and review process; they cannot excuse wrong-target input, data loss, or terminal instability.

## Runtime and evidence discipline

Implement the proposed verification commands in 16 as part of their owning stages. Those commands do not exist merely because the plan names them. Discover actual repository commands and pin added tooling. Keep input capture and negative controls harmless and test-owned.

Native compilation is not native runtime proof. A static mock or final screenshot cannot prove redraw stability. G01 requires temporal capture, scroll latency, mouse behavior, and detector negative controls on the stable target. Old protocol-22 replay/comparison may explain the regression but is optional. Preserve all required stable mouse assertions even when terminal graphics are unsupported.

Use bounded read-only provider calls with existing configured credentials for required live source checks. Hermetic fixtures exercise failure cases independently. Missing credentials or host capability leaves the relevant goal inconclusive; keep working on independent stages, then report the precise external block. Never fabricate a live result or solicit routine design approval to avoid making an implementation decision.

Tests must target explicit ledger-owned sessions. A script that defaults to `default` is unsafe even if its current caller passes another value. Enforce the guard in the shared test entry point and verify rejection before side effects. Cleanup checks ownership again and never broadly kills Herdr processes. If cleanup fails, record the resource and retry bounded cleanup without touching foreign sessions.

Record both the tested source identity and the delivery commit. A later evidence-only commit may cite the tested code commit when relevant source/test/config hashes remain unchanged; avoid rerunning every test just to record its result in Git. Code, dependency, fixture, or policy changes invalidate affected evidence. Preserve reports across compaction so completion is auditable.

## Autonomy and finish

Proceed without routine human interaction on reversible local implementation, isolated test setup, dependency additions, design details, worktrees, and commits. Existing credentials may be used for authorized read-only imports. Creating credentials, changing the user's live installation, modifying the protected server/session, publishing, pushing, tagging, or remote write actions are outside this handoff's scope. Missing external prerequisites do not authorize those actions.

If an external prerequisite remains after all independent work is exhausted, preserve the completed increments and return one precise blocked report with the missing operation and smallest necessary user action. Do not claim a complete release candidate while a required goal is inconclusive. Do not stop merely because a worker finished or context is filling; persist the state and continue through the remaining selected stages.

Finish when G00A/G00B and G01 through G10 satisfy 16, every selected story is verified, required ordinary and quality checks pass, docs explain how to tweak behavior, test-owned resources are cleaned, and all intended changes are committed. Package the candidate only into temporary output, leaving the user's active installation alone. Report the candidate commit, tested identities, goal index, artifact locations, parked scope, and any limitations. Verify the final Git status and distinguish pre-existing user work from your changes.

Start now with S00 inventory, then S01/BOOT-01 compatibility. Continue through the final timeline.
