import { describe, expect, it } from "vitest";
import type { OrchestrationSnapshot, Run, TaskOwnership, TaskView } from "../../protocol/generated/v1";
import { buildSupervisorGraph, chainIds, firstChild, nodeFacts, nodeId, nodeSelection, pathNodes, selectionNodeId } from "./topology";

function run(id: string, overrides: Partial<Run> = {}): Run {
  return { session_id: "session", brief: "", run_id: id, kind: id === "root" ? "supervisor" : "worker", label: id, root_id: "root", parent_run_id: id === "root" ? null : "root", task_id: null, attempt: 1, assignment_generation: null, launch_config: { harness_kind: "omp", command: "omp", extra_args: [], resume_args: null, inbox_prompt_hints: false, routes: [] }, task_revision_at_start: null, stage: "working", close_reason: null, dispatch: null, target: null, setup: null, cleanup_policy: null, last_report: null, result: null, source_output: null, integration: { state: "pending" }, annotations: [], project: null, checkout: null, attachment: null, checkout_claim_id: null, resume: null, retirement: null, created_at: "2026-10-07T00:00:00Z", updated_at: "2026-10-07T00:00:00Z", ...overrides };
}
function task(id: string, line: number, worker: string | null = null, lane: TaskView["lane"] = "queued"): TaskView {
  return { task: { task_id: id, title: id, body: "", description: "", description_editable: true, description_diagnostic: null, steps: [], step_progress: { done: 0, total: 0 }, steps_diagnostic: null, depends_on: [], follow_up_of: null, relations_diagnostic: null, checked: lane === "accepted", line, task_revision: "revision", diagnostic: null }, lane, current_run_id: worker, dependencies: { state: "none", unmet: [], problems: [] } };
}
function ownership(taskId: string, runId: string | null): TaskOwnership {
  return { root_id: "root", task_id: taskId, generation: runId ? 1 : 2, owner: runId ? { run_id: runId, conversation: { harness: "codex", session_id: `conversation:${runId}` }, claim_operation_id: `claim:${runId}` } : null };
}
function snapshot(runs: Run[], task_ownership: TaskOwnership[] = []): OrchestrationSnapshot {
  return { session_id: "session", state_store_id: "store", revision: 1, tasks_token: "token", roots: [], board: null, runs, task_ownership, messages: [], questions: [], intents: [], assignment_intents: [], cleanup: [], restoration: { reattached: 0, total_attachments: 0, cleanup_pending: 0 }, unmanaged_agents: [], attention: [], runtime: { status: "fresh", endpoint_identity: "endpoint", observed_at: "2026-10-07T00:00:00Z", runs: [] } };
}

