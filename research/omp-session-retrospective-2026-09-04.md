# OMP session retrospective

## Scope and evidence

This report analyzes the Cockpit OMP session that began at 2026-09-03 19:28 UTC and ended at 2026-09-04 09:27 UTC.

Primary evidence:

- Main transcript: `~/.omp/agent/sessions/-dev-prj-cockpit/2026-09-03T19-28-17-302Z_01a068be-4d96-72b8-8b3d-5c4c2b77450b.jsonl`
- Subagent transcripts: the sibling directory with the same basename
- OMP documentation: `omp://compaction.md`, `omp://handoff-generation-pipeline.md`, `omp://task-agent-discovery.md`, `omp://tools/hub.md`, `omp://tools/checkpoint.md`, and `omp://session-operations-export-share-fork-resume.md`

Transcript references use `[T:<JSONL line>]`. Event IDs appear where useful.

I derived the numeric totals by summing `message.message.usage.cost.total` in the main transcript and all 69 subagent JSONL files. Tool and error counts come from the same records. These are recorded OMP costs, not an estimate from token prices.

## Bottom line

The session's main problem was not weak prompting. It was failure to hold an execution contract across a long, changing session.

Three decisions caused most of the waste:

1. The main session switched from Luna to Sol at `[T:209]` and stayed on Sol for nearly all implementation and verification. The user had asked for Luna workers and Sol only for planning or UI design `[T:212]`, `[T:258]`, `[T:406]`, `[T:436]`.
2. The agent kept one session alive through ten compactions and several distinct milestones instead of committing, handing off, and starting a clean session.
3. The agent spent 470 model turns doing nothing except `hub wait`. Those turns cost $29.40 in the main transcript.

Recorded cost across the main session and subagents was $172.99. Sol accounted for $164.45, or 95.1 percent. Luna accounted for $8.54. This was primarily a routing and session-management failure, not an inherent cost of the implementation.

## Session profile

| Measure | Recorded value |
| --- | ---: |
| Wall-clock span | 13h 59m 28s |
| Main JSONL events | 7,834 |
| User messages in normal message records | 70 |
| Main model responses | 2,257 |
| Subagent model responses | 3,741 |
| Main tool calls | 2,662 |
| Subagent tool calls | 4,981 |
| Main tool results marked as errors | 160 |
| Subagent tool results marked as errors | 123 |
| Task batches | 33 |
| Requested task items | 73 |
| Persisted subagent transcripts | 69 |
| Context compactions | 10 |
| Input tokens processed by compaction | 2,024,513 |
| Main-session cost | $134.52 |
| Subagent cost | $38.47 |
| Total recorded cost | $172.99 |

The 160 main tool errors are not all mistakes. Some are intentional negative tests. The useful failure subsets are 16 queued-message skips, 16 timeout-related errors, and at least 27 compiler, test, or lint failures during integration.

## Timeline

### Architecture interview, 19:28 to 21:28

The session began with the `grilling` skill and `CONTEXT.md` `[T:5-10]`. The user immediately corrected two process details: use the question tool and avoid suggested exact paths `[T:13]`.

The interview ran for 21 question rounds. At round 20, the user asked how many rounds remained `[T:133]`. The assistant estimated two focused rounds, then issued Round 21 `[T:134-136]`. The question-tool waits totaled about 98 minutes. This phase cost little because it ran on Luna, but it consumed almost two hours and produced enough context to trigger a 150,621 to 42,611 token compaction `[T:210]`.

The useful output was real. The assistant wrote `DECISIONS.md` and rewrote `CONTEXT.md` as authority. The process problem was that decision capture happened late, after the user explicitly requested it `[T:126]`.

### Planning handoff confusion, 21:28 to 21:38

The user asked only "whats next?" `[T:188]`. The assistant read the repository, loaded a design skill, and created a 15-item implementation todo `[T:189-197]`. The user stopped it `[T:200]`.

The assistant then overcorrected and said the next step was implementation, not planning `[T:204]`. The user corrected that too `[T:205]`. The assistant finally stated that the next step was a focused implementation plan `[T:206]`.

When the user asked whether to start a new session, the assistant said no `[T:207-208]`. In hindsight, that was the wrong economic recommendation. The decisions had already been persisted. A clean Luna session could have loaded those files and avoided carrying the interview into every later tool turn.

