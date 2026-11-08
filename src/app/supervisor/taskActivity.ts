import type { OrchestrationSnapshot, TaskView } from "../../protocol/generated/v1";

export type TaskActivity = { kind: string; word: string; detail: string };

/** Durable lane and live observation are independent; never infer activity from a lane. */
export function taskActivity(view: TaskView, snapshot: OrchestrationSnapshot | null, rootId: string | null, fresh: boolean): TaskActivity | null {
  if (view.task.checked) return null;
  const unique = snapshot?.board?.root_id === rootId && snapshot.board.tasks.filter(item => item.task.task_id === view.task.task_id).length === 1;
  const run = snapshot && rootId && unique && view.current_run_id
    ? snapshot.runs.find(item => item.run_id === view.current_run_id && item.root_id === rootId && item.task_id === view.task.task_id && item.kind !== "supervisor")
    : undefined;
  const runtime = snapshot?.runtime.status === "fresh" && fresh ? snapshot.runtime : null;
  const observation = run && runtime?.runs.find(item => item.run_id === run.run_id);
  if (run?.stage === "closed" && run.close_reason === "cancelled") {
    const observed = observation?.presence === "present" && observation.actual_agent;
    return { kind: "hold", word: "on hold", detail: `Tracking cancelled. Cockpit does not stop the worker process. ${observed ? `Terminal observed${observation.agent_status ? ` ${observation.agent_status}` : ""} · as of ${runtime!.observed_at}.` : "Terminal not observed."}` };
  }
  if (view.lane !== "working") return null;
  if (view.waiting?.cause === "recovery_blocked") return { kind: "stalled", word: "stalled", detail: "Worker recovery is blocked." };
  if (view.waiting?.cause === "question") return { kind: "waiting", word: "waiting for answer", detail: "A worker question needs an answer." };
  if (view.worker_waiting || view.waiting?.cause === "worker_waiting") return { kind: "waiting", word: "waiting · worker says", detail: "Worker says it is waiting; Cockpit has not verified it." };
  if (view.waiting) return { kind: "waiting", word: "waiting", detail: view.waiting.detail ?? "Task is waiting." };
  if (view.attempt?.state === "stalled") return { kind: "stalled", word: "stalled", detail: "Worker attempt stalled." };
  if (view.attempt?.state === "starting") return { kind: "starting", word: "starting", detail: "Worker attempt starting." };
  if (!run || run.stage !== "working" || !runtime || !observation || observation.presence === "unobserved") return { kind: "unconfirmed", word: "unconfirmed", detail: "Live worker state unknown." };
  if (observation.presence === "missing" || observation.presence === "endpoint_changed") return { kind: "not-observed", word: "not observed", detail: "Worker terminal not found; this does not show it stopped." };
  if (!observation.actual_agent) return { kind: "unconfirmed", word: "unconfirmed", detail: "Live worker state unknown." };
  const asOf = `As of ${runtime.observed_at}.`;
  if (observation.agent_status === "working") return { kind: "working", word: "working", detail: `Worker observed working. ${asOf}` };
  if (observation.agent_status === "idle" || observation.agent_status === "done") return { kind: "idle", word: "idle", detail: `Worker observed ${observation.agent_status}${observation.state_changed_at ? ` since ${observation.state_changed_at}` : ""}. No result reported. ${asOf}` };
  if (observation.agent_status === "blocked") return { kind: "runtime-blocked", word: "runtime blocked", detail: `Worker runtime reported blocked. ${asOf}` };
  return { kind: "unconfirmed", word: "unconfirmed", detail: "Live worker state unknown." };
}
