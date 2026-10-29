import { useLayoutEffect, useMemo, useRef, useState } from "react";
import type { OrchestrationSnapshot, Run, TaskView } from "../../protocol/generated/v1";
import { SupervisorTasks, taskLanes } from "./SupervisorTasks";
import { SupervisorGraph } from "./SupervisorGraph";
import { TaskHistoryDetail } from "./TaskHistoryDetail";
import { taskHistory } from "./taskHistory";
import { dependencySummary, uniqueTask } from "./dependencies";
import { groupTasks } from "./taskProjects";
import "./supervisor.css";

const attentionLabels = {
  needs_input: "Needs input",
  runtime_blocked: "Runtime reported blocked",
  brief_unread: "Work brief stored; delivery unconfirmed",
  idle_without_report: "Idle without a result",
  exited_without_report: "Worker not observed",
  dispatch_unknown: "Dispatch outcome uncertain",
  intent_conflict: "Assignment conflict",
  retirement_unconfirmed: "Retirement unconfirmed",
} as const;

function taskAttention(snapshot: OrchestrationSnapshot | null, rootId: string | null, tasks: readonly TaskView[], fresh: boolean) {
  if (!snapshot || !rootId || snapshot.board?.root_id !== rootId) return [];
  return tasks.flatMap((view, index) => {
    if (!uniqueTask(tasks, view.task.task_id)) return [];
    const run = view.current_run_id ? snapshot.runs.find(item => item.run_id === view.current_run_id && item.root_id === rootId && item.task_id === view.task.task_id && item.kind !== "supervisor") : null;
    const labels = new Set<string>();
    if (run) {
      for (const item of snapshot.attention) {
        if (item.run_id !== run.run_id || item.task_id !== view.task.task_id) continue;
        if (item.kind === "to_accept") {
          const result = run.result?.outcome === "failed" ? "Failed result reported" : run.result?.outcome === "succeeded" ? "Successful result reported" : "Result reported";
          labels.add(`${result}; not task acceptance`);
        } else {
          const runtimeFact = item.kind === "runtime_blocked" || item.kind === "idle_without_report" || item.kind === "exited_without_report" || item.kind === "dispatch_unknown";
          labels.add(runtimeFact && (!fresh || snapshot.runtime.status !== "fresh") ? `Last observed: ${attentionLabels[item.kind]}` : attentionLabels[item.kind]);
        }
      }
      const observation = snapshot.runtime.status === "fresh" ? snapshot.runtime.runs.find(item => item.run_id === run.run_id) : null;
      const observedAgent = observation?.presence === "present" && observation.actual_agent;
      if (run.stage === "working" && (!fresh || !observedAgent)) labels.add("Live worker state unknown");
    }
    if ((!view.current_run_id || run?.stage === "queued") && !view.task.checked && (view.scheduled || view.lane === "blocked")) {
      const dependency = dependencySummary(view, tasks);
      labels.add(dependency ? `Blocked: ${dependency}` : view.lane === "blocked" ? "Blocked; reason unknown" : "Waiting for dispatch; reason unknown");
    }
    if (labels.size && run && run.stage !== "closed" && snapshot.runtime.status === "fresh") {
      const observation = snapshot.runtime.runs.find(item => item.run_id === run.run_id);
      if (observation?.presence === "present" && observation.actual_agent) {
        const status = observation.agent_status ? ` ${observation.agent_status}` : "";
        labels.add(`${fresh ? "Observed" : "Last observed"}${status} · as of ${snapshot.runtime.observed_at}`);
      }
    }
    return labels.size ? [{ view, rowKey: `${view.task.task_id}:${index}`, labels: [...labels] }] : [];
  });
}

