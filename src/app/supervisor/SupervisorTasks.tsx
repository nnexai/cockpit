import type { Run, TaskLane, TaskView } from "../../protocol/generated/v1";
import { dependencySummary } from "./dependencies";

export const taskLanes: readonly { lane: TaskLane; label: string }[] = [
  { lane: "blocked", label: "Blocked" }, { lane: "ready", label: "Ready" },
  { lane: "working", label: "Working" }, { lane: "review", label: "Review" },
  { lane: "accepted", label: "Done" },
];
export function taskStage(view: TaskView): string {
  const stage = view.task.checked ? "done" : view.attempt?.state === "starting" ? "starting" : view.attempt?.state === "stalled" ? "stalled" : view.lane === "accepted" ? "done" : view.lane;
  return `${stage}${view.attempt && view.attempt.attempt > 1 ? ` · attempt ${view.attempt.attempt}` : ""}`;
}
export function TaskRow({ view, tasks }: { view: TaskView; tasks: readonly TaskView[] }) {
  const dependency = dependencySummary(view, tasks);
  return <div className={`liaison-task-row lane-${view.lane}${view.attempt ? ` attempt-${view.attempt.state}` : ""}`}>
    <span className="liaison-task-dot" aria-hidden="true" />
    <span className="liaison-task-title" title={view.task.title}>{view.task.title}</span>
    <span className="liaison-task-stage">{taskStage(view)}</span>
    {dependency ? <span className="liaison-task-after" title={dependency}>↳ {dependency}</span> : null}
  </div>;
}
export function TaskRail({ tasks, waiting, status, onOverview }: { tasks: readonly TaskView[]; waiting: boolean; status: string | null; onOverview(): void }) {
  const open = tasks.filter(view => !view.task.checked);
  if (!open.length) return null;
  const done = tasks.length - open.length;
  const summary = taskLanes.filter(({ lane }) => lane !== "accepted").flatMap(({ lane }) => {
    const count = open.filter(view => view.lane === lane).length;
    return count ? [`${count} ${lane}`] : [];
  }).join(" · ");
  return <aside className="liaison-task-rail" role="group" aria-label="Tasks" tabIndex={0}>
    <header><strong>Tasks</strong><span className={waiting ? "liaison-waiting" : "liaison-task-status"}>{waiting ? "● waiting for you" : status ?? summary}</span><button type="button" aria-label="Open overview" title="Overview" onClick={onOverview}><svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5" aria-hidden="true"><rect x="2" y="2.5" width="5" height="11" rx="1" /><rect x="9" y="2.5" width="5" height="6" rx="1" /></svg></button></header>
    <ul>{open.map((view, index) => <li key={`${view.task.task_id}:${index}`}><TaskRow view={view} tasks={tasks} /></li>)}</ul>
    {done ? <footer><span className="liaison-task-dot lane-accepted" aria-hidden="true" />{done} done</footer> : null}
  </aside>;
}
export function SupervisorTasks({ tasks, runs }: { tasks: readonly TaskView[]; runs: readonly Run[] }) {
  const completedAt = new Map<string, string>();
  for (const run of runs) if (run.task_id && run.stage === "closed" && run.close_reason === "accepted") {
    if ((completedAt.get(run.task_id) ?? "") < run.updated_at) completedAt.set(run.task_id, run.updated_at);
  }
  return <div className="liaison-task-board" role="group" aria-label="Task board" tabIndex={0}>{taskLanes.map(({ lane, label }) => {
    const laneTasks = tasks.filter(view => view.lane === lane);
    const shown = lane === "accepted" ? laneTasks.slice().sort((a, b) => (completedAt.get(b.task.task_id) ?? "").localeCompare(completedAt.get(a.task.task_id) ?? "") || b.task.line - a.task.line).slice(0, 12) : laneTasks;
    return <section className={`liaison-task-lane lane-${lane}${!laneTasks.length ? " is-empty" : ""}`} key={lane} aria-label={`${label} tasks`}>
      <h2><span className="liaison-task-dot" aria-hidden="true" />{label}<span>{laneTasks.length}</span></h2>
      <ul>{shown.map((view, index) => {
        const dependency = dependencySummary(view, tasks), worker = runs.find(run => run.run_id === view.current_run_id)?.label;
        const meta = lane === "accepted" ? null : dependency ? `↳ ${dependency}` : [worker, view.scheduled && lane === "ready" ? "scheduled" : null, view.attempt && view.attempt.attempt > 1 ? `attempt ${view.attempt.attempt}` : null].filter(Boolean).join(" · ");
        const attemptState = lane !== "accepted" && !dependency && view.attempt && view.attempt.state !== "running" ? view.attempt.state : null;
        return <li className={`liaison-task-card lane-${lane}${view.attempt ? ` attempt-${view.attempt.state}` : ""}`} key={`${view.task.task_id}:${index}`}><div className="liaison-card-title">{view.task.title}</div>{meta || attemptState ? <div className="liaison-card-meta">{meta}{meta && attemptState ? " · " : null}{attemptState ? <span className={attemptState === "stalled" ? "liaison-attempt-stalled" : undefined}>{attemptState}</span> : null}</div> : null}</li>;
      })}</ul>
      {lane === "accepted" && laneTasks.length > shown.length ? <p className="liaison-task-earlier">+{laneTasks.length - shown.length} earlier</p> : null}
    </section>;
  })}</div>;
}
