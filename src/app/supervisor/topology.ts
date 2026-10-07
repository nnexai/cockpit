import type { OrchestrationSnapshot, Run, Subagent, TaskView } from "../../protocol/generated/v1";
import type { GlyphShape } from "../sidebar/StateGlyph";
import { agentState } from "./SupervisorActions";
import { graphLayout, type GraphNode, type GraphLayout } from "./graphLayout";
import { taskLanes } from "./boardNavigation";

export const nodeId = {
  run: (id: string) => `run:${encodeURIComponent(id)}`,
  task: (id: string) => `task:${encodeURIComponent(id)}`,
  subagent: (run: string, sub: string) => `sub:${encodeURIComponent(run)}:${encodeURIComponent(sub)}`,
};
export type TopologyNodeKind = "supervisor" | "task" | "worker" | "subagent";
export type TopologyEdgeKind = "task" | "assigned" | "delegated" | "subagent" | "unassigned";
export type TopologyNode = {
  id: string; kind: TopologyNodeKind; parentId: string | null; edge: TopologyEdgeKind | null; minColumn: number;
  run: Run | null; task: TaskView | null; subagent: Subagent | null;
  assignedRunId: string | null; linkedTaskId: string | null; unassigned: boolean;
};
export type TopologyLink = { from: string; to: string; kind: "assigned" };
export type SupervisorGraphModel = {
  nodes: readonly TopologyNode[]; byId: ReadonlyMap<string, TopologyNode>; children: ReadonlyMap<string, readonly string[]>;
  links: readonly TopologyLink[]; layout: GraphLayout; hiddenCompletedTasks: number;
  counts: { supervisors: number; tasks: number; workers: number; subagents: number };
};
const compare = (a: string, b: string) => a < b ? -1 : a > b ? 1 : 0;
const runOrder = (a: Run, b: Run) => compare(a.created_at, b.created_at) || compare(a.run_id, b.run_id);