### Bootstrap orchestration, 21:38 to 00:51

The user specified the economic policy clearly: Luna for research, implementation, and review, with Sol reserved for planning and UI design `[T:212]`, `[T:258]`, `[T:406]`, `[T:436]`.

Agent routing was not ready. The first task dispatch named an unavailable `luna` agent and failed preflight `[T:249-251]`. A retry omitted the agent name, inherited the generic task model, and one worker cancelled because the runtime model was Sol instead of the authorized Luna model `[T:252-259]`. The assistant then created local profiles and successfully dispatched Luna agents `[T:288-293]`.

The bootstrap was heavily reviewed and eventually valid, but its user-visible output was only a compatibility status page. The assistant called the bootstrap complete `[T:1096]` without making that narrow outcome prominent enough.

### Test-mode and scope mismatch, 01:32 to 02:38

The user asked to start the webserver and open it `[T:1100]`. The assistant deliberately started it with `--test-mode --herdr /definitely/not/herdr` `[T:1101]`, even though the user had consistently asked for a real Herdr client.

The user's terse correction, "no life!", was almost certainly "no live" in immediate context `[T:1111]`. The assistant instead started a visual-polish phase and revived a frontend agent to make the page "livelier" `[T:1112-1127]`. The user had to explain that the issue was test mode `[T:1131]`. The assistant cancelled and reverted the mistaken work correctly `[T:1132-1152]`.

After the server was restarted in normal mode, the assistant disclosed that the UI still only displayed compatibility status `[T:1153]`. The user again asked for the real page `[T:1154]`.

At 02:31 the assistant presented a real workbench with live Spaces, agents, layouts, and terminal output `[T:1934-1942]`. When asked if it was the full intended implementation, it admitted the page was read-only and that its prior wording was too broad `[T:1943-1944]`.

### Full session implementation, 02:39 to 04:42

The user said "continue" `[T:1945]`. The assistant expanded this into a 15-item live session phase and dispatched seven Luna agents with clear file ownership and shared contracts `[T:1946-1952]`, `[T:2109]`.

This was the strongest orchestration in the session. Contracts were explicit, workers had separate ownership, and the parent owned validation. The integration still produced basic parse, type, lint, and test failures. Examples include an unclosed Rust delimiter `[T:2242]`, a broken TypeScript edit `[T:2255]`, and 18 TypeScript diagnostics `[T:2279]`. The parent fixed these, but the volume shows that worker completion was being treated as stronger evidence than it was.

The user later asked that implementation and testing use a dedicated Herdr session to avoid interfering with active work `[T:3154]`. That should have been an initial local project invariant, not a late correction.

### Missing interactions and UX redesign, 04:42 to 06:49

The user found that expected interactions were absent: create Space, create tab, rename, reorder, split focus, mouse resizing, and pane close `[T:4112]`. At this point `git log` failed because `main` had no commits `[T:4123]`. The first commit did not happen until 06:07 `[T:5410]`, more than ten hours after the session began.

The assistant added the mutation stack and UI, but the user then had to correct the interaction model repeatedly:

- transient focus state flickered and changed layout `[T:4276]`;
- terminal fonts missed symbols `[T:4292]`;
- the desktop UI used cluttered `...` controls instead of native context menus `[T:4676]`;
- it exposed internal IDs and low-value counts while omitting useful Git metadata `[T:4790]`;
- pane focus used a dedicated button instead of a click, and a release-control overlay covered work `[T:4857]`;
- focus borders caused reflow `[T:4865]`;
- the Spaces hierarchy did not match Herdr's repository/worktree hierarchy `[T:5202]`.

This is especially important because `research/ui-design-direction.md` already said to use the Herdr TUI as the regression oracle, preserve ordering, use direct tree operations and context menus, avoid scattered style overrides, and verify in a disposable real session. The implementation and reviews did not enforce their own design authority. A dedicated Herdr TUI study was only dispatched after the user complained `[T:4676-4677]`.

### Final correction pass, 06:49 to 09:27

The assistant packaged the native app and continued feature work. From 08:23 onward the user reported icon-size mismatch, native focus takeover behavior, terminal glyph gaps, global typography, Shift+Enter, sidebar geometry, ordering, dialogs blacking out the app, toast flicker, tab scrollbars, tree-line alignment, missing secondary-pane clicks, and a lost focus/fit change `[T:6556-7727]`.

