import type { Run, TaskView } from "../../protocol/generated/v1";
import { orderDoneTasks } from "./taskHistory";

export type TaskProject = { key: string; label: string };
export type TaskGroup = TaskProject & { tasks: TaskView[]; open: number; done: number; live: boolean; activity: number };

/** Project identity and its display label are projected by the orchestration snapshot. */
export function projectOf(view: TaskView): TaskProject {
  return view.project ? { key: view.project.key ?? `provisional:${view.project.label}`, label: view.project.label } : { key: "", label: "General" };
}

export function groupTasks(tasks: readonly TaskView[], runs: readonly Run[]): TaskGroup[] {
  const groups = new Map<string, TaskGroup>();
  const activity = new Map<string, number>();
  for (const run of runs) {
    if (!run.task_id) continue;
    const time = Date.parse(run.updated_at);
    if (Number.isFinite(time)) activity.set(run.task_id, Math.max(activity.get(run.task_id) ?? -Infinity, time));
  }
  for (const view of tasks) {
    const project = projectOf(view);
    let group = groups.get(project.key);
    if (!group) {
      group = { ...project, tasks: [], open: 0, done: 0, live: false, activity: -Infinity };
      groups.set(project.key, group);
    }
    group.tasks.push(view);
    if (view.task.checked) group.done++;
    else {
      group.open++;
      group.live ||= view.lane === "working" || view.lane === "review";
    }
    group.activity = Math.max(group.activity, activity.get(view.task.task_id) ?? view.task.line);
  }
  for (const group of groups.values()) {
    const open = group.tasks.filter(view => !view.task.checked).sort((a, b) => a.task.line - b.task.line);
    group.tasks = [...open, ...orderDoneTasks(group.tasks.filter(view => view.task.checked), runs)];
  }
  const tier = (group: TaskGroup) => group.live ? 0 : group.open ? 1 : 2;
  return [...groups.values()].sort((a, b) => Number(!a.key) - Number(!b.key) || tier(a) - tier(b) || b.activity - a.activity || a.label.localeCompare(b.label, undefined, { sensitivity: "base" }) || a.key.localeCompare(b.key));
}