/** Relationships are canonical assignments and explicit parentage, never inferred from labels or task_id. */
export function buildSupervisorGraph({ snapshot, root, rootId, tasks, includeSubagents }: {
  snapshot: OrchestrationSnapshot; root: Run | null; rootId: string; tasks: readonly TaskView[]; includeSubagents: boolean;
}): SupervisorGraphModel {
  const runs = new Map<string, Run>();
  const candidates = root && !snapshot.runs.some(run => run.run_id === root.run_id) ? [...snapshot.runs, root] : snapshot.runs;
  for (const run of [...candidates].sort(runOrder)) {
    if (run.root_id === rootId && run.stage !== "closed" && !runs.has(run.run_id)) runs.set(run.run_id, run);
  }
  const supervisor = runs.get(rootId);
  const supervisorId = supervisor && (supervisor.kind === "supervisor" || supervisor.kind === "adopted") ? nodeId.run(rootId) : null;
  const uniqueTasks = new Map<string, TaskView>();
  for (const task of [...tasks].sort((a, b) => a.task.line - b.task.line || compare(a.task.task_id, b.task.task_id))) {
    if (!uniqueTasks.has(task.task.task_id)) uniqueTasks.set(task.task.task_id, task);
  }
  const assignments = new Map<string, TaskView>();
  for (const task of uniqueTasks.values()) {
    if (task.lane !== "accepted" && task.current_run_id && runs.has(task.current_run_id) && !assignments.has(task.current_run_id)) assignments.set(task.current_run_id, task);
  }
  const byId = new Map<string, TopologyNode>();
  const layoutNodes: GraphNode[] = [];
  const links: TopologyLink[] = [];
  const add = (node: TopologyNode, order: readonly (string | number)[], gapBefore = 0) => {
    byId.set(node.id, node); layoutNodes.push({ id: node.id, parentId: node.parentId, minColumn: node.minColumn, order, gapBefore });
  };
  let firstUnassigned = true;
  let hiddenCompletedTasks = 0;
  for (const task of uniqueTasks.values()) {
    if (task.lane === "accepted") { hiddenCompletedTasks++; continue; }
    const assignedRunId = task.current_run_id && runs.has(task.current_run_id) ? task.current_run_id : null;
    const unassigned = assignedRunId === null;
    add({ id: nodeId.task(task.task.task_id), kind: "task", parentId: supervisorId, edge: supervisorId ? unassigned ? "unassigned" : "task" : null,
      minColumn: 1, run: null, task, subagent: null, assignedRunId, linkedTaskId: null, unassigned },
    [unassigned ? 3 : 0, task.task.line, task.task.task_id], unassigned && firstUnassigned ? 0.4 : 0);
    if (unassigned) firstUnassigned = false;
  }
  for (const run of runs.values()) {
    const isSupervisor = nodeId.run(run.run_id) === supervisorId;
    const parentRun = run.parent_run_id ? runs.get(run.parent_run_id) : undefined;
    const task = assignments.get(run.run_id);
    const nested = parentRun && parentRun.run_id !== rootId;
    const parentId = isSupervisor ? null : task && !nested ? nodeId.task(task.task.task_id) : parentRun ? nodeId.run(parentRun.run_id) : null;
    add({ id: nodeId.run(run.run_id), kind: isSupervisor ? "supervisor" : "worker", parentId,
      edge: parentId ? task && !nested ? "assigned" : "delegated" : null, minColumn: isSupervisor ? 0 : 2,
      run, task: null, subagent: null, assignedRunId: null, linkedTaskId: task?.task.task_id ?? null, unassigned: false },
    [isSupervisor ? -1 : 1, run.created_at, run.run_id]);
  }
  for (const task of uniqueTasks.values()) {
    if (task.lane === "accepted" || !task.current_run_id || !runs.has(task.current_run_id)) continue;
    const from = nodeId.task(task.task.task_id), to = nodeId.run(task.current_run_id);
    if (byId.get(to)?.parentId !== from) links.push({ from, to, kind: "assigned" });
  }
  if (includeSubagents) {
    const agents = new Map<string, Subagent>();
    for (const agent of [...snapshot.subagents].sort((a, b) => compare(nodeId.subagent(a.run_id, a.subagent_id), nodeId.subagent(b.run_id, b.subagent_id)))) {
      const id = nodeId.subagent(agent.run_id, agent.subagent_id);
      if (runs.has(agent.run_id) && !agents.has(id)) agents.set(id, agent);
    }
    for (const [id, subagent] of agents) {
      const parent = subagent.parent_subagent_id ? nodeId.subagent(subagent.run_id, subagent.parent_subagent_id) : null;
      add({ id, kind: "subagent", parentId: parent && agents.has(parent) ? parent : nodeId.run(subagent.run_id), edge: "subagent", minColumn: 3,
        run: runs.get(subagent.run_id)!, task: null, subagent, assignedRunId: null, linkedTaskId: null, unassigned: false },
      [subagent.run_id === rootId ? 2 : 0, subagent.subagent_id]);
    }
  }
  const layout = graphLayout(layoutNodes);
  const children = new Map<string, string[]>();
  const counts = { supervisors: 0, tasks: 0, workers: 0, subagents: 0 };
  const nodes = layout.order.map(id => {
    const node = byId.get(id)!;
    node.parentId = layout.parentOf.get(id) ?? null;
    if (!node.parentId) node.edge = null;
    if (node.parentId) {
      const siblings = children.get(node.parentId);
      if (siblings) siblings.push(id); else children.set(node.parentId, [id]);
    }
    if (node.kind === "supervisor") counts.supervisors++;
    else if (node.kind === "task") counts.tasks++;
    else if (node.kind === "worker") counts.workers++;
    else counts.subagents++;
    return node;
  });
  return { nodes, byId, children, links, layout, hiddenCompletedTasks, counts };
}
export function firstChild(model: SupervisorGraphModel, id: string): string | null { return model.children.get(id)?.[0] ?? null; }
export function chainIds(model: SupervisorGraphModel, id: string): ReadonlySet<string> {
  const ids = new Set<string>();
  const pending = [id];
  const linked = new Map<string, string[]>();
  for (const link of model.links) {
    for (const [from, to] of [[link.from, link.to], [link.to, link.from]]) {
      const neighbors = linked.get(from);
      if (neighbors) neighbors.push(to); else linked.set(from, [to]);
    }
  }
  while (pending.length) {
    const current = pending.pop()!;
    const node = model.byId.get(current);
    if (!node || ids.has(current)) continue;
    ids.add(current);
    if (node.parentId) pending.push(node.parentId);
    if (node.assignedRunId) pending.push(nodeId.run(node.assignedRunId));
    if (node.linkedTaskId) pending.push(nodeId.task(node.linkedTaskId));
    for (const neighbor of linked.get(current) ?? []) pending.push(neighbor);
  }
  return ids;
}
export function pathNodes(model: SupervisorGraphModel, id: string, includeChildren: boolean): readonly TopologyNode[] {
  const ids = new Set(chainIds(model, id));
  const nodes = model.nodes.filter(node => node.kind !== "supervisor" && ids.has(node.id));
  const selected = model.byId.get(id);
  const worker = selected?.run ? model.byId.get(nodeId.run(selected.run.run_id)) : undefined;
  if (worker?.linkedTaskId && model.links.some(link => link.to === worker.id && link.from === nodeId.task(worker.linkedTaskId!))) {
    const index = nodes.findIndex(node => node.id === nodeId.task(worker.linkedTaskId!));
    if (index > 0) nodes.unshift(nodes.splice(index, 1)[0]);
  }
  if (includeChildren) for (const child of model.children.get(id) ?? []) {
    const node = model.byId.get(child)!;
    if (!ids.has(child) && node.kind !== "supervisor") nodes.push(node);
  }
  return nodes;
}
export type NodeSelection = { task: string } | { run: string; subagent: string | null };
export function selectionNodeId(s: { selectedTask: string | null; selectedRun: string | null; selectedSubagent: string | null }): string | null {
  return s.selectedTask !== null ? nodeId.task(s.selectedTask) : s.selectedRun !== null ? s.selectedSubagent !== null ? nodeId.subagent(s.selectedRun, s.selectedSubagent) : nodeId.run(s.selectedRun) : null;
}
export function nodeSelection(node: TopologyNode): NodeSelection {
  return node.task ? { task: node.task.task.task_id } : { run: node.run!.run_id, subagent: node.subagent?.subagent_id ?? null };
}
export type NodeFacts = { glyph: GlyphShape | "document"; title: string; role: string; status: string; provenance: string; relation: string | null };
export function nodeFacts(node: TopologyNode, ctx: {
  model: SupervisorGraphModel; snapshot: OrchestrationSnapshot; live: boolean; connected: boolean; runtimeLive: boolean; labelFor?: (run: Run) => string;
}): NodeFacts {
  const labelFor = ctx.labelFor ?? ((run: Run) => run.label);
  const parent = node.parentId ? ctx.model.byId.get(node.parentId) : undefined;
  if (node.task) {
    const worker = node.assignedRunId ? ctx.model.byId.get(nodeId.run(node.assignedRunId))?.run : null;
    const closed = node.task.current_run_id ? ctx.snapshot.runs.find(run => run.run_id === node.task!.current_run_id && run.stage === "closed") : null;
    return { glyph: "document", title: node.task.task.title, role: "Task", status: taskLanes.find(lane => lane.lane === node.task!.lane)!.label,
      provenance: worker ? labelFor(worker) : closed ? "worker closed" : "not assigned", relation: worker ? `Assigned to ${labelFor(worker)}` : null };
  }
  const run = node.run!;
  if (node.subagent) {
    const status = node.subagent.status;
    return { glyph: status === "running" ? "working" : status === "done" ? "done" : status === "failed" ? "blocked" : "unknown",
      title: node.subagent.label, role: node.subagent.role ?? "OMP subagent", status: status.replaceAll("_", " "), provenance: "OMP events · no terminal",
      relation: `Subagent of ${parent?.subagent?.label ?? labelFor(run)}` };
  }
  const fresh = ctx.live && ctx.connected && ctx.runtimeLive && ctx.snapshot.runtime.status === "fresh";
  const state = agentState(ctx.snapshot, run, ctx.connected, ctx.runtimeLive);
  const observed = fresh && ctx.snapshot.runtime.status === "fresh" ? ctx.snapshot.runtime.runs.find(item => item.run_id === run.run_id) : undefined;
  const status = fresh && state.verified ? observed?.agent_status ?? "unknown" : "unobserved";
  return { glyph: status === "working" || status === "idle" || status === "blocked" || status === "done" ? status : "unknown",
    title: labelFor(run), role: node.kind === "supervisor" ? "Supervisor" : "Worker", status: status.replaceAll("_", " "),
    provenance: `Herdr${state.verified && observed?.presence === "present" && observed.workspace_label ? ` · ${observed.workspace_label}` : ""}`,
    relation: parent?.task ? `Assigned to task ${parent.task.task.title}` : node.linkedTaskId ? `Assigned to task ${ctx.model.byId.get(nodeId.task(node.linkedTaskId))?.task?.task.title ?? node.linkedTaskId}` : parent?.run ? `Delegated by ${labelFor(parent.run)}` : null };
}