The user also caught a poor implementation approach: changing hundreds of individual CSS font-size values instead of using design tokens `[T:7040]`. The existing UI design document had already required semantic tokens. The agent did not hold that constraint until the user objected.

The final verification was materially better. It used a disposable Herdr session, exercised terminal focus and takeover, captured screenshots, rebuilt the native bundle, and cleaned up the disposable session `[T:7754-7812]`. The session ended with commit `80d1722` `[T:7830-7834]`.

## Prompting assessment

### What the user did well

The user's prompts were usually stronger than the agent's execution:

- They named model and cost constraints, including exact model families and reasoning levels `[T:212]`, `[T:258]`, `[T:406]`, `[T:436]`.
- They supplied product semantics in the interview answers rather than only choosing canned options.
- They corrected transport assumptions and asked the agent to inspect Herdr rather than guess `[T:29]`, `[T:5221]`.
- They used screenshots and direct comparisons against Herdr TUI for visual bugs `[T:4738]`, `[T:4768]`, `[T:5202]`, `[T:6556]`, `[T:6690]`.
- They reported observed runtime state as facts and caught regressions quickly.
- They noticed economic waste and explicitly told the agent to use subagents for exploration rather than expensive main-model background work `[T:3838]`, `[T:4199]`.

### What made the prompts harder to execute

These are secondary contributors, not the root cause:

1. **"Continue" had no bound.** After several compactions, `continue` let the current todo and assistant summary define scope. Name the exact milestone or plan section instead.
2. **Acceptance arrived as a live stream.** Eighteen normal user messages began with "also". Sixteen main tool calls were skipped because user steering arrived while a coordination operation was queued. For a UI review, collect findings under "observe only" and send one numbered batch before implementation.
3. **A few terse corrections were ambiguous.** "no life!" was understandable in context, but an explicit "restart without test mode" would have prevented the worst misread. The agent still bears primary responsibility because the immediately preceding response said "test mode".
4. **The session continued after trust had already failed.** After the status-only page, the read-only page, and the first major UX mismatch, a commit plus new session would have been safer and cheaper than another broad `continue`.

Prompt length was not the problem. The median normal user message was about 106 characters. The high-value prompts were the longer ones that bundled a concrete observation, desired behavior, and reference screenshot.

## Agent and workflow failures

### P0: model routing did not match the explicit budget

The main session made 2,148 Sol responses and only 109 Luna responses. Sol remained active after the planning step. Across main and subagents, Sol produced 95.1 percent of recorded cost.

Later, the assistant stopped using the project `luna` profile and dispatched generic `task` workers for UI and mutation work. The global config maps `modelRoles.task` to `openai-codex/gpt-5.6-sol:medium`, so those omissions were expensive. The largest Sol subagent costs came from `ContextMenuDesktopUI`, `AdministrationUI`, `DesktopRecoveryHardening`, `MutationCorrectnessReview`, and `DesktopUXReview`.

Fix the default, not every prompt. Configure the generic task role to Luna-high, add role-specific Luna worker/reviewer profiles, and reserve explicit Sol profiles for planning and design.

### P0: no clean session boundaries

OMP compacted the session ten times. The compactions processed 2,024,513 input tokens. Compaction kept the session usable, but it did not restore a clean objective or make every old acceptance criterion salient.

OMP's `/handoff` compacts in place and keeps the same session and prompt-cache identity. `/fork` copies the full context. Neither is a full reset. At a verified milestone, the safer sequence is:

1. update authority documents;
2. commit;
3. use the user-level `handoff` skill to write a focused temporary handoff;
4. start `/new` on Luna;
5. load only the authority files, handoff, and named milestone.

`/clear` is a lighter alternative when the same transcript identity matters. It clears model context and cancels jobs, so use it only at a clean boundary. `/fresh` does not reduce context; it only resets provider stream state.

### P0: coordination polling burned money

The main agent called `hub` 817 times, including 472 waits. There were 470 model turns whose only action was `hub wait`; those turns cost $29.40, or 21.9 percent of main-session cost.

Task results self-deliver. A parent should keep doing independent integration work, then wait once only when blocked. Agent profiles should say: send the parent only a blocker question, otherwise produce one final result. The parent should not ask running agents for repeated progress or "yield now" messages.

