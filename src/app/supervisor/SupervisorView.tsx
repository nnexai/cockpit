import { useLayoutEffect, useMemo, useRef, useState } from "react";
import type { OrchestrationSnapshot, Run, TaskView } from "../../protocol/generated/v1";
import { SupervisorTasks, taskLanes } from "./SupervisorTasks";
import { SupervisorGraph } from "./SupervisorGraph";
import { TaskHistoryDetail } from "./TaskHistoryDetail";
import { taskHistory } from "./taskHistory";
import "./supervisor.css";

/** Optional overview; the normal terminal remains mounted as the conversation. */
export function SupervisorView({ tasks, runs, waiting, status, onClose, snapshot, rootId, fresh }: {
  tasks: readonly TaskView[]; runs: readonly Run[]; waiting: boolean; status: string | null; onClose(): void;
  snapshot: OrchestrationSnapshot | null; rootId: string | null; fresh: boolean;
}) {
  const [mode, setMode] = useState<"tasks" | "graph">("tasks");
  const [selected, setSelected] = useState<{ taskId: string; rowKey: string } | null>(null);
  const overview = useRef<HTMLElement>(null), back = useRef<HTMLButtonElement>(null);
  const focusIntent = useRef<"back" | "origin" | "conversation" | null>("conversation");
  const origin = useRef<HTMLElement | null>(null), returnRow = useRef<string | null>(null);
  const lastFocused = useRef<HTMLElement | null>(null);
  const view = selected ? tasks.find((task, index) => `${task.task.task_id}:${index}` === selected.rowKey) ?? tasks.find(task => task.task.task_id === selected.taskId) : null;
  const history = useMemo(() => snapshot && rootId && view ? taskHistory(snapshot, rootId, view, tasks, fresh) : null, [snapshot, rootId, view, tasks, fresh]);
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
    if (focusIntent.current === "back" || focusIntent.current === "conversation") back.current?.focus({ preventScroll: true });
    if (focusIntent.current === "origin") {
      const target = origin.current?.isConnected && visible(origin.current) ? origin.current : findRow(returnRow.current);
      (target ?? back.current)?.focus({ preventScroll: true });
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
    <header><button type="button" className={selected ? "liaison-detail-back" : "liaison-conversation"} aria-label={selected ? "Overview" : undefined} ref={back} onClick={selected ? goBack : onClose}>{selected ? "‹ Overview" : "‹ Conversation"}</button><h1>{selected ? "Task history" : "Overview"}</h1>{!selected ? <div role="group" aria-label="Overview view"><button type="button" aria-pressed={mode === "tasks"} onClick={() => setMode("tasks")}>Board</button><button type="button" aria-pressed={mode === "graph"} onClick={() => setMode("graph")}>Graph</button></div> : null}<span className={waiting ? "liaison-waiting" : "liaison-task-status"}>{waiting ? "● waiting for you" : status ?? summary}</span></header>
    <div className="liaison-overview-content" hidden={!!selected} inert={!!selected}>
      {mode === "tasks" ? <SupervisorTasks tasks={tasks} runs={runs} onOpen={open} /> : <SupervisorGraph tasks={tasks} onOpen={open} />}
    </div>
    {selected && view ? <div className="liaison-fullscreen-detail"><TaskHistoryDetail view={view} history={history} /></div> : null}
  </section>;
}
