import { useId, useLayoutEffect, useMemo, useRef, useState } from "react";
import type { TaskLane, TaskView } from "../../protocol/generated/v1";
import type { SupervisorOverview } from "./useSupervisor";
import type { CockpitClient } from "../../client/CockpitClient";
import { useTaskHistoryMessages } from "./useTaskHistoryMessages";
import { dependencySummary } from "./dependencies";
import { TaskHistoryDetail } from "./TaskHistoryDetail";
import { orderDoneTasks, taskHistory } from "./taskHistory";
import { groupTasks, projectOf } from "./taskProjects";
import { taskActivity, type TaskActivity } from "./taskActivity";
import { dispositionHistory, dispositionLabel, taskDisposition, type TaskDisposition } from "./taskDisposition";
import { uniqueTask } from "./dependencies";

export function blockedBySetAside(view: TaskView, tasks: readonly TaskView[], snapshot: SupervisorOverview | null, rootId: string | null): boolean {
  return view.dependencies.unmet.some(item => {
    const prerequisite = uniqueTask(tasks, item.task_id);
    return prerequisite !== null && taskDisposition(snapshot, rootId, prerequisite, tasks) !== null;
  });
}
export function setAsidePrerequisites(view: TaskView, tasks: readonly TaskView[], snapshot: SupervisorOverview | null, rootId: string | null): string[] {
  return view.dependencies.unmet.flatMap(item => {
    const prerequisite = uniqueTask(tasks, item.task_id);
    return prerequisite && !prerequisite.task.checked && taskDisposition(snapshot, rootId, prerequisite, tasks) ? [prerequisite.task.title] : [];
  });
}