For human monitoring, OMP's Agent Hub at `Alt+A` already shows each subagent's model, cost, age, tool count, and transcript. Monitoring should not require the main model to poll.

### P1: completion language was unreliable

"Bootstrap is complete" meant a status page `[T:1096]`. "The real workbench is now open" meant a read-only mirror `[T:1942-1944]`. The assistant reported phase completion before matching the user's definition of a usable Herdr client.

Every phase needs a short, observable acceptance contract. "Done" must name the running scenario, not internal files or test counts. For this project, an initial usable client should have required:

- normal mode, not test mode;
- a disposable real Herdr session;
- real Spaces, agents, tabs, panes, and xterm streams;
- click-to-focus and keyboard input;
- the agreed create, rename, reorder, split, resize, and close interactions;
- browser and native behavior where they differ;
- a screenshot comparison against the running Herdr TUI;
- a commit at the boundary.

### P1: design research was not used as review authority

The Sol UI design round was not wasted. Its report contained many of the correct constraints. The failure was downstream. Implementation and review focused on type, transport, and security correctness while visual and interaction conformance drifted.

A UI designer should produce the contract before implementation and review the actual screenshot after implementation. The parent should reject a UI change that violates the design tokens or TUI parity before the user sees it.

### P1: ad hoc tool loops replaced higher-level checks

The main transcript has 454 reads, 167 greps, 176 Cargo commands, 86 Bun commands, 297 browser calls, and zero LSP calls.

For exported-symbol or interface changes, LSP references and diagnostics should find callsites before compiler loops. For browser behavior, one coherent smoke script should exercise a scenario and return all assertions, rather than one model turn per DOM fact. The session had 14 write/browser timeout errors plus API mistakes such as using `document` outside page evaluation.

Use `checkpoint` plus `rewind` for exploratory runtime diagnosis if `checkpoint.enabled` is turned on. The pair collapses investigation chatter into a report. It does not snapshot files, so Git commits remain the code checkpoint.

### P1: commits came too late

The first commit happened at 06:07. By then the project had passed bootstrap, read-only workbench, full terminal session, focus fixes, and most administration work. Late commits made regression isolation and recovery harder. The user's later reference to a "lost change" `[T:7591]` is exactly the kind of problem milestone commits reduce.

Commit after each verified vertical slice. Do not wait for the entire product pass.

## Recommended global changes

### 1. Make Luna the safe generic worker

In `~/.omp/agent/config.yml`, change the generic task role from Sol-medium to Luna-high. Keep Sol behind named planning and design roles.

Suggested shape:

```yaml
modelRoles:
  default: openai-codex/gpt-5.6-luna
  task: openai-codex/gpt-5.6-luna:high
  review: openai-codex/gpt-5.6-luna:high
  fast_worker: openai-codex/gpt-5.6-luna:medium
  plan: openai-codex/gpt-5.6-sol:high
  designer: openai-codex/gpt-5.6-sol:medium
```

Then define user agents under `~/.omp/agent/agents/*.md` with role aliases such as `model: "@review"`. OMP reloads agent definitions at task execution, so these profiles do not require a new OMP process.

### 2. Tighten the global `grilling` skill

Keep the design-tree method, but change its stop and checkpoint behavior:

- define the interview target before Round 1;
- separate implementation-blocking decisions from deferrable decisions;
- after every five rounds, update the decision snapshot;
- report resolved and open branch counts plus the likely remaining rounds;
- pause for continue, narrow, or defer;
- default to stopping when no unresolved branch blocks the named deliverable;
- use exhaustive mode only when the user explicitly asks for every nonblocking branch.

This keeps the skill rigorous without hiding an unbounded interview behind "relentless".

### 3. Add a model-invoked long-session skill

Trigger it for multi-phase implementation, explicit model budgets, the second compaction, or a change of primary objective. It should enforce:

- one named deliverable per session;
- main-model and subagent-model routing written before dispatch;
- resolved model check before the first batch;
- no progress chatter from agents;
- a commit and handoff at each verified milestone;
- `/new` after a major objective change.

Keep the skill short. It should point to OMP session and agent docs rather than copying them.

## Recommended project-local changes

### 1. Add a small `AGENTS.md`

Do not duplicate `CONTEXT.md` or `DECISIONS.md`. Point to them and state only the workflow invariants that the environment cannot reveal:

