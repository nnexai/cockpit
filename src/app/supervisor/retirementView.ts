import type { CleanupItem, CleanupResourceSummary, OrchestrationSnapshot, Run, RunCleanupSummary, TaskView } from "../../protocol/generated/v1";

export type CleanupView = { label: string; detail: string; tier: "notice" | "recover" | null };
export function cleanupForRun(snapshot: OrchestrationSnapshot, runId: string): RunCleanupSummary | undefined {
  return snapshot.cleanup?.find(summary => summary.run_id === runId);
}
/** Only the canonical owner identifies a task's current worker. */
export function taskOwnerRun(snapshot: OrchestrationSnapshot, task: TaskView): Run | undefined {
  const owner = snapshot.task_ownership.find(ownership => ownership.root_id === snapshot.board?.root_id && ownership.task_id === task.task.task_id)?.owner;
  return owner ? snapshot.runs.find(run => run.run_id === owner.run_id && run.root_id === snapshot.board?.root_id) : undefined;
}
/** Accepted tasks retain their historical accepted worker after ownership is cleared. */
export function taskRun(snapshot: OrchestrationSnapshot, task: TaskView): Run | undefined {
  const current = taskOwnerRun(snapshot, task);
  if (current || task.lane !== "accepted" || !task.task.checked) return current;
  let accepted: Run | undefined;
  for (const run of snapshot.runs) {
    if (run.root_id === snapshot.board?.root_id && run.task_id === task.task.task_id && run.stage === "closed" && run.close_reason === "accepted" && run.result?.kind === "result" && run.result.outcome === "succeeded" && (!accepted || run.updated_at > accepted.updated_at)) accepted = run;
  }
  return accepted;
}
export function outcomeView(run: Run): { label: "Source complete" | "Integrated" | "Done"; detail: string } | null {
  if (run.result?.kind !== "result" || run.result.outcome !== "succeeded") return null;
  if (run.stage !== "closed" || run.close_reason !== "accepted") return { label: "Source complete", detail: "Awaiting acceptance" };
  const integration = run.integration;
  if (integration?.state === "integrated") {
    const method = integration.method === "ancestor" ? `in ${integration.target_ref}` : integration.method === "patch_equivalent" ? `equivalent patch in ${integration.target_ref}` : `adapted — recorded in ${integration.target_ref}`;
    return { label: "Integrated", detail: method };
  }
  if (integration?.state === "not_applicable" && run.source_output?.kind === "non_code" && integration.result_message_id === run.result.message_id) return { label: "Done", detail: "Non-code deliverable" };
  return { label: "Source complete", detail: integration?.state === "pending" ? "Waiting for integration" : "Integration not confirmed" };
}
export function cleanupResourceLabel(resource: CleanupResourceSummary): string {
  switch (resource.kind) {
    case "worker_terminal": return "Worker terminal";
    case "worktree": return "Task worktree";
    case "branch": return "Branch";
    case "space": return "Space";
    case "scratch": return "Scratch";
    case "fixture_set": return "Disposable fixture set";
  }
}
export function cleanupItemView(item: CleanupItem): string {
  const reason = item.reason?.message;
  switch (item.state) {
    case "removed": return "Removed";
    case "kept": return `Kept — ${reason ?? "protected work"}`;
    case "unknown": return `Not confirmed — ${reason ?? "cleanup outcome unknown"}`;
    case "waiting_integration": return `Waiting — ${reason ?? "integration not confirmed"}`;
    case "reconciling": return `Waiting — ${reason ?? "reconnecting"}`;
    case "running": return "Waiting — cleanup in progress";
    case "pending": return `Waiting — ${reason ?? "cleanup pending"}`;
    case "waiting":
    case "stalled": return `Waiting — ${reason ?? (item.state === "stalled" ? "cleanup stalled" : "cleanup waiting")}`;
  }
}
export function cleanupView(summary: RunCleanupSummary | undefined): CleanupView | null {
  if (!summary) return null;
  const items = summary.items;
  if (!items.length) return { label: "No managed resources to clean up", detail: "No cleanup resources are recorded for this run.", tier: null };
  const unknown = items.find(item => item.cleanup.state === "unknown");
  const stalled = items.find(item => item.cleanup.state === "stalled");
  const kept = items.find(item => item.cleanup.state === "kept");
  const detail = (unknown ?? stalled ?? kept)?.cleanup.reason?.message ?? "The runtime rechecks owned resources automatically and keeps protected work.";
  if (unknown) return { label: "Cleanup not confirmed", detail, tier: "recover" };
  if (kept) return { label: `Kept: ${kept.cleanup.reason?.message ?? "protected work"}`, detail, tier: "notice" };
  if (stalled) return { label: "Cleanup stalled", detail, tier: "notice" };
  if (items.every(item => item.cleanup.state === "removed")) return { label: "Cleaned up", detail: "All recorded cleanup resources were removed.", tier: null };
  if (items.some(item => item.cleanup.state === "reconciling")) return { label: "Cleanup reconnecting", detail, tier: null };
  if (items.some(item => item.cleanup.state === "running")) return { label: "Cleaning up…", detail, tier: null };
  if (items.some(item => item.cleanup.state === "waiting_integration")) return { label: "Worktree waiting for integration", detail, tier: null };
  return { label: "Cleanup waiting", detail, tier: null };
}
/** Counts canonical completed tasks, not resources or historical run attempts. */
export function keptTaskCount(snapshot: OrchestrationSnapshot, tasks: readonly TaskView[]): number {
  const keptRuns = new Set((snapshot.cleanup ?? []).filter(summary => summary.items.some(item => item.cleanup.state === "kept")).map(summary => summary.run_id));
  const keptTasks = new Set(snapshot.runs.filter(run => keptRuns.has(run.run_id) && run.root_id === snapshot.board?.root_id && run.task_id).map(run => run.task_id));
  return new Set(tasks.filter(task => task.lane === "accepted" && task.task.checked && keptTasks.has(task.task.task_id)).map(task => task.task.task_id)).size;
}
