import { useId, useLayoutEffect, useMemo, useRef, useState } from "react";
import type { OrchestrationSnapshot, Run, TaskLane, TaskView } from "../../protocol/generated/v1";
import { dependencySummary } from "./dependencies";
import { TaskHistoryDetail } from "./TaskHistoryDetail";
import { orderDoneTasks, taskHistory } from "./taskHistory";

export const taskLanes: readonly { lane: TaskLane; label: string }[] = [
  { lane: "blocked", label: "Blocked" }, { lane: "ready", label: "Ready" },
  { lane: "working", label: "Working" }, { lane: "review", label: "Review" },
  { lane: "accepted", label: "Done" },
];
export function taskStage(view: TaskView): string {
  const stage = view.task.checked ? "done" : view.attempt?.state === "starting" ? "starting" : view.attempt?.state === "stalled" ? "stalled" : view.lane === "accepted" ? "done" : view.lane;
  return `${stage}${view.attempt && view.attempt.attempt > 1 ? ` · attempt ${view.attempt.attempt}` : ""}`;
}
export function TaskRow({ view, tasks, onOpen, rowKey, selected }: { view: TaskView; tasks: readonly TaskView[]; onOpen?(): void; rowKey?: string; selected?: boolean }) {
  const dependency = dependencySummary(view, tasks);
  const content = <><span className="liaison-task-dot" aria-hidden="true" />
    <span className="liaison-task-title" title={view.task.title}>{view.task.title}</span>
    <span className="liaison-task-stage">{taskStage(view)}</span>
    {dependency ? <span className="liaison-task-after" title={dependency}>↳ {dependency}</span> : null}</>;
  const className = `liaison-task-row lane-${view.lane}${view.attempt ? ` attempt-${view.attempt.state}` : ""}`;
  return onOpen ? <button type="button" className={className} data-row-key={rowKey} aria-current={selected ? "true" : undefined} onClick={onOpen} aria-label={`${view.task.title}, ${taskStage(view)}. Open history`}>{content}</button> : <div className={className}>{content}</div>;
}
export function TaskRail({ tasks, waiting, status, onOverview, snapshot, rootId, fresh }: {
  tasks: readonly TaskView[]; waiting: boolean; status: string | null; onOverview(): void;
  snapshot: OrchestrationSnapshot | null; rootId: string | null; fresh: boolean;
}) {
  const [selected, setSelected] = useState<{ taskId: string; rowKey: string } | null>(null);
  const [doneOpen, setDoneOpen] = useState(false), [doneLimit, setDoneLimit] = useState(5);
  const rail = useRef<HTMLElement>(null), backButton = useRef<HTMLButtonElement>(null);
  const focusIntent = useRef<"back" | "row" | null>(null);
  const returnRow = useRef<string | null>(null);
  const listId = useId();
  const open = tasks.filter(view => !view.task.checked);
  const rootRuns = useMemo(() => snapshot?.runs.filter(run => run.root_id === rootId) ?? [], [snapshot, rootId]);
  const done = useMemo(() => orderDoneTasks(tasks.filter(view => view.task.checked), rootRuns), [tasks, rootRuns]);
  const view = selected ? tasks.find((task, index) => `${task.task.task_id}:${index}` === selected.rowKey) ?? tasks.find(task => task.task.task_id === selected.taskId) : null;
  const history = useMemo(() => snapshot && rootId && view ? taskHistory(snapshot, rootId, view, tasks, fresh) : null, [snapshot, rootId, view, tasks, fresh]);
  const allDone = open.length === 0;
  useLayoutEffect(() => {
    if (allDone) setDoneOpen(false);
    if (selected && !view) {
      const hadFocus = rail.current?.contains(document.activeElement);
      setSelected(null);
      if (hadFocus) rail.current?.focus({ preventScroll: true });
      return;
    }
    if (focusIntent.current === "back") backButton.current?.focus({ preventScroll: true });
    if (focusIntent.current === "row") {
      const row = [...(rail.current?.querySelectorAll<HTMLElement>("[data-row-key]") ?? [])].find(element => element.dataset.rowKey === returnRow.current);
      (row ?? rail.current)?.focus({ preventScroll: true });
    }
    focusIntent.current = null;
  }, [selected, view, tasks, allDone]);
  if (!tasks.length) return null;
  const goBack = () => {
    returnRow.current = selected?.rowKey ?? null;
    focusIntent.current = "row"; setSelected(null);
    if (!allDone && view?.task.checked) setDoneOpen(true);
  };
  const rows = (items: readonly TaskView[]) => items.map(item => {
    const index = tasks.indexOf(item), rowKey = `${item.task.task_id}:${index}`;
    return <li key={rowKey}><TaskRow view={item} tasks={tasks} rowKey={rowKey} onOpen={() => { focusIntent.current = "back"; setSelected({ taskId: item.task.task_id, rowKey }); }} /></li>;
  });
  const summary = taskLanes.filter(({ lane }) => lane !== "accepted").flatMap(({ lane }) => {
    const count = open.filter(view => view.lane === lane).length;
    return count ? [`${count} ${lane}`] : [];
  }).join(" · ");
  const overview = <button type="button" aria-label="Open overview" title="Overview" onClick={onOverview}><svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5" aria-hidden="true"><rect x="2" y="2.5" width="5" height="11" rx="1" /><rect x="9" y="2.5" width="5" height="6" rx="1" /></svg></button>;
  const hint = waiting ? "● waiting for you" : status ?? (allDone ? `all done · ${done.length} completed` : summary);
  return <aside ref={rail} className={`liaison-task-rail${selected && view ? " has-detail" : ""}`} role="group" aria-label={selected && view ? "Task history" : "Tasks"} tabIndex={0}
    onPointerDown={event => event.stopPropagation()}
    onKeyDown={event => { if (event.key === "Escape" && selected) { event.preventDefault(); event.stopPropagation(); goBack(); } }}>
    <header>{selected && view ? <button ref={backButton} type="button" className="liaison-detail-back" aria-label="Back to tasks" onClick={goBack}>‹ Tasks</button>
      : <><strong>Tasks</strong><span className={waiting ? "liaison-waiting" : "liaison-task-status"}>{hint}</span></>}{overview}</header>
    {selected && view ? <TaskHistoryDetail view={view} history={history} /> : <>
      <ul id={listId}>{rows(open)}{(allDone || doneOpen) && done.length ? <><li className="liaison-done-heading">Done</li>{rows(done.slice(0, doneLimit))}{done.length > doneLimit ? <li><button type="button" className="liaison-earlier-toggle" onClick={() => setDoneLimit(value => value + 5)}>Show {Math.min(5, done.length - doneLimit)} earlier tasks</button></li> : null}</> : null}</ul>
      {!allDone && done.length ? <footer><button type="button" className="liaison-done-toggle" aria-expanded={doneOpen} aria-controls={listId} onClick={() => setDoneOpen(value => !value)}><span className="liaison-task-dot lane-accepted" aria-hidden="true" />{done.length} done<span aria-hidden="true">›</span></button></footer> : null}
    </>}
  </aside>;
}
export function SupervisorTasks({ tasks, runs, onOpen, selectedKey }: { tasks: readonly TaskView[]; runs: readonly Run[]; onOpen(view: TaskView, rowKey: string): void; selectedKey?: string | null }) {
  const [doneLimit, setDoneLimit] = useState(12);
  const completed = orderDoneTasks(tasks.filter(view => view.lane === "accepted"), runs);
  return <div className="liaison-task-board" role="group" aria-label="Task board" tabIndex={0}>{taskLanes.map(({ lane, label }) => {
    const laneTasks = tasks.filter(view => view.lane === lane);
    const shown = lane === "accepted" ? completed.slice(0, doneLimit) : laneTasks;
    return <section className={`liaison-task-lane lane-${lane}${!laneTasks.length ? " is-empty" : ""}`} key={lane} aria-label={`${label} tasks`}>
      <h2><span className="liaison-task-dot" aria-hidden="true" />{label}<span>{laneTasks.length}</span></h2>
      <ul>{shown.map(view => {
        const dependency = dependencySummary(view, tasks), worker = runs.find(run => run.run_id === view.current_run_id)?.label;
        const meta = lane === "accepted" ? null : dependency ? `↳ ${dependency}` : [worker, view.scheduled && lane === "ready" ? "scheduled" : null, view.attempt && view.attempt.attempt > 1 ? `attempt ${view.attempt.attempt}` : null].filter(Boolean).join(" · ");
        const attemptState = lane !== "accepted" && !dependency && view.attempt && view.attempt.state !== "running" ? view.attempt.state : null;
        const rowKey = `${view.task.task_id}:${tasks.indexOf(view)}`;
        return <li className={`liaison-task-card lane-${lane}${view.attempt ? ` attempt-${view.attempt.state}` : ""}`} key={rowKey}><button type="button" className="liaison-card-open" data-row-key={rowKey} aria-current={selectedKey === rowKey ? "true" : undefined} aria-label={`${view.task.title}, ${taskStage(view)}. Open history`} onClick={() => onOpen(view, rowKey)}><div className="liaison-card-title">{view.task.title}</div>{meta || attemptState ? <div className="liaison-card-meta">{meta}{meta && attemptState ? " · " : null}{attemptState ? <span className={attemptState === "stalled" ? "liaison-attempt-stalled" : undefined}>{attemptState}</span> : null}</div> : null}</button></li>;
      })}</ul>
      {lane === "accepted" && laneTasks.length > shown.length ? <button type="button" className="liaison-task-earlier" onClick={() => setDoneLimit(value => value + 12)}>Show {Math.min(12, laneTasks.length - shown.length)} earlier tasks</button> : null}
    </section>;
  })}</div>;
}