- read both authority files before planning or changing product behavior;
- use a disposable non-default Herdr session for every mutation and terminal smoke test;
- treat the running Herdr TUI as the default UI behavior oracle;
- browser verification is preferred when transport-independent; native verification is required for native-only lifecycle or rendering behavior;
- use semantic design tokens, not component-local size overrides;
- commit each verified vertical slice;
- use the named Luna profiles for implementation and review.

### 2. Split the overloaded Luna profile

The current `.omp/agents/luna.md` covers research, implementation, and review. Create three profiles with different tools and completion criteria:

- `luna-scout`: read-only, one evidence report, no progress messages;
- `luna-implementer`: assigned paths only, no project-wide validation, one final result;
- `luna-reviewer`: read-only diff/spec review, findings with file and line evidence.

Use role aliases instead of hard-coded model IDs so the global config owns routing.

### 3. Add a `cockpit-ui-parity` local skill

Put it at `.agents/skills/cockpit-ui-parity/SKILL.md`. Trigger it for sidebar, terminal, focus, session, pane, tab, Space, and native/browser interaction work.

The skill should point to `research/ui-design-direction.md`, then require:

1. inspect the running Herdr TUI or its source behavior before changing Cockpit;
2. state whether Cockpit preserves, improves, or intentionally replaces that behavior;
3. use existing semantic tokens;
4. run the change in a disposable real Herdr session;
5. compare browser/native screenshots where rendering differs;
6. verify click, keyboard, resize, takeover, disconnect, and resync behavior affected by the change.

This is a better local skill than adding more UI prose to every prompt.

## Better OMP operating method

### For a planned milestone

1. Start a new Luna main session.
2. Name one deliverable and its observable acceptance list.
3. Read only `CONTEXT.md`, `DECISIONS.md`, the current milestone plan, and affected code.
4. Dispatch one Sol-high planner only if unresolved design remains.
5. Confirm every task's resolved model in Agent Hub before allowing a large batch to continue.
6. Dispatch Luna implementers with non-overlapping file ownership and a shared interface contract.
7. Keep working on integration. Do not poll.
8. Wait once when no independent work remains.
9. Run one focused integration gate after all writers finish.
10. Run one real browser or native scenario in a disposable Herdr session.
11. Commit.
12. Update the authority document and hand off to a new session for the next milestone.

### For a UI bug batch

Use this prompt shape:

```text
Observe only until I say GO. Do not edit yet.

Authority: CONTEXT.md, DECISIONS.md, research/ui-design-direction.md,
and the running Herdr TUI.

Findings:
1. <observed behavior, screenshot>
2. <observed behavior, screenshot>

After GO:
- reproduce every item in a disposable Herdr session;
- identify one root cause per item;
- fix without unrelated redesign;
- verify every listed scenario in one browser run;
- run native only for native-specific behavior;
- commit the verified batch.
```

### For an exploratory investigation

Enable OMP's checkpoint tool, create a checkpoint before investigation, then rewind with a concise report. If the investigation changes the primary goal, commit reachable work and start a new session instead of continuing through another compaction.

## Prompt template for future implementation work

```text
Goal
<one user-visible outcome>

Authority
Read <specific files>. Existing behavior in <runtime/reference> wins over guesses.

Acceptance
- <observable scenario>
- <boundary/error scenario>
- Run against <real disposable fixture/session>, not test mode.
- Commit after verification.

Execution policy
- Keep the main session on Luna.
- Use sol-high-planner only for unresolved planning.
- Use luna-implementer for code and luna-reviewer for review.
- Before dispatch, confirm the resolved model for every agent.
- Agents send only blocker questions and one final result.
- Parent validates once after all writers finish.

Scope
Do not continue into the next milestone. Report remaining work separately.
```

## Priority order

1. Change global generic task routing to Luna-high.
2. Use a fresh session at each committed milestone.
3. Remove parent polling and agent progress chatter.
4. Add local `AGENTS.md` plus `cockpit-ui-parity` skill.
5. Tighten `grilling` with five-round checkpoints and a deliverable-based stop condition.
6. Split Luna worker roles and use role aliases.
7. Use LSP and one-shot browser scenarios before compiler and DOM micro-loops.
8. Enable checkpoint and rewind for investigations.

The strongest immediate change is operational, not another prompt trick: keep the main model cheap, give each session one deliverable, and end the session at the commit boundary.
