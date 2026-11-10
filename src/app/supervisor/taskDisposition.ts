import type { ActorRef, TaskView } from "../../protocol/generated/v1";
import type { SupervisorOverview } from "./useSupervisor";

export type TaskDisposition = { kind: "completed_elsewhere" | "abandoned"; reason: string; evidence?: string | null; at: string; by: ActorRef; reopened_at?: string | null; reopened_by?: ActorRef | null };

export function taskDisposition(snapshot: SupervisorOverview | null, rootId: string | null, view: TaskView, tasks: readonly TaskView[]): TaskDisposition | null {
  if (!snapshot || !rootId || tasks.filter(task => task.task.task_id === view.task.task_id).length !== 1) return null;
  const matches = snapshot.task_ownership.filter(owner => owner.root_id === rootId && owner.task_id === view.task.task_id);
  return matches.length === 1 ? (matches[0] as typeof matches[number] & { disposition?: TaskDisposition | null }).disposition ?? null : null;
}


export function dispositionHistory(snapshot: SupervisorOverview | null, rootId: string | null, view: TaskView, tasks: readonly TaskView[]): TaskDisposition[] {
  if (!snapshot || !rootId || tasks.filter(task => task.task.task_id === view.task.task_id).length !== 1) return [];
  const matches = snapshot.task_ownership.filter(owner => owner.root_id === rootId && owner.task_id === view.task.task_id);
  return matches.length === 1 ? (matches[0] as typeof matches[number] & { disposition_history?: TaskDisposition[] }).disposition_history ?? [] : [];
}
export function dispositionLabel(disposition: TaskDisposition): string {
  return disposition.kind === "completed_elsewhere" ? "Set aside · completed elsewhere" : disposition.kind === "abandoned" ? "Set aside · abandoned" : "Set aside · reason unavailable";
}

export function dispositionActor(actor: ActorRef): string {
  return actor.type === "operator" ? "Operator" : actor.type === "dispatcher" ? "Dispatcher" : `Run ${actor.run_id}`;
}
