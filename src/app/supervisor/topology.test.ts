import { describe, expect, it } from "vitest";
import type { OrchestrationSnapshot, Run, Subagent, TaskView } from "../../protocol/generated/v1";
import { buildSupervisorGraph, chainIds, firstChild, nodeFacts, nodeId, nodeSelection, pathNodes, selectionNodeId } from "./topology";

function run(id: string, overrides: Partial<Run> = {}): Run {
  return { session_id: "session", prepare_brief: "", run_id: id, kind: id === "root" ? "supervisor" : "worker", label: id, root_id: "root", parent_run_id: id === "root" ? null : "root", task_id: null, attempt: 1, task_revision_at_propose: null, stage: "working", close_reason: null, dispatch: null, target: null, setup: null, prepare_plan: null, init_receipt: null, work_plan: null, grants: [], last_report: null, result: null, annotations: [], location: null, bound_omp_session: null, bound_omp_process: null, launch_shell_identity: null, retirement: null, supersedes_run_id: null, created_at: "2026-10-07T00:00:00Z", updated_at: "2026-10-07T00:00:00Z", ...overrides };
}
function task(id: string, line: number, worker: string | null = null, lane: TaskView["lane"] = "queued"): TaskView {
  return { task: { task_id: id, title: id, body: "", checked: lane === "accepted", line, task_revision: "revision", diagnostic: null }, lane, current_run_id: worker };
}
function snapshot(runs: Run[], subagents: Subagent[] = []): OrchestrationSnapshot {
  return { session_id: "session", revision: 1, tasks_token: "token", roots: [], board: null, runs, subagents, messages: [], intents: [], assignment_intents: [], unmanaged_agents: [], attention: [], runtime: { status: "fresh", endpoint_identity: "endpoint", observed_at: "2026-10-07T00:00:00Z", runs: [] } };
}
function subagent(id: string, parent: string | null = null): Subagent {
  return { run_id: "worker", subagent_id: id, parent_subagent_id: parent, label: id, role: "scout", status: "running", summary: null, last_control: null, updated_at: "2026-10-07T00:00:00Z" };
}

