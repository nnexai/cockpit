import { useId, useMemo, useState } from "react";
import type { TaskView } from "../../protocol/generated/v1";
import { dependencyLayout } from "./dependencyLayout";
import { dependencySummary, uniqueTask } from "./dependencies";
import { TaskRow, taskStage } from "./SupervisorTasks";
import { projectOf } from "./taskProjects";

/** Task prerequisites only: no agent topology or provenance relationships. */
export function SupervisorGraph({ tasks, onOpen, selectedKey }: { tasks: readonly TaskView[]; onOpen(view: TaskView, rowKey: string): void; selectedKey?: string | null }) {
  const marker = useId().replaceAll(":", ""), [pointer, setHover] = useState<string | null>(null), [focused, setFocused] = useState<string | null>(null);
  const hover = focused ?? pointer;
  const open = useMemo(() => tasks.filter(view => !view.task.checked), [tasks]);
  const model = useMemo(() => dependencyLayout(open), [open]);
  const positions = new Map(model.positions.map(position => [position.view.task.task_id, position]));
  const highlighted = new Set<string>();
  if (hover) {
    highlighted.add(hover);
    // Walk each direction separately; a shared prerequisite does not highlight siblings.
    for (const upstream of [true, false]) {
      const visited = new Set([hover]), remaining = [hover];
      while (remaining.length) {
        const id = remaining.pop()!;
        for (const edge of model.edges) {
          const next = upstream && edge.to === id ? edge.from : !upstream && edge.from === id ? edge.to : null;
          if (next && !visited.has(next)) { visited.add(next); highlighted.add(next); remaining.push(next); }
        }
      }
    }
  }
  return <div className="liaison-graph-wrap"><div className="liaison-task-graph" role="group" aria-label="Task dependency graph" tabIndex={0}>
    <div className="liaison-graph-canvas" style={{ width: model.width, height: model.height }}>
      <svg width={model.width} height={model.height} aria-hidden="true"><defs>{[false, true].map(active => <marker key={String(active)} id={`${marker}${active ? "-active" : ""}`} viewBox="0 0 8 8" markerWidth="7" markerHeight="7" refX="7" refY="4" orient="auto"><path d="M0,0 L8,4 L0,8" fill={active ? "var(--accent)" : "var(--border-strong)"} /></marker>)}</defs>{model.edges.map(edge => {
        const from = positions.get(edge.from)!, to = positions.get(edge.to)!, active = highlighted.has(edge.from) && highlighted.has(edge.to);
        const x1 = from.x + 240, y1 = from.y + 24, x2 = to.x, y2 = to.y + 24;
        return <path key={`${edge.from}:${edge.to}`} className={`liaison-graph-edge${active ? " is-highlighted" : ""}`} d={y1 === y2 ? `M${x1},${y1} L${x2},${y2}` : `M${x1},${y1} C${x1 + 16},${y1} ${x2 - 16},${y2} ${x2},${y2}`} markerEnd={`url(#${marker}${active ? "-active" : ""})`} />;
      })}</svg>
      {model.positions.map(({ view, x, y, cycle }) => {
        const dependency = view.dependencies.state === "invalid" ? dependencySummary(view, tasks) : view.dependencies.unmet.map(item => uniqueTask(tasks, item.task_id)?.task.title ?? "Unreadable prerequisite").join(", ");
        const detail = `${projectOf(view).label} · ${taskStage(view)}${view.dependencies.unmet.length ? ` · after ${view.dependencies.unmet.length}` : view.scheduled && view.lane === "ready" ? " · scheduled" : ""}${cycle ? " · prerequisite cycle" : ""}`;
        const rowKey = `${view.task.task_id}:${tasks.indexOf(view)}`;
        return <button type="button" key={view.task.task_id} data-row-key={rowKey} aria-current={selectedKey === rowKey ? "true" : undefined} className={`liaison-graph-task lane-${view.lane}${view.attempt ? ` attempt-${view.attempt.state}` : ""}${hover === view.task.task_id ? " is-highlighted" : ""}`} style={{ left: x, top: y }} title={`${view.task.title}, ${detail}${dependency ? `, ${dependency}` : ""}`} aria-label={`${view.task.title}, ${detail}${dependency ? `, ${dependency}` : ""}. Open history`} onMouseEnter={() => setHover(view.task.task_id)} onMouseLeave={() => setHover(null)} onFocus={() => setFocused(view.task.task_id)} onBlur={() => setFocused(null)} onClick={() => { setHover(null); onOpen(view, rowKey); }}><span className="liaison-task-dot" aria-hidden="true" /><span className="liaison-task-title">{view.task.title}</span><span className="liaison-task-stage">{detail}</span></button>;
      })}
    </div>
    <ul className="liaison-dependency-outline" role="list" aria-label="Task dependency outline">{model.positions.map(position => {
      const rowKey = `${position.view.task.task_id}:${tasks.indexOf(position.view)}`;
      return <li key={position.view.task.task_id} data-indented={position.level > 0 || undefined} style={{ paddingLeft: Math.min(position.level, 6) * 20 }}><TaskRow view={position.view} tasks={tasks} rowKey={rowKey} selected={selectedKey === rowKey} projectLabel={projectOf(position.view).label} onOpen={() => onOpen(position.view, rowKey)} />{position.view.dependencies.unmet.length > 1 ? <p className="liaison-outline-after">also after {position.view.dependencies.unmet.slice(1).map(item => `“${uniqueTask(tasks, item.task_id)?.task.title ?? "Unreadable prerequisite"}”`).join(", ")}</p> : null}</li>;
    })}</ul>
    {open.some(view => !positions.has(view.task.task_id)) ? <p className="liaison-task-status">Some task identities are unreadable.</p> : null}
  </div>{tasks.length > open.length ? <footer className="liaison-graph-footer">{tasks.length - open.length} done not shown</footer> : null}</div>;
}
