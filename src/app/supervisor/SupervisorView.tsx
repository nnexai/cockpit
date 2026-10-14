import { useEffect, useRef, useState } from "react";
import type { Run, TaskView } from "../../protocol/generated/v1";
import { SupervisorTasks, taskLanes } from "./SupervisorTasks";
import { SupervisorGraph } from "./SupervisorGraph";
import "./supervisor.css";

/** Optional overview; the normal terminal remains mounted as the conversation. */
export function SupervisorView({ tasks, runs, waiting, status, onClose }: { tasks: readonly TaskView[]; runs: readonly Run[]; waiting: boolean; status: string | null; onClose(): void }) {
  const [mode, setMode] = useState<"tasks" | "graph">("tasks");
  const back = useRef<HTMLButtonElement>(null);
  useEffect(() => { back.current?.focus({ preventScroll: true }); }, []);
  const summary = taskLanes.filter(({ lane }) => lane !== "accepted").flatMap(({ lane }) => {
    const count = tasks.filter(view => view.lane === lane).length;
    return count ? [`${count} ${lane}`] : [];
  }).join(" · ");
  return <section className="liaison-task-overview" aria-label="Task overview" onKeyDown={event => { if (event.key === "Escape") { event.preventDefault(); onClose(); } }}>
    <header><button type="button" className="liaison-conversation" ref={back} onClick={onClose}>‹ Conversation</button><h1>Overview</h1><div role="group" aria-label="Overview view"><button type="button" aria-pressed={mode === "tasks"} onClick={() => setMode("tasks")}>Board</button><button type="button" aria-pressed={mode === "graph"} onClick={() => setMode("graph")}>Graph</button></div><span className={waiting ? "liaison-waiting" : "liaison-task-status"}>{waiting ? "● waiting for you" : status ?? summary}</span></header>
    {mode === "tasks" ? <SupervisorTasks tasks={tasks} runs={runs} /> : <SupervisorGraph tasks={tasks} />}
  </section>;
}