export const taskLanes: readonly { lane: TaskLane; label: string }[] = [
  { lane: "blocked", label: "Blocked" }, { lane: "ready", label: "Ready" },
  { lane: "working", label: "Assigned" }, { lane: "review", label: "Review" },
  { lane: "accepted", label: "Done" },
];
export function taskStage(view: TaskView, activity: TaskActivity | null = null, disposition: TaskDisposition | null = null): string {
  const stage = disposition ? dispositionLabel(disposition) : view.task.checked || view.lane === "accepted" ? "done" : activity?.word ?? (view.lane === "working" ? "unconfirmed" : view.lane);
  return `${stage}${view.attempt && view.attempt.attempt > 1 ? ` · attempt ${view.attempt.attempt}` : ""}`;
}
export function TaskRow({ view, tasks, onOpen, rowKey, selected, projectLabel, activity = null, disposition = null, prerequisiteSetAside = false }: { view: TaskView; tasks: readonly TaskView[]; onOpen?(): void; rowKey?: string; selected?: boolean; projectLabel?: string; activity?: TaskActivity | null; disposition?: TaskDisposition | null; prerequisiteSetAside?: boolean }) {
  const dependency = prerequisiteSetAside ? "Prerequisite not satisfied" : dependencySummary(view, tasks);
  const content = <><span className="liaison-task-dot" aria-hidden="true" />
    <span className="liaison-task-title" title={view.task.title}>{view.task.title}</span>
    <span className="liaison-task-stage" title={activity?.detail}>{projectLabel ? `${projectLabel} · ` : ""}{taskStage(view, activity, disposition)}</span>
    {dependency ? <span className="liaison-task-after" title={dependency}>↳ {dependency}</span> : null}</>;
  const className = `liaison-task-row lane-${disposition ? "set-aside" : view.lane}${!disposition && view.attempt ? ` attempt-${view.attempt.state}` : ""}${!disposition && activity ? ` activity-${activity.kind}` : ""}`;
  return onOpen ? <button type="button" className={className} data-row-key={rowKey} aria-current={selected ? "true" : undefined} onClick={onOpen} aria-label={`${view.task.title}, ${taskStage(view, activity, disposition)}${prerequisiteSetAside ? ", Prerequisite not satisfied" : ""}. Open history`}>{content}</button> : <div className={className}>{content}</div>;
}
export function TaskRail({ client, tasks, waiting, status, onOverview, snapshot, rootId, fresh, connected = true, active = true }: {
  client: CockpitClient; connected?: boolean; active?: boolean; tasks: readonly TaskView[]; waiting: boolean; status: string | null; onOverview(): void;
  snapshot: SupervisorOverview | null; rootId: string | null; fresh: boolean;
}) {
  const [selected, setSelected] = useState<{ taskId: string; rowKey: string; client: CockpitClient; scope: string } | null>(null);
  const [doneOpen, setDoneOpen] = useState(false), [doneLimit, setDoneLimit] = useState(5);
  const rail = useRef<HTMLElement>(null), backButton = useRef<HTMLButtonElement>(null);
  const focusIntent = useRef<"back" | "row" | null>(null);
  const returnRow = useRef<string | null>(null);
  const listId = useId();
  const open = tasks.filter(view => !view.task.checked && !taskDisposition(snapshot, rootId, view, tasks));
  const setAside = tasks.filter(view => taskDisposition(snapshot, rootId, view, tasks));
  const rootRuns = useMemo(() => snapshot?.runs.filter(run => run.root_id === rootId) ?? [], [snapshot, rootId]);
  const done = useMemo(() => orderDoneTasks(tasks.filter(view => view.task.checked), rootRuns), [tasks, rootRuns]);
  const groups = useMemo(() => groupTasks(tasks, rootRuns), [tasks, rootRuns]);
  const selectionScope = JSON.stringify([snapshot?.session_id, snapshot?.state_store_id, rootId]);
  const view = active && selected && selected.client === client && selected.scope === selectionScope ? tasks.find((task, index) => `${task.task.task_id}:${index}` === selected.rowKey) ?? tasks.find(task => task.task.task_id === selected.taskId) : null;
  const historyMessages = useTaskHistoryMessages(client, snapshot && rootId && view && tasks.filter(task => task.task.task_id === view.task.task_id).length === 1 ? { sessionId: snapshot.session_id, stateStoreId: snapshot.state_store_id, rootId, taskId: view.task.task_id } : null, snapshot?.revision ?? 0, connected, snapshot?.tasks_token);
  const history = useMemo(() => {
    if (!snapshot || !rootId || !view) return null;
    const data = historyMessages.data;
    if (!data && tasks.filter(task => task.task.task_id === view.task.task_id).length === 1) return null;
    return taskHistory(data ? { ...snapshot, cleanup: data.cleanup, task_ownership: data.task_ownership, unreflected_inputs: data.unreflected_inputs } : snapshot, rootId, view, tasks, fresh, data?.messages ?? null, data?.runs ?? null);
  }, [snapshot, rootId, view, tasks, fresh, historyMessages.data]);
  const allDone = open.length === 0;
  useLayoutEffect(() => {
    if (allDone) setDoneOpen(false);
    if (selected && !view) {
      const hadFocus = rail.current?.contains(document.activeElement);
      setSelected(null);
      if (hadFocus && active) rail.current?.focus({ preventScroll: true });
      return;
    }
    if (focusIntent.current === "back") backButton.current?.focus({ preventScroll: true });
    if (focusIntent.current === "row") {
      const row = [...(rail.current?.querySelectorAll<HTMLElement>("[data-row-key]") ?? [])].find(element => element.dataset.rowKey === returnRow.current);
      (row ?? rail.current)?.focus({ preventScroll: true });
    }
    focusIntent.current = null;
  }, [selected, view, tasks, allDone, active]);
  if (!tasks.length) return null;
  const goBack = () => {
    returnRow.current = selected?.rowKey ?? null;
    focusIntent.current = "row"; setSelected(null);
    if (!allDone && view?.task.checked) setDoneOpen(true);
  };
  const rows = (items: readonly TaskView[]) => items.map(item => {
    const index = tasks.indexOf(item), rowKey = `${item.task.task_id}:${index}`;
    return <li key={rowKey}><TaskRow view={item} tasks={tasks} rowKey={rowKey} activity={taskActivity(item, snapshot, rootId, fresh)} disposition={taskDisposition(snapshot, rootId, item, tasks)} prerequisiteSetAside={blockedBySetAside(item, tasks, snapshot, rootId)} onOpen={() => { focusIntent.current = "back"; setSelected({ taskId: item.task.task_id, rowKey, client, scope: selectionScope }); }} /></li>;
  });
  const summary = taskLanes.filter(({ lane }) => lane !== "accepted").flatMap(({ lane, label }) => {
    const count = open.filter(view => view.lane === lane && !taskDisposition(snapshot, rootId, view, tasks)).length;
    return count ? [`${count} ${label.toLowerCase()}`] : [];
  }).concat(setAside.length ? [`${setAside.length} set aside`] : []).join(" · ");
  const overview = <button type="button" aria-label="Open overview" title="Overview" onClick={onOverview}><svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5" aria-hidden="true"><rect x="2" y="2.5" width="5" height="11" rx="1" /><rect x="9" y="2.5" width="5" height="6" rx="1" /></svg></button>;
  const shownDone = new Set((allDone || doneOpen) ? done.slice(0, doneLimit) : []);
  const hint = waiting ? "● waiting for you" : status ?? (allDone ? setAside.length ? `No open tasks · ${done.length} done · ${setAside.length} set aside` : `all done · ${done.length} completed` : summary);
  return <aside ref={rail} className={`liaison-task-rail${selected && view ? " has-detail" : ""}`} role="group" aria-label={selected && view ? "Task history" : "Tasks"} tabIndex={0}
    onPointerDown={event => event.stopPropagation()}
    onKeyDown={event => { if (event.key === "Escape" && selected) { event.preventDefault(); event.stopPropagation(); goBack(); } }}>
    <header>{selected && view ? <button ref={backButton} type="button" className="liaison-detail-back" aria-label="Back to tasks" onClick={goBack}>‹ Tasks</button>
      : <><strong>Tasks</strong><span className={waiting ? "liaison-waiting" : "liaison-task-status"}>{hint}</span></>}{overview}</header>
    {selected && view ? <TaskHistoryDetail view={view} history={history} activity={taskActivity(view, snapshot, rootId, fresh)} disposition={taskDisposition(snapshot, rootId, view, tasks)} dispositionHistory={dispositionHistory(snapshot, rootId, view, tasks)} setAsidePrerequisites={setAsidePrerequisites(view, tasks, snapshot, rootId)} historyTransfer={historyMessages} /> : <>
      <ul id={listId} className="liaison-project-list">{groups.map((group, index) => {
        const visible = group.tasks.filter(item => !item.task.checked || shownDone.has(item));
        if (!visible.length) return null;
        const headingId = `${listId}-project-${index}`;
        const asideCount = group.tasks.filter(item => !item.task.checked && taskDisposition(snapshot, rootId, item, tasks)).length;
        const count = [group.open - asideCount > 0 ? `${group.open - asideCount} open` : "", group.done ? `${group.done} done` : "", asideCount ? `${asideCount} set aside` : ""].filter(Boolean).join(" · ");
        return <li className={`liaison-project${group.key ? "" : " is-unrouted"}`} key={group.key}><h3 className="liaison-project-head"><span className="liaison-project-name" id={headingId} title={group.label}>{group.label}</span><span className="liaison-project-count">{count}</span></h3><ul aria-labelledby={headingId}>{rows(visible)}</ul></li>;
      })}{(allDone || doneOpen) && done.length > doneLimit ? <li><button type="button" className="liaison-earlier-toggle" onClick={() => setDoneLimit(value => value + 5)}>Show {Math.min(5, done.length - doneLimit)} earlier tasks</button></li> : null}</ul>
      {!allDone && done.length ? <footer><button type="button" className="liaison-done-toggle" aria-expanded={doneOpen} aria-controls={listId} onClick={() => setDoneOpen(value => !value)}><span className="liaison-task-dot lane-accepted" aria-hidden="true" />{done.length} done<span aria-hidden="true">›</span></button></footer> : null}
    </>}
  </aside>;
}
export function SupervisorTasks({ tasks, runs, onOpen, selectedKey, project, snapshot, rootId, fresh }: { tasks: readonly TaskView[]; runs: readonly SupervisorOverview["runs"][number][]; onOpen(view: TaskView, rowKey: string): void; selectedKey?: string | null; project: string | null; snapshot: SupervisorOverview | null; rootId: string | null; fresh: boolean }) {
  const [doneLimit, setDoneLimit] = useState(12);
  const filtered = project === null ? tasks : tasks.filter(view => projectOf(view).key === project);
  const completed = orderDoneTasks(filtered.filter(view => view.lane === "accepted" && !taskDisposition(snapshot, rootId, view, tasks)), runs);
  const lanes: { lane: TaskLane | null; label: string }[] = [...taskLanes];
  if (filtered.some(view => taskDisposition(snapshot, rootId, view, tasks))) lanes.push({ lane: null, label: "Set aside" });
  return <div className="liaison-task-board" role="group" aria-label="Task board" tabIndex={0}>{lanes.map(({ lane, label }) => {
    const laneTasks = filtered.filter(view => lane === null ? taskDisposition(snapshot, rootId, view, tasks) !== null : view.lane === lane && !taskDisposition(snapshot, rootId, view, tasks));
    const shown = lane === "accepted" ? completed.slice(0, doneLimit) : laneTasks;
    return <section className={`liaison-task-lane lane-${lane ?? "set-aside"}${!laneTasks.length ? " is-empty" : ""}`} key={lane ?? "set-aside"} aria-label={`${label} tasks`}>
      <h2><span className="liaison-task-dot" aria-hidden="true" />{label}<span>{laneTasks.length}</span></h2>
      <ul>{shown.map(view => {
        const disposition = taskDisposition(snapshot, rootId, view, tasks);
        const dependency = blockedBySetAside(view, tasks, snapshot, rootId) ? "Prerequisite not satisfied" : dependencySummary(view, tasks), worker = runs.find(run => run.run_id === view.current_run_id)?.label;
        const activity = taskActivity(view, snapshot, rootId, fresh);
        const meta = disposition ? dispositionLabel(disposition) : lane === "accepted" ? null : [dependency ? `↳ ${dependency}` : worker, activity?.word, view.scheduled && lane === "ready" ? "scheduled" : null, view.attempt && view.attempt.attempt > 1 ? `attempt ${view.attempt.attempt}` : null].filter(Boolean).join(" · ");
        const rowKey = `${view.task.task_id}:${tasks.indexOf(view)}`;
        return <li className={`liaison-task-card lane-${lane ?? "set-aside"}${!disposition && view.attempt ? ` attempt-${view.attempt.state}` : ""}${!disposition && activity ? ` activity-${activity.kind}` : ""}`} key={rowKey}><button type="button" className="liaison-card-open" data-row-key={rowKey} aria-current={selectedKey === rowKey ? "true" : undefined} aria-label={`${view.task.title}, ${taskStage(view, activity, disposition)}. Open history`} onClick={() => onOpen(view, rowKey)}><div className="liaison-card-title">{view.task.title}</div>{project === null || meta ? <div className="liaison-card-meta" title={activity?.detail}>{project === null ? <><span className="liaison-card-project">{projectOf(view).label}</span>{meta ? " · " : null}</> : null}{meta}</div> : null}</button></li>;
      })}</ul>
      {lane === "accepted" && laneTasks.length > shown.length ? <button type="button" className="liaison-task-earlier" onClick={() => setDoneLimit(value => value + 12)}>Show {Math.min(12, laneTasks.length - shown.length)} earlier tasks</button> : null}
    </section>;
  })}</div>;
}