describe("Supervisor topology", () => {
  it("keeps every open canonical task, worker and subagent without fabricating assignments", () => {
    const root = run("root");
    const snap = snapshot([root, run("worker", { task_id: "misleading" }), run("nested", { parent_run_id: "worker" }), run("closed", { stage: "closed" }), run("other", { root_id: "other-root" }), run("orphan", { parent_run_id: "missing" })], [subagent("child"), subagent("grandchild", "child")]);
    const tasks = [task("assigned", 1, "worker"), task("nested-task", 2, "nested"), task("no-worker", 3), task("closed-worker", 4, "closed"), task("done", 5, "worker", "accepted")];
    const model = buildSupervisorGraph({ snapshot: snap, root, rootId: "root", tasks, includeSubagents: true });
    expect(model.counts).toEqual({ supervisors: 1, tasks: 4, workers: 3, subagents: 2 });
    expect(model.hiddenCompletedTasks).toBe(1);
    expect(model.byId.has(nodeId.run("closed"))).toBe(false);
    expect(model.byId.has(nodeId.run("other"))).toBe(false);
    expect(model.byId.get(nodeId.run("worker"))!.parentId).toBe(nodeId.task("assigned"));
    expect(model.byId.get(nodeId.run("nested"))!.parentId).toBe(nodeId.run("worker"));
    expect(model.byId.get(nodeId.run("orphan"))!.parentId).toBeNull();
    expect(model.links).toEqual([{ from: nodeId.task("nested-task"), to: nodeId.run("nested"), kind: "assigned" }]);
    expect(model.byId.get(nodeId.task("closed-worker"))).toMatchObject({ unassigned: true, assignedRunId: null, edge: "unassigned" });
    expect(firstChild(model, nodeId.run("root"))).toBe(nodeId.task("assigned"));
    expect(firstChild(model, nodeId.run("worker"))).toBe(nodeId.subagent("worker", "child"));
    expect([...chainIds(model, nodeId.run("nested"))].sort()).toEqual([nodeId.run("nested"), nodeId.run("worker"), nodeId.run("root"), nodeId.task("assigned"), nodeId.task("nested-task")].sort());
    expect(pathNodes(model, nodeId.subagent("worker", "grandchild"), false).map(node => node.id)).toEqual([nodeId.task("assigned"), nodeId.run("worker"), nodeId.subagent("worker", "child"), nodeId.subagent("worker", "grandchild")]);
    expect(pathNodes(model, nodeId.run("nested"), false).map(node => node.id)).toEqual([nodeId.task("nested-task"), nodeId.task("assigned"), nodeId.run("worker"), nodeId.run("nested")]);
    expect(pathNodes(model, nodeId.run("worker"), true).map(node => node.id)).toEqual([nodeId.task("assigned"), nodeId.run("worker"), nodeId.subagent("worker", "child"), nodeId.run("nested")]);
    for (const node of model.nodes) {
      const selection = nodeSelection(node);
      expect(selectionNodeId("task" in selection ? { selectedTask: selection.task, selectedRun: null, selectedSubagent: null } : { selectedTask: null, selectedRun: selection.run, selectedSubagent: selection.subagent })).toBe(node.id);
    }
  });

  it("retains orphaned work after root closure, while hiding closed runs and their subagents", () => {
    const root = run("root", { stage: "closed" });
    const snap = snapshot([root, run("worker"), run("closed", { stage: "closed" })], [subagent("open"), { ...subagent("closed-child"), run_id: "closed" }]);
    const model = buildSupervisorGraph({ snapshot: snap, root, rootId: "root", tasks: [task("assigned", 1, "worker"), task("unassigned", 2)], includeSubagents: true });
    expect(model.counts).toEqual({ supervisors: 0, tasks: 2, workers: 1, subagents: 1 });
    expect(model.byId.get(nodeId.task("assigned"))!.parentId).toBeNull();
    expect(model.byId.get(nodeId.run("worker"))!.parentId).toBe(nodeId.task("assigned"));
    const withoutSubagents = buildSupervisorGraph({ snapshot: snap, root, rootId: "root", tasks: [task("assigned", 1, "worker")], includeSubagents: false });
    expect(withoutSubagents.nodes.map(node => node.id)).toEqual([nodeId.task("assigned"), nodeId.run("worker")]);
  });

  it("uses collision-safe identities, deduplicates and handles cyclic or deeply nested events iteratively", () => {
    expect(nodeId.subagent("a:b", "c")).not.toBe(nodeId.subagent("a", "b:c"));
    const root = run("root");
    const agents = Array.from({ length: 2500 }, (_, i) => subagent(`sub-${i}`, i ? `sub-${i - 1}` : null));
    agents.push(subagent("cycle-a", "cycle-b"), subagent("cycle-b", "cycle-a"), subagent("sub-0"));
    const snap = snapshot([root, run("worker"), run("worker"), run("a", { parent_run_id: "b" }), run("b", { parent_run_id: "a" })], agents);
    const model = buildSupervisorGraph({ snapshot: snap, root, rootId: "root", tasks: [task("one", 1, "worker"), task("one", 2)], includeSubagents: true });
    expect(model.nodes).toHaveLength(2507);
    expect(new Set(model.nodes.map(node => node.id)).size).toBe(model.nodes.length);
    expect(model.byId.get(nodeId.run("a"))!.parentId).toBeNull();
    expect(model.byId.get(nodeId.subagent("worker", "cycle-a"))!.parentId).toBeNull();
    expect(pathNodes(model, nodeId.subagent("worker", "sub-2499"), false)).toHaveLength(2502);
    expect(model.layout.positions.every(position => Number.isFinite(position.x) && Number.isFinite(position.y))).toBe(true);
    expect(buildSupervisorGraph({ snapshot: { ...snap, runs: [...snap.runs].reverse(), subagents: [...snap.subagents].reverse() }, root, rootId: "root", tasks: [task("one", 2), task("one", 1, "worker")], includeSubagents: true }).layout).toEqual(model.layout);
  });

  it("does not turn stale, missing or unbound runtime records into a working agent", () => {
    const root = run("root");
    const worker = run("worker", { kind: "adopted", bound_omp_session: "omp" });
    const snap = snapshot([root, worker], [subagent("child")]);
    if (snap.runtime.status === "fresh") snap.runtime.runs.push({ run_id: "worker", presence: "present", actual_omp: true, workspace_id: "space", workspace_label: "Project", tab_id: null, tab_label: null, pane_id: "pane", agent_status: "working", state_changed_at: null });
    const model = buildSupervisorGraph({ snapshot: snap, root, rootId: "root", tasks: [], includeSubagents: true });
    const node = model.byId.get(nodeId.run("worker"))!;
    const ctx = { model, snapshot: snap, live: true, connected: true, runtimeLive: true };
    expect(nodeFacts(node, ctx)).toMatchObject({ glyph: "working", status: "working", provenance: "Herdr · Project" });
    expect(nodeFacts(node, { ...ctx, live: false })).toMatchObject({ glyph: "unknown", status: "unobserved", provenance: "Herdr" });
    if (snap.runtime.status === "fresh") snap.runtime.runs[0].presence = "missing";
    expect(nodeFacts(node, ctx)).toMatchObject({ glyph: "unknown", status: "unobserved", provenance: "Herdr" });
    expect(nodeFacts(model.byId.get(nodeId.subagent("worker", "child"))!, { ...ctx, live: false })).toMatchObject({ glyph: "working", status: "running", provenance: "OMP events · no terminal" });
  });
});
