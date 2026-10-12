import type { OrchestrationSnapshot, Run, TaskView } from "../../protocol/generated/v1";
import type { GlyphShape } from "../sidebar/StateGlyph";
import { agentState, taskAssignment } from "./SupervisorActions";
import { graphLayout, type GraphNode, type GraphLayout } from "./graphLayout";
import { taskLanes } from "./boardNavigation";

export const nodeId = {
  run: (id: string) => `run:${encodeURIComponent(id)}`,
  task: (id: string) => `task:${encodeURIComponent(id)}`,
};
export type TopologyNodeKind = "supervisor" | "task" | "worker";
export type TopologyEdgeKind = "task" | "assigned" | "delegated" | "unassigned";
export type TopologyNode = {
  id: string; kind: TopologyNodeKind; parentId: string | null; edge: TopologyEdgeKind | null; minColumn: number;
  run: Run | null; task: TaskView | null;
  assignedRunId: string | null; linkedTaskId: string | null; unassigned: boolean;
};
export type TopologyLink = { from: string; to: string; kind: "assigned" };
export type SupervisorGraphModel = {
  nodes: readonly TopologyNode[]; byId: ReadonlyMap<string, TopologyNode>; children: ReadonlyMap<string, readonly string[]>;
  rootId: string;
  links: readonly TopologyLink[]; layout: GraphLayout; hiddenCompletedTasks: number;
  counts: { supervisors: number; tasks: number; workers: number };
};
const compare = (a: string, b: string) => a < b ? -1 : a > b ? 1 : 0;
const runOrder = (a: Run, b: Run) => compare(a.created_at, b.created_at) || compare(a.run_id, b.run_id);

/** Relationships are canonical assignments and explicit parentage, never inferred from labels or task_id. */
export function buildSupervisorGraph({ snapshot, root, rootId, tasks }: {
  snapshot: OrchestrationSnapshot; root: Run | null; rootId: string; tasks: readonly TaskView[];
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
  const owners = new Map(snapshot.task_ownership.filter(ownership => ownership.root_id === rootId).map(ownership => [ownership.task_id, ownership.owner]));
  const assignments = new Map<string, TaskView>();
  for (const task of uniqueTasks.values()) {
    const runId = owners.get(task.task.task_id)?.run_id;
    if (task.lane !== "accepted" && runId && runs.has(runId) && !assignments.has(runId)) assignments.set(runId, task);
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
    const assignedRunId = owners.get(task.task.task_id)?.run_id ?? null;
    const unassigned = assignedRunId === null;
    add({ id: nodeId.task(task.task.task_id), kind: "task", parentId: supervisorId, edge: supervisorId ? unassigned ? "unassigned" : "task" : null,
      minColumn: 1, run: null, task, assignedRunId, linkedTaskId: null, unassigned },
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
      run, task: null, assignedRunId: null, linkedTaskId: task?.task.task_id ?? null, unassigned: false },
    [isSupervisor ? -1 : 1, run.created_at, run.run_id]);
  }
  for (const task of uniqueTasks.values()) {
    const runId = owners.get(task.task.task_id)?.run_id;
    if (task.lane === "accepted" || !runId || !runs.has(runId)) continue;
    const from = nodeId.task(task.task.task_id), to = nodeId.run(runId);
    if (byId.get(to)?.parentId !== from) links.push({ from, to, kind: "assigned" });
  }
  const layout = graphLayout(layoutNodes);
  const children = new Map<string, string[]>();
  const counts = { supervisors: 0, tasks: 0, workers: 0 };
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
    return node;
  });
  return { nodes, byId, children, links, layout, hiddenCompletedTasks, counts, rootId };
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
export type NodeSelection = { task: string } | { run: string };
export function selectionNodeId(s: { selectedTask: string | null; selectedRun: string | null }): string | null {
  return s.selectedTask !== null ? nodeId.task(s.selectedTask) : s.selectedRun !== null ? nodeId.run(s.selectedRun) : null;
}
export function nodeSelection(node: TopologyNode): NodeSelection {
  return node.task ? { task: node.task.task.task_id } : { run: node.run!.run_id };
}
export type NodeFacts = { glyph: GlyphShape | "document"; title: string; role: string; status: string; provenance: string; relation: string | null };
export function nodeFacts(node: TopologyNode, ctx: {
  model: SupervisorGraphModel; snapshot: OrchestrationSnapshot; live: boolean; connected: boolean; runtimeLive: boolean; labelFor?: (run: Run) => string;
}): NodeFacts {
  const labelFor = ctx.labelFor ?? ((run: Run) => run.label);
  const parent = node.parentId ? ctx.model.byId.get(node.parentId) : undefined;
  if (node.task) {
    const { owner, worker, pending } = taskAssignment(ctx.snapshot, node.task, ctx.model.rootId);
    const label = worker?.label ? `${labelFor(worker)} · ` : "";
    return { glyph: "document", title: node.task.task.title, role: "Task", status: node.task.dependencies.state === "invalid" ? "Prerequisites need fixing" : node.task.dependencies.state === "blocked" ? `${taskLanes.find(lane => lane.lane === node.task!.lane)!.label} · Waiting on ${node.task.dependencies.unmet.length}` : taskLanes.find(lane => lane.lane === node.task!.lane)!.label,
      provenance: owner ? `Assigned · ${label}${owner.conversation.harness}` : pending ? "Unassigned · ownership pending" : "Unassigned",
      relation: owner ? `Assigned conversation ${owner.conversation.harness} · ${owner.conversation.session_id}` : null };
  }
  const run = node.run!;
  const fresh = ctx.live && ctx.connected && ctx.runtimeLive && ctx.snapshot.runtime.status === "fresh";
  const state = agentState(ctx.snapshot, run, ctx.connected, ctx.runtimeLive);
  const observed = fresh && ctx.snapshot.runtime.status === "fresh" ? ctx.snapshot.runtime.runs.find(item => item.run_id === run.run_id) : undefined;
  const status = fresh && state.verified ? observed?.agent_status ?? "unknown" : "unobserved";
  return { glyph: status === "working" || status === "idle" || status === "blocked" || status === "done" ? status : "unknown",
    title: labelFor(run), role: node.kind === "supervisor" ? "Supervisor" : "Worker", status: status.replaceAll("_", " "),
    provenance: `Herdr${state.verified && observed?.presence === "present" && observed.workspace_label ? ` · ${observed.workspace_label}` : ""}`,
    relation: parent?.task ? `Assigned to task ${parent.task.task.title}` : node.linkedTaskId ? `Assigned to task ${ctx.model.byId.get(nodeId.task(node.linkedTaskId))?.task?.task.title ?? node.linkedTaskId}` : parent?.run ? `Delegated by ${labelFor(parent.run)}` : null };
}
