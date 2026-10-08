import { describe, expect, it } from "vitest";
import type { Attention, OrchestrationSnapshot, RetirementIdentity, Run, TaskView } from "../../protocol/generated/v1";
import { deriveAttention, rootAttentionSummary } from "./attention";

const at = "2026-10-07T12:00:00Z";
function run(runId: string, rootId = "root", taskId: string | null = null): Run {
  return { session_id: "session", run_id: runId, root_id: rootId, parent_run_id: runId === rootId ? null : rootId,
    kind: runId === rootId ? "supervisor" : "worker", label: runId, prepare_brief: "", task_id: taskId, attempt: 1,
    task_revision_at_propose: null, stage: "working", close_reason: null, dispatch: null, target: null, setup: null,
    prepare_plan: null, init_receipt: null, work_plan: null, grants: [], last_report: null, result: null,
    annotations: [], location: null, bound_omp_session: null, bound_omp_process: null, launch_shell_identity: null, retirement: null, supersedes_run_id: null, created_at: at, updated_at: at };
}
function task(id = "task", currentRunId: string | null = "worker"): TaskView {
  return { task: { task_id: id, title: "Improve search", body: "Keep behavior", description: "Keep behavior", description_editable: true, description_diagnostic: null, steps: [], step_progress: { done: 0, total: 0 }, steps_diagnostic: null, depends_on: [], follow_up_of: null, relations_diagnostic: null, checked: false, line: 1, task_revision: "revision", diagnostic: null }, lane: "working", current_run_id: currentRunId, dependencies: { state: "none", unmet: [], problems: [] } };
}
function snapshot(attention: Attention[], runs = [run("root"), run("worker", "root", "task")]): OrchestrationSnapshot {
  return { session_id: "session", revision: 1, tasks_token: "token", roots: [], board: null, runs, messages: [], subagents: [],
    intents: [], assignment_intents: [], attention, unmanaged_agents: [], runtime: { status: "fresh", endpoint_identity: "endpoint", observed_at: at, runs: [] } };
}
const entry = (kind: Attention["kind"], runId = "worker", since = at, taskId: string | null = null): Attention => ({ kind, run_id: runId, task_id: taskId, message_seq: null, since });
const retirementIdentity: RetirementIdentity = {
  run_attempt: 1, launch_attempt: 1, launch_tag: "launch", endpoint_identity: "endpoint", session_id: "session",
  workspace_id: "space", tab_id: "tab", pane_id: "pane", terminal_id: "terminal", herdr_boot_id: "boot",
  omp_session_id: "omp", process: { pid: 101, start_ticks: 1000, kernel_boot_id: "kernel-boot" },
  shell: { process: { pid: 202, start_ticks: 2000, kernel_boot_id: "fictional-shell-boot" }, executable_device: "8", executable_inode: "9003", argv_digest: "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc" },
};

