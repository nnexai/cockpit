# Example: typed DTO fixtures (for slice tests and the SYNTHETIC harness)

These are illustrative snippets. They extend the existing builders `run`, `observed`, `task`, `board` and `snapshot` from `src/app/supervisor/SupervisorView.test.tsx:11-25`. Core attention entries are written out explicitly, as core would produce them (`projection.rs:371-508`). The UI must never derive them (PLAN D1).

Harness screenshots built from these fixtures are labelled **SYNTHETIC DTO**.

```ts
import type { Attention, OrchestrationSnapshot, Subagent, TaskIntent } from "../../protocol/generated/v1";

const at = "2026-10-07T12:00:00Z";
const earlier = "2026-10-07T11:50:00Z";
const att = (kind: Attention["kind"], run_id: string, task_id: string | null, since = at, message_seq: number | null = null): Attention =>
  ({ kind, run_id, task_id, message_seq, since });

// attention-mix: scenarios 1, 2, 16
export function attentionMix(): OrchestrationSnapshot {
  const root = run({ last_report: { message_id: "q1", kind: "needs_input", outcome: null, summary: "Which task should get priority?", plan: null, at } });
  const missing = run({ kind: "worker", run_id: "w-missing", label: "Worker 1", parent_run_id: "root", task_id: "t-a", stage: "working" });
  const blocked = run({ kind: "worker", run_id: "w-blocked", label: "Worker 4", parent_run_id: "root", task_id: "t-b", stage: "working" });
  const review = run({ kind: "worker", run_id: "w-review", label: "Worker 5", parent_run_id: "root", task_id: "t-c", stage: "reported",
    result: { message_id: "r1", kind: "result", outcome: "succeeded", summary: "Migration reviewed.", plan: null, at: earlier } });
  const s = snapshot([root, missing, blocked, review], [
    task({ task: { ...task().task, task_id: "t-a", title: "Audit shortcuts" }, current_run_id: "w-missing", lane: "working" }),
    task({ task: { ...task().task, task_id: "t-b", title: "Document supervisor" }, current_run_id: "w-blocked", lane: "working" }),
    task({ task: { ...task().task, task_id: "t-c", title: "Review migration" }, current_run_id: "w-review", lane: "review" }),
    task({ task: { ...task().task, task_id: "t-d", title: "Draft release notes" }, current_run_id: null, lane: "queued" }),
  ]);
  s.runtime = { status: "fresh", endpoint_identity: "endpoint", observed_at: at, runs: [
    observed("root"), observed("w-missing", { presence: "missing", actual_omp: false, pane_id: null }),
    observed("w-blocked", { agent_status: "blocked" }), observed("w-review", { agent_status: "done" }),
  ] };
  s.assignment_intents = [{ root_id: "root", task_id: "t-d", state: "conflict" }];
  s.intents = [{ intent_id: "i1", root_id: "root", task_id: "t-c", run_id: "w-review", expected_task_revision: "task-revision", state: "conflict",
    origin: "supervisor", supervisor_run_id: "root", omp_session_id: "native", result_message_id: "r1" } satisfies TaskIntent];
  s.attention = [
    att("needs_input", "root", null),                   // Decide (root)
    att("exited_without_report", "w-missing", "t-a"),   // Recover
    att("runtime_blocked", "w-blocked", "t-b"),         // Recover
    att("to_accept", "w-review", "t-c", earlier),       // supervisor-owned, NOT queued (scenario 2)
    att("intent_conflict", "w-review", "t-c"),          // Notice, joins intents[0].intent_id
  ];
  return s; // expected counts: decide 1, recover 2, notice 2 (intent_conflict + local assignment conflict)
}

// worker question stays with the supervisor (existing invariant SupervisorView.test.tsx:126-149)
export function workerQuestion(): OrchestrationSnapshot {
  const worker = run({ kind: "worker", run_id: "worker", parent_run_id: "root", task_id: "task-a", stage: "working",
    last_report: { message_id: "wq", kind: "needs_input", outcome: null, summary: "Which test should I use?", plan: null, at } });
  const s = snapshot([run(), worker], [task()]);
  s.attention = [att("needs_input", "worker", "task-a")];   // → SupervisorOwned, card "Waiting for supervisor", no Decide row
  return s;
}

// topology edge cases: nested worker holding a task, nested subagents, unassigned + hidden done, taskless worker, cycle
export function topologyEdges(): OrchestrationSnapshot {
  const root = run();
  const w2 = run({ kind: "worker", run_id: "w2", label: "Worker 2", parent_run_id: "root", task_id: "t-map", stage: "working", created_at: "2026-10-07T10:00:00Z" });
  const nested = run({ kind: "worker", run_id: "w2:n", label: "Nested worker", parent_run_id: "w2", task_id: "t-nest", stage: "working" }); // colon in id → encoding test
  const taskless = run({ kind: "worker", run_id: "w-free", label: "Helper", parent_run_id: "root", task_id: null, stage: "working" });
  const cycA = run({ kind: "worker", run_id: "cyc-a", label: "Cycle A", parent_run_id: "cyc-b", task_id: null, stage: "working" });
  const cycB = run({ kind: "worker", run_id: "cyc-b", label: "Cycle B", parent_run_id: "cyc-a", task_id: null, stage: "working" });
  const s = snapshot([root, w2, nested, taskless, cycA, cycB], [
    task({ task: { ...task().task, task_id: "t-map", title: "Map sidebar", line: 1 }, current_run_id: "w2", lane: "working" }),
    task({ task: { ...task().task, task_id: "t-nest", title: "Nested work", line: 2 }, current_run_id: "w2:n", lane: "working" }), // assigned link, not unassigned
    task({ task: { ...task().task, task_id: "t-new", title: "Draft release notes", line: 3 }, current_run_id: null, lane: "queued" }), // dotted, gap 0.4
    task({ task: { ...task().task, task_id: "t-done", title: "Old work", line: 4, checked: true }, current_run_id: null, lane: "accepted" }), // hidden count 1
  ]);
  s.subagents = [
    { run_id: "w2", subagent_id: "scout", parent_subagent_id: null, role: "Read-only scout", label: "Read-only scout", status: "running", summary: null, last_control: null, updated_at: at },
    { run_id: "w2", subagent_id: "lint", parent_subagent_id: "scout", role: "Doc linter", label: "Doc linter", status: "done", summary: null, last_control: null, updated_at: at },
    { run_id: "w2", subagent_id: "lost", parent_subagent_id: "missing", role: null, label: "Detached child", status: "failed", summary: null, last_control: null, updated_at: at }, // attaches to run
  ] satisfies Subagent[];
  return s;
}
// expected (PLAN D5):
//   run:root (col 0) → task:t-map (1) → run:w2 (2) → sub:w2:scout (3) → sub:w2:lint (4)
//                                                    → sub:w2:lost (3)
//                                                    → run:w2%3An (3, delegated) ; link task:t-nest → run:w2%3An
//                    → task:t-nest (1, not unassigned)
//                    → run:w-free (2, delegated from supervisor; no fake task)
//                    → task:t-new (1, unassigned, gapBefore 0.4, last)
//   cyc-a/cyc-b: both drawn; one edge removed at lexically smallest member ("cyc-a" becomes a root, col 2)
//   hiddenCompletedTasks = 1
```

## Static-figure geometry check (S2 `graphLayout.test`)

DESIGN:210-221 gives exact positions. Building the same forest with `GRAPH_GEOMETRY` must yield the positions below, with canvas 1344×478:

| id | x | y |
|---|---|---|
| supervisor | 8 | 215 |
| task 1–3 (Audit, Map, Improve) | 280 | 8 / 64 / 120 |
| task 4 (Document supervisor), Worker 4 | 280 / 552 | 204 |
| Read-only scout, Doc linter | 824 / 1096 | 176 |
| Schema checker | 824 | 232 |
| task 5, task 6 | 280 | 288 / 344 |
| unassigned task (gap 0.4) | 280 | 422 |

Leaf rows: 0, 1, 2, 3 (Doc linter), 4 (Schema), 5, 6, 7.4. Worker 4 sits at row 3.5. The supervisor sits at (0 + 7.4) / 2 = 3.7, so y = round(8 + 56 × 3.7) = 215.
