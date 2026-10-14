import type { TaskView } from "../../protocol/generated/v1";

/** Resolve only unique canonical identities, never a run Result. */
export function uniqueTask(tasks: readonly TaskView[], id: string): TaskView | null {
  let found: TaskView | null = null;
  for (const task of tasks) if (task.task.task_id === id) { if (found) return null; found = task; }
  return found;
}
export function dependencySummary(view: TaskView, tasks: readonly TaskView[]): string | null {
  if (view.task.checked) return null;
  if (view.dependencies.state === "invalid") return "Prerequisites unreadable";
  const unmet = view.dependencies.unmet;
  if (!unmet.length) return null;
  const first = uniqueTask(tasks, unmet[0].task_id);
  if (!first) return "Prerequisites unreadable";
  return `after “${first.task.title}”${unmet.length > 1 ? ` +${unmet.length - 1}` : ""}`;
}
