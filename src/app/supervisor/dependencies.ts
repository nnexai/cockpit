import type { OrchestrationSnapshot, TaskView } from "../../protocol/generated/v1";

/** Resolve only unique canonical identities, never a run Result. */
export function uniqueTask(tasks: readonly TaskView[], id: string): TaskView | null {
  let found: TaskView | null = null;
  for (const task of tasks) if (task.task.task_id === id) { if (found) return null; found = task; }
  return found;
}
export function dependencySummary(view: TaskView, tasks: readonly TaskView[]): string | null {
  if (view.task.checked) return null;
  if (view.dependencies.state === "invalid") return `Prerequisites need fixing · ${view.dependencies.problems.map(problem => problem.message).join(" · ") || view.task.relations_diagnostic || "unreadable relationships"}`;
  const unmet = view.dependencies.unmet;
  if (!unmet.length) return null;
  const first = uniqueTask(tasks, unmet[0].task_id);
  return unmet.length === 1 ? `Waiting on “${first?.task.title ?? unmet[0].task_id}”` : `Waiting on ${unmet.length} · “${first?.task.title ?? unmet[0].task_id}” +${unmet.length - 1}`;
}
export function dependencyCandidateReason(tasks: readonly TaskView[], taskId: string, candidateId: string): string | null {
  if (taskId === candidateId) return "A task cannot wait on itself.";
  const candidate = uniqueTask(tasks, candidateId);
  if (!candidate || candidate.task.diagnostic || candidate.dependencies.state === "invalid") return "Resolve this task's identity or prerequisite diagnostic first.";
  const remaining = [candidateId], visited = new Set<string>();
  while (remaining.length) {
    const id = remaining.pop()!;
    if (id === taskId) return "Would create a dependency cycle: this task already waits on the edited task.";
    if (visited.has(id)) continue;
    visited.add(id);
    const current = uniqueTask(tasks, id);
    if (current) remaining.push(...current.task.depends_on);
  }
  return null;
}
export function taskContentReason(snapshot: OrchestrationSnapshot, task: TaskView | null, live: boolean): string | null {
  if (!live) return "Reconnect and check current agent status before changing task content.";
  if (!task) return "The canonical task is unavailable.";
  if (snapshot.runs.find(run => run.run_id === snapshot.board?.root_id)?.stage === "closed") return "Tracking is closed; tasks are read-only.";
  if (task.task.checked) return "Task is complete; its content and steps are read-only.";
  if (task.task.diagnostic) return "Resolve the canonical task diagnostic before editing.";
  if (snapshot.intents.some(intent => intent.root_id === snapshot.board?.root_id && intent.task_id === task.task.task_id)) return "Resolve the pending acceptance decision before editing task content.";
  if (snapshot.runs.some(run => run.root_id === snapshot.board?.root_id && run.task_id === task.task.task_id && run.stage !== "closed" && run.stage !== "reported")) return "Work is active. Wait for the worker's Result before editing task content.";
  return null;
}
export function dependentCount(view: TaskView, tasks: readonly TaskView[]): number {
  return tasks.filter(task => !task.task.checked && task.dependencies.unmet.some(item => item.task_id === view.task.task_id)).length;
}
export function dependencyChain(tasks: readonly TaskView[], seeds: readonly string[]): Set<string> {
  const upstream = new Map<string, string[]>(), downstream = new Map<string, string[]>();
  for (const view of tasks) {
    const id = view.task.task_id;
    upstream.set(id, view.task.depends_on);
    for (const prerequisite of view.task.depends_on) {
      const dependents = downstream.get(prerequisite) ?? [];
      dependents.push(id); downstream.set(prerequisite, dependents);
    }
  }
  const selected = new Set(seeds);
  // Traverse directions separately: a shared ancestor does not highlight sibling work.
  for (const links of [upstream, downstream]) {
    const visited = new Set(seeds), remaining = [...seeds];
    while (remaining.length) for (const next of links.get(remaining.pop()!) ?? []) {
      if (!visited.has(next)) { visited.add(next); selected.add(next); remaining.push(next); }
    }
  }
  return selected;
}