describe("Supervisor topology", () => {
  it("keeps the canonical worker forest and draws only current ownership assignments", () => {
    const root = run("root");
    const snap = snapshot([root, run("worker", { task_id: "misleading" }), run("nested", { parent_run_id: "worker" }), run("closed", { stage: "closed" }), run("other", { root_id: "other-root" }), run("orphan", { parent_run_id: "missing" })], [ownership("assigned", "worker"), ownership("nested-task", "nested")]);
    const tasks = [task("assigned", 1, "historical"), task("nested-task", 2), task("no-worker", 3), task("closed-worker", 4, "closed"), task("done", 5, "worker", "accepted")];
    const model = buildSupervisorGraph({ snapshot: snap, root, rootId: "root", tasks });
    expect(model.counts).toEqual({ supervisors: 1, tasks: 4, workers: 3 });
    expect(model.hiddenCompletedTasks).toBe(1);
    expect(model.byId.has(nodeId.run("closed"))).toBe(false);
    expect(model.byId.has(nodeId.run("other"))).toBe(false);
    expect(model.byId.get(nodeId.run("worker"))!.parentId).toBe(nodeId.task("assigned"));
    expect(model.byId.get(nodeId.run("nested"))!.parentId).toBe(nodeId.run("worker"));
    expect(model.byId.get(nodeId.run("orphan"))!.parentId).toBeNull();
    expect(model.links).toEqual([{ from: nodeId.task("nested-task"), to: nodeId.run("nested"), kind: "assigned" }]);
    expect(nodeFacts(model.byId.get(nodeId.task("assigned"))!, { model, snapshot: snap, live: true, connected: true, runtimeLive: true })).toMatchObject({ provenance: "Assigned · worker · codex", relation: "Assigned conversation codex · conversation:worker" });
    expect(model.byId.get(nodeId.task("closed-worker"))).toMatchObject({ unassigned: true, assignedRunId: null, edge: "unassigned" });
    expect(firstChild(model, nodeId.run("root"))).toBe(nodeId.task("assigned"));
    expect(firstChild(model, nodeId.run("worker"))).toBe(nodeId.run("nested"));
    expect([...chainIds(model, nodeId.run("nested"))].sort()).toEqual([nodeId.run("nested"), nodeId.run("worker"), nodeId.run("root"), nodeId.task("assigned"), nodeId.task("nested-task")].sort());
    expect(pathNodes(model, nodeId.run("nested"), false).map(node => node.id)).toEqual([nodeId.task("nested-task"), nodeId.task("assigned"), nodeId.run("worker"), nodeId.run("nested")]);
    expect(pathNodes(model, nodeId.run("worker"), true).map(node => node.id)).toEqual([nodeId.task("assigned"), nodeId.run("worker"), nodeId.run("nested")]);
    for (const node of model.nodes) {
      const selection = nodeSelection(node);
      expect(selectionNodeId("task" in selection ? { selectedTask: selection.task, selectedRun: null } : { selectedTask: null, selectedRun: selection.run })).toBe(node.id);
    }
  });

  it("does not turn pending or historical runs into current task owners", () => {
    const root = run("root"), historical = run("historical", { task_id: "vacant", assignment_generation: 1 }), pending = run("pending", { task_id: "vacant", assignment_generation: 3, stage: "starting" });
    const snap = snapshot([root, historical, pending], [ownership("vacant", null), ownership("missing-process", "departed"), { ...ownership("foreign", "historical"), root_id: "another-root" }]);
    const model = buildSupervisorGraph({ snapshot: snap, root, rootId: "root", tasks: [task("vacant", 1, "pending"), task("missing-process", 2), task("foreign", 3, "historical")] });
    expect(model.byId.get(nodeId.task("vacant"))).toMatchObject({ assignedRunId: null, unassigned: true });
    expect(model.byId.get(nodeId.task("foreign"))).toMatchObject({ assignedRunId: null, unassigned: true });
    expect(model.byId.get(nodeId.task("missing-process"))).toMatchObject({ assignedRunId: "departed", unassigned: false });
    expect(model.byId.get(nodeId.run("pending"))!.parentId).toBe(nodeId.run("root"));
    expect(model.byId.get(nodeId.run("historical"))!.linkedTaskId).toBeNull();
    expect(model.links).toEqual([]);
    const facts = { model, snapshot: snap, live: true, connected: true, runtimeLive: true };
    expect(nodeFacts(model.byId.get(nodeId.task("vacant"))!, facts).provenance).toBe("Unassigned · ownership pending");
    expect(nodeFacts(model.byId.get(nodeId.task("missing-process"))!, facts)).toMatchObject({ provenance: "Assigned · codex", relation: "Assigned conversation codex · conversation:departed" });
    expect([...chainIds(model, nodeId.task("vacant"))]).not.toContain(nodeId.run("pending"));
  });

  it("retains orphaned work after root closure while hiding closed runs", () => {
    const root = run("root", { stage: "closed" });
    const snap = snapshot([root, run("worker"), run("closed", { stage: "closed" })], [ownership("assigned", "worker")]);
    const model = buildSupervisorGraph({ snapshot: snap, root, rootId: "root", tasks: [task("assigned", 1, "worker"), task("unassigned", 2)] });
    expect(model.counts).toEqual({ supervisors: 0, tasks: 2, workers: 1 });
    expect(model.byId.get(nodeId.task("assigned"))!.parentId).toBeNull();
    expect(model.byId.get(nodeId.run("worker"))!.parentId).toBe(nodeId.task("assigned"));
  });

  it("uses collision-safe identities, deduplicates and handles cyclic or deeply nested workers iteratively", () => {
    expect(nodeId.run("task:a")).not.toBe(nodeId.task("run:a"));
    const root = run("root");
    const workers = Array.from({ length: 2500 }, (_, i) => run(`nested-${i}`, { parent_run_id: i ? `nested-${i - 1}` : "worker" }));
    const snap = snapshot([root, run("worker"), run("worker"), ...workers, run("a", { parent_run_id: "b" }), run("b", { parent_run_id: "a" })], [ownership("one", "worker")]);
    const model = buildSupervisorGraph({ snapshot: snap, root, rootId: "root", tasks: [task("one", 1, "worker"), task("one", 2)] });
    expect(model.nodes).toHaveLength(2505);
    expect(new Set(model.nodes.map(node => node.id)).size).toBe(model.nodes.length);
    expect(model.byId.get(nodeId.run("a"))!.parentId).toBeNull();
    expect(pathNodes(model, nodeId.run("nested-2499"), false)).toHaveLength(2502);
    expect(model.layout.positions.every(position => Number.isFinite(position.x) && Number.isFinite(position.y))).toBe(true);
    expect(buildSupervisorGraph({ snapshot: { ...snap, runs: [...snap.runs].reverse() }, root, rootId: "root", tasks: [task("one", 2), task("one", 1, "worker")] }).layout).toEqual(model.layout);
  });

  it("does not turn stale, missing or unbound runtime records into a working agent", () => {
    const root = run("root");
    const worker = run("worker", { kind: "adopted", attachment: { attachment_id: "00000000-0000-4000-8000-000000000002", epoch: 1, state: "connected", launch_mode: null, location: null, conversation: { harness: "codex", session_id: "opaque/session" }, process: null, shell: null, machine_id: null, launch_attempt: 0, updated_at: "2026-10-07T00:00:00Z" } });
    const snap = snapshot([root, worker]);
    if (snap.runtime.status === "fresh") snap.runtime.runs.push({ run_id: "worker", presence: "present", actual_agent: true, workspace_id: "space", workspace_label: "Project", tab_id: null, tab_label: null, pane_id: "pane", agent_status: "working", state_changed_at: null });
    const model = buildSupervisorGraph({ snapshot: snap, root, rootId: "root", tasks: [] });
    const node = model.byId.get(nodeId.run("worker"))!;
    const ctx = { model, snapshot: snap, live: true, connected: true, runtimeLive: true };
    expect(nodeFacts(node, ctx)).toMatchObject({ glyph: "working", status: "working", provenance: "Herdr · Project" });
    expect(nodeFacts(node, { ...ctx, live: false })).toMatchObject({ glyph: "unknown", status: "unobserved", provenance: "Herdr" });
    if (snap.runtime.status === "fresh") snap.runtime.runs[0].presence = "missing";
    expect(nodeFacts(node, ctx)).toMatchObject({ glyph: "unknown", status: "unobserved", provenance: "Herdr" });
  });
});