describe("Supervisor attention boundaries", () => {
  it("keeps supervisor preparation, execution, review and worker questions out of the user queue", () => {
    const state = snapshot([
      entry("awaits_prepare"), entry("awaits_execute"), entry("to_accept"), entry("needs_input"),
      entry("needs_input", "root"),
    ]);
    const model = deriveAttention({ snapshot: state, rootId: "root", local: [] });
    expect(model.counts).toEqual({ decide: 1, recover: 0, notice: 0 });
    expect(model.items[0].runId).toBe("root");
    expect(model.supervisorOwned.map(item => item.kind).sort()).toEqual(["awaits_execute", "awaits_prepare", "needs_input", "to_accept"]);
    expect(model.tierForTask(task())).toBeNull();
    expect(model.ownedForRun("worker")?.since).toBe(at);
    state.attention = state.attention.filter(item => !(item.kind === "needs_input" && item.run_id === "root"));
    expect(deriveAttention({ snapshot: state, rootId: "root", local: [] }).total).toBe(0);
  });

  it("deduplicates recovery causes while retaining core priority and the earliest age", () => {
    const earlier = "2026-10-07T11:00:00Z";
    const model = deriveAttention({ snapshot: snapshot([
      entry("runtime_blocked"), entry("dispatch_unknown", "worker", earlier), entry("exited_without_report"),
    ]), rootId: "root", local: [{ kind: "agent_status", runId: "worker", taskId: "task" }, { kind: "orphaned_worker", runId: "worker" }] });
    expect(model.total).toBe(1);
    expect(model.items[0].sources.map(source => source.origin === "core" ? source.kind : source.condition.kind)).toEqual(["exited_without_report", "dispatch_unknown", "runtime_blocked", "orphaned_worker"]);
    expect(model.items[0].since).toBe(earlier);
    expect(model.counts.recover).toBe(1);
    expect(model.itemsForTask(task())).toEqual(model.itemsForRun("worker"));
    expect(model.tierForTask(task())).toBe("recover");
  });

  it("groups acceptance and assignment notices by task without promoting ordinary review to Decide", () => {
    const model = deriveAttention({ snapshot: snapshot([
      entry("to_accept"), entry("intent_conflict", "worker", at, "task"),
      entry("idle_without_report"), entry("brief_unread"), entry("plan_changed"),
    ]), rootId: "root", local: [{ kind: "assignment", taskId: "task", state: "conflict" }] });
    expect(model.counts).toEqual({ decide: 0, recover: 0, notice: 2 });
    const conflict = model.items.find(item => item.subject.kind === "task")!;
    expect(conflict.sources.map(source => source.origin === "core" ? source.kind : source.condition.kind)).toEqual(["intent_conflict", "assignment"]);
    expect(model.itemsForTask(task()).length).toBe(2);
    expect(model.tierForTask(task())).toBe("notice");
  });

  it("orders by urgency, real core age and then local-only rows without inventing timestamps", () => {
    const model = deriveAttention({ snapshot: snapshot([
      entry("needs_input", "root"), entry("idle_without_report", "worker", "2026-10-07T10:00:00Z"),
      entry("runtime_blocked", "worker", "2026-10-07T13:00:00+02:00"),
      entry("dispatch_unknown", "other", "2026-10-07T11:30:00Z"),
    ], [run("root"), run("worker"), run("other")]), rootId: "root", local: [{ kind: "start_unknown" }, { kind: "notice", message: "Saved" }] });
    expect(model.items.map(item => item.id)).toEqual(["decide:run:root", "recover:run:worker", "recover:run:other", "recover:local:start_unknown", "notice:run:worker", "notice:local:notice"]);
    expect(model.items.filter(item => item.subject.kind === "workarea").every(item => item.since === null)).toBe(true);
  });

  it("isolates roots and counts only user-facing core subjects in the root selector", () => {
    const state = snapshot([
      entry("needs_input", "root"), entry("awaits_execute"), entry("needs_input"),
      entry("runtime_blocked"), entry("dispatch_unknown"), entry("needs_input", "second"),
      entry("exited_without_report", "second-worker"), entry("needs_input", "missing"),
    ], [run("root"), run("worker"), run("second", "second"), run("second-worker", "second")]);
    expect(rootAttentionSummary(state, "root")).toEqual({ decide: 1, recover: 1, notice: 0 });
    expect(rootAttentionSummary(state, "second")).toEqual({ decide: 1, recover: 1, notice: 0 });
    const model = deriveAttention({ snapshot: state, rootId: "root", local: [{ kind: "agent_status", runId: "second-worker", taskId: null }] });
    expect(model.items.map(item => item.runId)).toEqual(["root", "worker"]);
  });

  it("does not infer core attention from saved run state when the authoritative list clears", () => {
    const state = snapshot([]);
    state.runs[1].stage = "reported";
    state.runtime = { status: "unavailable", error: { code: "offline", message: "Disconnected" } };
    const model = deriveAttention({ snapshot: state, rootId: "root", local: [{ kind: "terminal_error", message: "Could not open terminal" }] });
    expect(model.counts).toEqual({ decide: 0, recover: 1, notice: 0 });
    expect(model.items[0].id).toBe("recover:local:terminal_error");
    expect(model.tierForTask(task())).toBeNull();
    expect(model.supervisorOwned).toEqual([]);
  });

  it("uses explicit local recovery until core takes ownership, while failed reports and document issues stay notices", () => {
    const state = snapshot([]);
    const local = [
      { kind: "agent_status" as const, runId: "worker", taskId: "task" },
      { kind: "root_failed_report" as const, runId: "root" },
      { kind: "unidentified_items" as const, count: 2 },
      { kind: "navigation_error" as const },
      { kind: "change_unconfirmed" as const, message: "Saved draft is locked" },
    ];
    const before = deriveAttention({ snapshot: state, rootId: "root", local });
    expect(before.counts).toEqual({ decide: 0, recover: 3, notice: 2 });
    expect(before.tierForTask(task())).toBe("recover");
    expect(before.tierForRun("root")).toBe("notice");
    expect(before.items.every(item => item.since === null)).toBe(true);
    state.attention.push(entry("dispatch_unknown"));
    const after = deriveAttention({ snapshot: state, rootId: "root", local });
    expect(after.counts).toEqual(before.counts);
    expect(after.itemsForRun("worker")[0].sources).toEqual([{ origin: "core", kind: "dispatch_unknown", messageSeq: null, since: at }]);
  });

  it("keeps unconfirmed retirement in Recover for a closed accepted worker and its checked task", () => {
    const worker = run("worker", "root", "task");
    worker.stage = "closed"; worker.close_reason = "accepted";
    worker.retirement = {
      retirement_id: "retirement", trigger: "accept", result_message_id: "result", task_revision: "revision",
      identity: retirementIdentity, state: { state: "unknown", at, phase: "terminal_close", detail: "Closure outcome unknown" },
      created_at: at, updated_at: at,
    };
    const checked = task();
    checked.lane = "accepted"; checked.task.checked = true;
    const state = snapshot([entry("retirement_unconfirmed", "worker", at, "task")], [run("root"), worker]);
    const model = deriveAttention({ snapshot: state, rootId: "root", local: [] });
    expect(model.counts).toEqual({ decide: 0, recover: 1, notice: 0 });
    expect(model.itemsForTask(checked)).toEqual(model.itemsForRun("worker"));
    expect(model.tierForTask(checked)).toBe("recover");
    expect(model.items[0].since).toBe(at);
    expect(model.items[0].sources).toEqual([{ origin: "core", kind: "retirement_unconfirmed", messageSeq: null, since: at }]);
    expect(rootAttentionSummary(state, "root").recover).toBe(1);
    const earlier = "2026-10-07T11:00:00Z";
    state.attention.push(entry("dispatch_unknown", "worker", earlier));
    const combined = deriveAttention({ snapshot: state, rootId: "root", local: [] });
    expect(combined.counts.recover).toBe(1);
    expect(combined.items[0].sources.map(source => source.origin === "core" ? source.kind : source.condition.kind)).toEqual(["retirement_unconfirmed", "dispatch_unknown"]);
    expect(combined.items[0].since).toBe(earlier);
  });

  it("does not manufacture queue work for retained, waiting or absent retirement records", () => {
    const worker = run("worker", "root", "task");
    worker.stage = "closed"; worker.close_reason = "accepted";
    const state = snapshot([], [run("root"), worker]);
    expect(deriveAttention({ snapshot: state, rootId: "root", local: [] }).total).toBe(0);
    worker.retirement = {
      retirement_id: "retirement", trigger: "accept", result_message_id: "result", task_revision: "revision",
      identity: null, state: { state: "retained", at, reason: "identity_incomplete", native_stopped: false },
      created_at: at, updated_at: at,
    };
    expect(deriveAttention({ snapshot: state, rootId: "root", local: [] }).total).toBe(0);
    worker.retirement.identity = retirementIdentity;
    worker.retirement.state = { state: "waiting", blockers: ["running_subagents"] };
    expect(deriveAttention({ snapshot: state, rootId: "root", local: [] }).total).toBe(0);
  });

  it("keeps explicit orphan recovery without a selected root and uses collision-safe subject ids", () => {
    const state = snapshot([], [run("a:b%", "closed", "task:%")]);
    const model = deriveAttention({ snapshot: state, rootId: null, local: [
      { kind: "orphaned_worker", runId: "a:b%" }, { kind: "assignment", taskId: "task:%", state: "pending" },
    ] });
    expect(model.items.map(item => item.id)).toEqual(["recover:run:a%3Ab%25", "notice:task:task%3A%25"]);
    expect(model.tierForTask(task("task:%", "a:b%"))).toBe("recover");
    expect(model.itemsForTask(task("task:%", "a:b%")).length).toBe(2);
  });
});