/** Optional overview; the normal terminal remains mounted as the conversation. */
export function SupervisorView({ tasks, runs, waiting, status, onClose, snapshot, rootId, fresh }: {
  tasks: readonly TaskView[]; runs: readonly Run[]; waiting: boolean; status: string | null; onClose(): void;
  snapshot: OrchestrationSnapshot | null; rootId: string | null; fresh: boolean;
}) {
  const [mode, setMode] = useState<"tasks" | "graph">("tasks");
  const [project, setProject] = useState<string | null>(null);
  const groups = useMemo(() => groupTasks(tasks, runs), [tasks, runs]);
  const activeProject = project !== null && groups.some(group => group.key === project) ? project : null;
  const attention = useMemo(() => taskAttention(snapshot, rootId, tasks, fresh), [snapshot, rootId, tasks, fresh]);
  const [selected, setSelected] = useState<{ taskId: string; rowKey: string } | null>(null);
  const overview = useRef<HTMLElement>(null), conversation = useRef<HTMLButtonElement>(null), closeDetail = useRef<HTMLButtonElement>(null);
  const focusIntent = useRef<"back" | "origin" | "conversation" | null>("conversation");
  const origin = useRef<HTMLElement | null>(null), returnRow = useRef<string | null>(null);
  const lastFocused = useRef<HTMLElement | null>(null);
  const view = selected ? tasks.find((task, index) => `${task.task.task_id}:${index}` === selected.rowKey) ?? tasks.find(task => task.task.task_id === selected.taskId) : null;
  const history = useMemo(() => snapshot && rootId && view ? taskHistory(snapshot, rootId, view, tasks, fresh) : null, [snapshot, rootId, view, tasks, fresh]);
  const selectedKey = view ? `${view.task.task_id}:${tasks.indexOf(view)}` : null;
  const visible = (element: HTMLElement) => {
    for (let ancestor: HTMLElement | null = element; ancestor && ancestor !== overview.current; ancestor = ancestor.parentElement) {
      if (ancestor.hidden || getComputedStyle(ancestor).display === "none") return false;
    }
    return true;
  };
  const findRow = (rowKey: string | null) => {
    const rows = overview.current?.querySelectorAll<HTMLElement>("[data-row-key]") ?? [];
    for (const element of rows) {
      if (element.dataset.rowKey === rowKey && visible(element)) return element;
    }
    return null;
  };
  useLayoutEffect(() => {
    if (selected && !view) {
      const active = document.activeElement;
      const hadFocus = overview.current?.contains(active) || active === document.body && lastFocused.current && !lastFocused.current.isConnected;
      focusIntent.current = hadFocus ? "conversation" : null;
      setSelected(null);
      return;
    }
    if (focusIntent.current === "back") {
      closeDetail.current?.focus({ preventScroll: true });
      findRow(selectedKey)?.scrollIntoView?.({ block: "nearest", inline: "nearest" });
    }
    if (focusIntent.current === "conversation") conversation.current?.focus({ preventScroll: true });
    if (focusIntent.current === "origin") {
      const target = origin.current?.isConnected && visible(origin.current) ? origin.current : findRow(returnRow.current);
      (target ?? conversation.current)?.focus({ preventScroll: true });
    }
    focusIntent.current = null;
  }, [selected, view, tasks]);
  const open = (task: TaskView, rowKey: string) => {
    const active = document.activeElement;
    origin.current = active instanceof HTMLElement && overview.current?.contains(active) && active.dataset.rowKey === rowKey ? active : findRow(rowKey) ?? null;
    focusIntent.current = "back";
    setSelected({ taskId: task.task.task_id, rowKey });
  };
  const goBack = () => {
    returnRow.current = view ? `${view.task.task_id}:${tasks.indexOf(view)}` : selected?.rowKey ?? null;
    focusIntent.current = "origin";
    setSelected(null);
  };
  const summary = taskLanes.filter(({ lane }) => lane !== "accepted").flatMap(({ lane }) => {
    const count = tasks.filter(view => view.lane === lane).length;
    return count ? [`${count} ${lane}`] : [];
  }).join(" · ");
  return <section ref={overview} className="liaison-task-overview" aria-label="Task overview" onFocusCapture={event => { lastFocused.current = event.target as HTMLElement; }}
    onKeyDown={event => { if (event.key === "Escape") { event.preventDefault(); event.stopPropagation(); if (selected) goBack(); else onClose(); } }}>
    <header><button type="button" className="liaison-conversation" ref={conversation} onClick={onClose}>‹ Conversation</button><h1>Overview</h1><div role="group" aria-label="Overview view"><button type="button" aria-pressed={mode === "tasks"} onClick={() => setMode("tasks")}>Board</button><button type="button" aria-pressed={mode === "graph"} onClick={() => setMode("graph")}>Graph</button></div><span className={waiting ? "liaison-waiting" : "liaison-task-status"}>{waiting ? "● waiting for you" : status ?? summary}</span></header>
    <div className="liaison-overview-body">
      <div className="liaison-overview-content">
        {attention.length || snapshot && (!fresh || snapshot.runtime.status !== "fresh") ? <section className="liaison-attention" aria-label="Task attention">
          <div className="liaison-attention-heading"><h2>Task attention</h2><span>All projects</span>{snapshot && (!fresh || snapshot.runtime.status !== "fresh") ? <span>Runtime observations may be stale or unavailable.</span> : null}</div>
          {attention.length ? <ul>{attention.map(({ view: item, rowKey, labels }) => <li key={rowKey}>
            <button type="button" data-row-key={rowKey} aria-current={selectedKey === rowKey ? "true" : undefined} onClick={() => open(item, rowKey)}>
              <span className="liaison-attention-title">{item.task.title}</span><span className="liaison-attention-reason">{labels.join(" · ")}</span><span className="liaison-attention-action">History ›</span>
            </button>
          </li>)}</ul> : null}
        </section> : null}
        {mode === "tasks" ? <>
          {groups.length > 1 ? <div className="liaison-project-filter" role="group" aria-label="Project">
            <button type="button" aria-pressed={activeProject === null} onClick={() => setProject(null)}>All {groups.some(group => group.open) ? <span>{groups.reduce((sum, group) => sum + group.open, 0)}</span> : null}</button>
            {groups.map(group => <button type="button" key={group.key} aria-pressed={activeProject === group.key} onClick={() => setProject(group.key)}>{group.label} {group.open ? <span>{group.open}</span> : null}</button>)}
          </div> : null}
          <SupervisorTasks tasks={tasks} runs={runs} selectedKey={selectedKey} onOpen={open} project={activeProject} />
        </> : <SupervisorGraph tasks={tasks} selectedKey={selectedKey} onOpen={open} />}
      </div>
      {selected && view ? <aside className="liaison-overview-detail" aria-label="Task history">
        <header><button type="button" className="liaison-detail-back" ref={closeDetail} aria-label="Close task history" onClick={goBack}>✕ Close</button></header>
        <TaskHistoryDetail view={view} history={history} />
      </aside> : null}
    </div>
  </section>;
}
